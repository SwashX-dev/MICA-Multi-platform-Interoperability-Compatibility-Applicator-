//! Graphical backend: an SDL3 window, keyboard input, and a renderer for the
//! application's drawing commands.
//!
//! The application draws on its logical canvas. This backend scales the canvas
//! to fit the window (by a whole number when it can, letterboxed) and draws a
//! status bar under it that names the current mode and every key active in it.
//!
//! Modes: NORMAL runs the simulation in real time; `P` enters PAUSED, where `N`
//! advances one tick and `P` or `Esc` returns to NORMAL. A scripted session
//! enters ENDED when its script runs out. `Q` quits from any mode.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use sdl3::event::Event;
use sdl3::keyboard::{KeyboardState, Keycode, Scancode};
use sdl3::pixels::Color as SdlColor;
use sdl3::render::{FRect, WindowCanvas};

use crate::pacer::Pacer;
use crate::runtime::{App, Buttons, Color, DrawCommand, Input, Rect};
use crate::script::{Recorder, Script};

/// The built-in debug font is 8x8 pixels per character.
const GLYPH: f32 = 8.0;
/// Padding around status bar text, in text pixels.
const PAD: f32 = 3.0;

const LETTERBOX: SdlColor = SdlColor::RGB(0x08, 0x09, 0x0b);
const BAR: SdlColor = SdlColor::RGB(0x22, 0x25, 0x2d);
const TEXT: SdlColor = SdlColor::RGB(0xc8, 0xcc, 0xd4);
const CHIP_TEXT: SdlColor = SdlColor::RGB(0x10, 0x11, 0x14);

pub struct Options {
    /// Initial window size, as a multiple of the canvas size.
    pub scale: u32,
    /// Drive the application from this script (with a display name) instead
    /// of the keyboard.
    pub script: Option<(String, Script)>,
    /// With a script: quit when it ends instead of waiting on the last frame.
    pub exit_at_end: bool,
    /// Save the last frame, status bar included, as a PNG file on exit.
    pub screenshot: Option<PathBuf>,
    /// Show a recording indicator in the status bar.
    pub recording: bool,
}

/// What happened during a session.
pub struct Outcome {
    /// The input of every tick simulated, from the keyboard or the script.
    pub inputs: Script,
    /// The frame on screen at the end.
    pub final_frame: Vec<DrawCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Paused,
    Ended,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Paused => "PAUSED",
            Mode::Ended => "ENDED",
        }
    }

    fn color(self) -> SdlColor {
        match self {
            Mode::Normal => SdlColor::RGB(0x7f, 0xc8, 0x8a),
            Mode::Paused => SdlColor::RGB(0xf2, 0xc1, 0x4e),
            Mode::Ended => SdlColor::RGB(0x9a, 0xa3, 0xb5),
        }
    }

    /// Every key that does something in this mode.
    fn hints(self, scripted: bool) -> &'static str {
        match (self, scripted) {
            (Mode::Normal, false) => "Arrows/WASD move  P pause  Q quit",
            (Mode::Normal, true) => "P pause  Q quit",
            (Mode::Paused, false) => "N step (with Arrows/WASD)  P/Esc resume  Q quit",
            (Mode::Paused, true) => "N step  P/Esc resume  Q quit",
            (Mode::Ended, _) => "Q quit",
        }
    }
}

struct Status<'a> {
    mode: Mode,
    script: Option<&'a str>,
    tick: u64,
    recording: bool,
}

impl Status<'_> {
    fn detail(&self) -> String {
        let mut detail = format!("tick {}", self.tick);
        if let Some(name) = self.script {
            detail = format!("{name}  {detail}");
        }
        if self.recording {
            detail.push_str("  REC");
        }
        detail
    }
}

pub fn play(app: &mut App, options: &Options) -> Result<Outcome> {
    let info = app.info().clone();
    let canvas_size = (info.canvas.width, info.canvas.height);
    let scripted: Option<(&str, Vec<Buttons>)> = options
        .script
        .as_ref()
        .map(|(name, script)| (name.as_str(), script.inputs().collect()));

    let sdl = sdl3::init().map_err(|e| anyhow!("cannot initialize SDL: {e}"))?;
    let video = sdl
        .video()
        .map_err(|e| anyhow!("cannot initialize SDL video: {e}"))?;
    let (width, height) = initial_window_size(canvas_size, options.scale);
    let window = video
        .window(&format!("{} - portable-host", info.name), width, height)
        .position_centered()
        .resizable()
        .high_pixel_density()
        .build()
        .map_err(|e| anyhow!("cannot open a window: {e}"))?;
    let mut canvas = window.into_canvas();
    // SAFETY: the renderer pointer comes from a live canvas.
    let vsync = unsafe { sdl3::sys::render::SDL_SetRenderVSync(canvas.raw(), 1) };
    println!(
        "display    {}, renderer {}, vsync {}",
        video_driver(),
        renderer_name(&canvas),
        if vsync { "on" } else { "off" }
    );
    let mut events = sdl
        .event_pump()
        .map_err(|e| anyhow!("cannot read SDL events: {e}"))?;

    let mut pacer = Pacer::new(info.ticks_per_second);
    let mut recorder = Recorder::new();
    let mut mode = Mode::Normal;
    let mut frame = app.render()?;
    let mut last = Instant::now();

    loop {
        let mut steps = 0;
        let mut quit = false;
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. } => quit = true,
                Event::KeyDown {
                    keycode: Some(key),
                    repeat: false,
                    ..
                } => match (mode, key) {
                    (_, Keycode::Q) => quit = true,
                    (Mode::Normal, Keycode::P) => mode = Mode::Paused,
                    (Mode::Paused, Keycode::P | Keycode::Escape) => mode = Mode::Normal,
                    (Mode::Paused, Keycode::N) => steps += 1,
                    _ => {}
                },
                _ => {}
            }
        }
        if quit {
            break;
        }

        let now = Instant::now();
        let elapsed = now - last;
        last = now;
        let due = match mode {
            Mode::Normal => pacer.advance(elapsed),
            Mode::Paused => {
                // Time spent paused is never caught up afterwards.
                pacer.reset();
                steps
            }
            Mode::Ended => 0,
        };

        let keyboard = held_buttons(&events.keyboard_state());
        for _ in 0..due {
            let held = match &scripted {
                Some((_, inputs)) => match usize::try_from(app.ticks())
                    .ok()
                    .and_then(|tick| inputs.get(tick))
                {
                    Some(&held) => held,
                    None => break,
                },
                None => keyboard,
            };
            recorder.record(held);
            app.update(Input { held })?;
        }
        if due > 0 {
            frame = app.render()?;
        }
        for entry in app.take_log() {
            eprintln!("[{}] {entry}", info.name);
        }
        if let Some((_, inputs)) = &scripted
            && recorder.ticks() >= inputs.len() as u64
            && mode != Mode::Ended
        {
            if options.exit_at_end {
                break;
            }
            mode = Mode::Ended;
        }

        let status = Status {
            mode,
            script: scripted.as_ref().map(|(name, _)| *name),
            tick: app.ticks(),
            recording: options.recording,
        };
        draw(&mut canvas, canvas_size, &frame, &status)?;
        canvas.present();
        if !vsync {
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    if let Some(path) = &options.screenshot {
        let status = Status {
            mode,
            script: scripted.as_ref().map(|(name, _)| *name),
            tick: app.ticks(),
            recording: options.recording,
        };
        draw(&mut canvas, canvas_size, &frame, &status)?;
        save_png(&canvas, path)?;
    }

    Ok(Outcome {
        inputs: recorder.finish(),
        final_frame: frame,
    })
}

/// Movement uses physical key positions, so WASD works on any keyboard layout.
fn held_buttons(keys: &KeyboardState) -> Buttons {
    let bindings = [
        ([Scancode::Up, Scancode::W], Buttons::UP),
        ([Scancode::Down, Scancode::S], Buttons::DOWN),
        ([Scancode::Left, Scancode::A], Buttons::LEFT),
        ([Scancode::Right, Scancode::D], Buttons::RIGHT),
    ];
    let mut held = Buttons::empty();
    for (codes, button) in bindings {
        if codes.iter().any(|&code| keys.is_scancode_pressed(code)) {
            held |= button;
        }
    }
    held
}

/// Where the canvas and the status bar go in a window, in window pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Layout {
    canvas_x: f32,
    canvas_y: f32,
    scale: f32,
    bar_y: f32,
    bar_height: f32,
    text_scale: f32,
}

fn text_scale(window_w: f32, window_h: f32) -> f32 {
    // Text about 3% of the window height, or less if the window is narrow (a
    // tiling window manager may make it tall and thin), in whole multiples of
    // the 8px font.
    ((window_h * 0.03).min(window_w * 0.02) / GLYPH)
        .round()
        .clamp(1.0, 4.0)
}

fn layout(window: (u32, u32), canvas: (u32, u32)) -> Layout {
    let (window_w, window_h) = (window.0 as f32, window.1 as f32);
    let (canvas_w, canvas_h) = (canvas.0 as f32, canvas.1 as f32);
    let text_scale = text_scale(window_w, window_h);
    let bar_height = (GLYPH + 2.0 * PAD) * text_scale;
    let area_h = (window_h - bar_height).max(1.0);
    let fit = (window_w / canvas_w).min(area_h / canvas_h);
    // A whole-number scale keeps every logical pixel the same size on screen.
    let scale = if fit >= 1.0 { fit.floor() } else { fit };
    Layout {
        canvas_x: ((window_w - canvas_w * scale) / 2.0).floor(),
        canvas_y: ((area_h - canvas_h * scale) / 2.0).floor(),
        scale,
        bar_y: window_h - bar_height,
        bar_height,
        text_scale,
    }
}

fn initial_window_size(canvas: (u32, u32), scale: u32) -> (u32, u32) {
    let (width, height) = (canvas.0 * scale.max(1), canvas.1 * scale.max(1));
    let bar = (GLYPH + 2.0 * PAD) * text_scale(width as f32, height as f32);
    (width, height + bar as u32)
}

/// The part of `rect` that lies on a `width` by `height` canvas, if any.
fn clip(rect: &Rect, width: u32, height: u32) -> Option<(i64, i64, i64, i64)> {
    let left = i64::from(rect.x).max(0);
    let top = i64::from(rect.y).max(0);
    let right = (i64::from(rect.x) + i64::from(rect.width)).min(i64::from(width));
    let bottom = (i64::from(rect.y) + i64::from(rect.height)).min(i64::from(height));
    (left < right && top < bottom).then_some((left, top, right - left, bottom - top))
}

fn sdl_color(color: &Color) -> SdlColor {
    SdlColor::RGB(color.r, color.g, color.b)
}

fn draw(
    canvas: &mut WindowCanvas,
    canvas_size: (u32, u32),
    frame: &[DrawCommand],
    status: &Status,
) -> Result<()> {
    let (width, height) = canvas
        .output_size()
        .map_err(|e| anyhow!("cannot read the window size: {e}"))?;
    let layout = layout((width, height), canvas_size);
    canvas.set_draw_color(LETTERBOX);
    canvas.clear();

    let (canvas_w, canvas_h) = canvas_size;
    for command in frame {
        let (color, area) = match command {
            DrawCommand::Clear(color) => (color, Some((0, 0, canvas_w.into(), canvas_h.into()))),
            DrawCommand::FillRect(fill) => (&fill.color, clip(&fill.rect, canvas_w, canvas_h)),
        };
        let Some((x, y, w, h)) = area else { continue };
        canvas.set_draw_color(sdl_color(color));
        fill(
            canvas,
            FRect::new(
                layout.canvas_x + x as f32 * layout.scale,
                layout.canvas_y + y as f32 * layout.scale,
                w as f32 * layout.scale,
                h as f32 * layout.scale,
            ),
        )?;
    }
    draw_status_bar(canvas, &layout, width as f32, status)
}

fn draw_status_bar(
    canvas: &mut WindowCanvas,
    layout: &Layout,
    width: f32,
    status: &Status,
) -> Result<()> {
    canvas.set_draw_color(BAR);
    fill(
        canvas,
        FRect::new(0.0, layout.bar_y, width, layout.bar_height),
    )?;

    // Text is laid out in units of `text_scale` window pixels.
    let scale = layout.text_scale;
    canvas
        .set_scale(scale, scale)
        .map_err(|e| anyhow!("cannot scale text: {e}"))?;
    let y = layout.bar_y / scale + PAD;
    let chip = format!(" {} ", status.mode.label());
    canvas.set_draw_color(status.mode.color());
    fill(
        canvas,
        FRect::new(PAD, y - 2.0, chip.len() as f32 * GLYPH, GLYPH + 4.0),
    )?;
    canvas.set_draw_color(CHIP_TEXT);
    debug_text(canvas, PAD, y, &chip);

    let hints = status.mode.hints(status.script.is_some());
    let hints_x = PAD + (chip.len() + 1) as f32 * GLYPH;
    canvas.set_draw_color(TEXT);
    debug_text(canvas, hints_x, y, hints);

    // The tick counter goes on the right when there is room for it.
    let detail = status.detail();
    let detail_x = width / scale - PAD - detail.len() as f32 * GLYPH;
    if detail_x >= hints_x + (hints.len() + 2) as f32 * GLYPH {
        debug_text(canvas, detail_x, y, &detail);
    }
    canvas
        .set_scale(1.0, 1.0)
        .map_err(|e| anyhow!("cannot reset scale: {e}"))
}

fn fill(canvas: &mut WindowCanvas, rect: FRect) -> Result<()> {
    canvas
        .fill_rect(rect)
        .map_err(|e| anyhow!("cannot draw: {e}"))
}

/// Draw ASCII text with SDL's built-in 8x8 debug font in the current color.
fn debug_text(canvas: &mut WindowCanvas, x: f32, y: f32, text: &str) {
    let Ok(text) = CString::new(text) else { return };
    // SAFETY: the renderer comes from a live canvas and `text` is a valid,
    // NUL-terminated string that outlives the call.
    unsafe {
        sdl3::sys::render::SDL_RenderDebugText(canvas.raw(), x, y, text.as_ptr());
    }
}

fn video_driver() -> String {
    // SAFETY: SDL returns a static string or null.
    let name = unsafe { sdl3::sys::video::SDL_GetCurrentVideoDriver() };
    c_string(name)
}

fn renderer_name(canvas: &WindowCanvas) -> String {
    // SAFETY: the renderer comes from a live canvas; SDL returns a string it
    // owns, or null.
    let name = unsafe { sdl3::sys::render::SDL_GetRendererName(canvas.raw()) };
    c_string(name)
}

fn c_string(pointer: *const std::ffi::c_char) -> String {
    if pointer.is_null() {
        return "unknown".to_string();
    }
    // SAFETY: checked for null; SDL's strings are NUL-terminated.
    unsafe { std::ffi::CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

/// Save what has been drawn since the last present as a PNG.
fn save_png(canvas: &WindowCanvas, path: &Path) -> Result<()> {
    use sdl3::sys::render::SDL_RenderReadPixels;
    use sdl3::sys::surface::{SDL_DestroySurface, SDL_SavePNG};

    let c_path = CString::new(path.as_os_str().as_encoded_bytes())
        .context("the screenshot path contains a NUL byte")?;
    // SAFETY: the renderer comes from a live canvas; the surface SDL returns
    // is checked for null and destroyed exactly once.
    unsafe {
        let surface = SDL_RenderReadPixels(canvas.raw(), std::ptr::null());
        if surface.is_null() {
            bail!("cannot read back the frame: {}", sdl3::get_error());
        }
        let saved = SDL_SavePNG(surface, c_path.as_ptr());
        SDL_DestroySurface(surface);
        ensure!(
            saved,
            "cannot save {}: {}",
            path.display(),
            sdl3::get_error()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_window_fits_canvas_at_requested_scale() {
        let (width, height) = initial_window_size((320, 180), 3);
        let layout = layout((width, height), (320, 180));
        assert_eq!(layout.scale, 3.0);
        assert_eq!((layout.canvas_x, layout.canvas_y), (0.0, 0.0));
        assert_eq!(layout.bar_y, 540.0);
    }

    #[test]
    fn wide_window_letterboxes_with_whole_number_scale() {
        let layout = layout((1920, 1080), (320, 180));
        let area = 1080.0 - layout.bar_height;
        assert_eq!(layout.scale, (area / 180.0).floor());
        assert_eq!(
            layout.canvas_x,
            ((1920.0 - 320.0 * layout.scale) / 2.0).floor()
        );
    }

    #[test]
    fn tall_tiled_window_keeps_text_proportionate() {
        // A tiling compositor resized the default window to 912x933.
        let layout = layout((912, 933), (320, 180));
        assert_eq!(layout.text_scale, 2.0);
        assert_eq!(layout.scale, 2.0);
        assert_eq!(layout.canvas_x, 136.0);
    }

    #[test]
    fn tiny_window_scales_down() {
        let layout = layout((160, 120), (320, 180));
        assert!(layout.scale < 1.0);
        assert!(320.0 * layout.scale <= 160.0);
    }

    #[test]
    fn clips_rectangles_to_the_canvas() {
        let rect = |x, y, width, height| Rect {
            x,
            y,
            width,
            height,
        };
        assert_eq!(clip(&rect(10, 10, 5, 5), 320, 180), Some((10, 10, 5, 5)));
        assert_eq!(clip(&rect(-4, -4, 8, 8), 320, 180), Some((0, 0, 4, 4)));
        assert_eq!(
            clip(&rect(316, 178, 8, 8), 320, 180),
            Some((316, 178, 4, 2))
        );
        assert_eq!(clip(&rect(400, 10, 8, 8), 320, 180), None);
        assert_eq!(clip(&rect(i32::MAX, 0, u32::MAX, 8), 320, 180), None);
        assert_eq!(
            clip(&rect(i32::MIN, 0, u32::MAX, 8), 320, 180),
            Some((0, 0, 320, 8))
        );
    }
}
