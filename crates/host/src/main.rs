//! `portable-host`: run portable application components on this machine.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::{Args, Parser, Subcommand};

use portable_host::headless;
use portable_host::runtime::{App, AppComponent, CONTRACT, GRANTED_IMPORTS, Limits, Runtime};
use portable_host::script::Script;
use portable_host::trace::{first_difference, format_frame, frame_sha256};

#[derive(Parser)]
#[command(
    version,
    about = "Run portable application components (WebAssembly) on this machine",
    after_help = concat!("Contract: portable:platform@0.1.0. Host: ", env!("CARGO_PKG_VERSION"), ".")
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Open a window and play a component with the keyboard
    Play(PlayArgs),
    /// Run a component headlessly against scripted input
    Replay(ReplayArgs),
    /// Show a component's hash, requested imports, exports, and declared info
    Inspect {
        /// The application component (.wasm)
        component: PathBuf,
    },
}

#[derive(Args)]
struct LimitArgs {
    /// Fuel (roughly, wasm instructions) available to each call into the app
    #[arg(long, value_name = "UNITS", default_value_t = Limits::default().fuel_per_call)]
    fuel: u64,
    /// Largest linear memory the app may use, in MiB
    #[arg(long, value_name = "MIB", default_value_t = Limits::default().max_memory_bytes >> 20)]
    max_memory: usize,
}

impl LimitArgs {
    fn limits(&self) -> Limits {
        Limits {
            fuel_per_call: self.fuel,
            max_memory_bytes: self.max_memory << 20,
        }
    }
}

#[derive(Args)]
struct ReplayArgs {
    /// The application component (.wasm)
    component: PathBuf,
    /// Input script: `ticks N`, then lines of `<tick> <buttons...>`
    script: PathBuf,
    /// Write the full trace (one line per tick) to this file
    #[arg(long, value_name = "FILE")]
    trace: Option<PathBuf>,
    /// Compare the trace with this golden file; fail on any difference
    #[arg(long, value_name = "FILE")]
    expect: Option<PathBuf>,
    #[command(flatten)]
    limits: LimitArgs,
}

#[derive(Args)]
struct PlayArgs {
    /// The application component (.wasm)
    component: PathBuf,
    /// Save the input you play as a script, for `replay`
    #[arg(long, value_name = "FILE")]
    record: Option<PathBuf>,
    /// Drive the app from a script instead of the keyboard
    #[arg(long, value_name = "FILE")]
    script: Option<PathBuf>,
    /// With --script: quit when the script ends
    #[arg(long, requires = "script")]
    exit_at_end: bool,
    /// Save the last frame (with the status bar) as a PNG on exit
    #[arg(long, value_name = "FILE")]
    screenshot: Option<PathBuf>,
    /// Initial window size as a multiple of the app's canvas
    #[arg(long, default_value_t = 3)]
    scale: u32,
    #[command(flatten)]
    limits: LimitArgs,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Play(args) => play(args),
        Command::Replay(args) => replay(args),
        Command::Inspect { component } => inspect(&component),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:?}");
            ExitCode::FAILURE
        }
    }
}

fn host_line() -> String {
    format!(
        "{}-{} (portable-host {}, contract {CONTRACT})",
        std::env::consts::ARCH,
        std::env::consts::OS,
        env!("CARGO_PKG_VERSION"),
    )
}

fn print_component(path: &Path, component: &AppComponent) {
    let summary = component.summary();
    println!("component  {}", path.display());
    println!("  sha256   {}", summary.sha256);
}

fn print_app(app: &App) {
    let info = app.info();
    println!(
        "app        {}, canvas {}x{}, {} ticks/s",
        info.name, info.canvas.width, info.canvas.height, info.ticks_per_second
    );
}

fn print_log(app: &mut App) {
    let name = app.info().name.clone();
    for entry in app.take_log() {
        eprintln!("[{name}] {entry}");
    }
}

fn read_script(path: &Path) -> Result<Script> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read script {}", path.display()))?;
    Script::parse(&text).with_context(|| format!("invalid script {}", path.display()))
}

fn replay(args: ReplayArgs) -> Result<()> {
    let runtime = Runtime::new()?;
    let component = runtime.load_file(&args.component)?;
    let script = read_script(&args.script)?;
    let mut app = runtime.instantiate(&component, args.limits.limits())?;

    println!("host       {}", host_line());
    print_component(&args.component, &component);
    print_app(&app);
    println!(
        "script     {}, {} ticks",
        args.script.display(),
        script.ticks()
    );

    let result = headless::replay(&mut app, &script);
    print_log(&mut app);
    let trace = result?;
    let final_frame = app.render()?;
    println!(
        "trace      sha256 {} ({} frames)",
        trace.sha256(),
        trace.frames()
    );
    println!(
        "final      sha256 {}: {}",
        frame_sha256(&final_frame),
        format_frame(&final_frame)
    );

    if let Some(path) = &args.trace {
        std::fs::write(path, trace.text())
            .with_context(|| format!("cannot write trace {}", path.display()))?;
        println!("wrote      {}", path.display());
    }
    if let Some(path) = &args.expect {
        let expected = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read golden trace {}", path.display()))?;
        if let Some((line, want, got)) = first_difference(&expected, trace.text()) {
            bail!(
                "the trace differs from {} at line {line}\n  expected: {want}\n  actual:   {got}",
                path.display()
            );
        }
        println!("golden     {}: identical", path.display());
    }
    Ok(())
}

fn inspect(path: &Path) -> Result<()> {
    let runtime = Runtime::new()?;
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let (_, summary) = runtime
        .describe(&bytes)
        .with_context(|| format!("cannot load {}", path.display()))?;
    println!("component  {}", path.display());
    println!("  sha256   {}", summary.sha256);
    println!("  size     {} bytes", summary.size);
    for (index, name) in summary.imports.iter().enumerate() {
        let label = if index == 0 { "imports" } else { "" };
        let verdict = if GRANTED_IMPORTS.contains(&name.as_str()) {
            "granted"
        } else {
            "REFUSED"
        };
        println!("{label:<10} {name:<40} {verdict}");
    }
    if summary.imports.is_empty() {
        println!("imports    (none)");
    }
    for (index, name) in summary.exports.iter().enumerate() {
        let label = if index == 0 { "exports" } else { "" };
        println!("{label:<10} {name}");
    }
    let component = runtime
        .load(&bytes)
        .with_context(|| format!("{} cannot run on this host", path.display()))?;
    let app = runtime.instantiate(&component, Limits::default())?;
    print_app(&app);
    Ok(())
}

#[cfg(not(feature = "sdl"))]
fn play(_: PlayArgs) -> Result<()> {
    bail!(
        "this portable-host was built without its graphical backend (cargo feature `sdl`); \
         use `replay` instead"
    )
}

#[cfg(feature = "sdl")]
fn play(args: PlayArgs) -> Result<()> {
    use portable_host::sdl;

    let runtime = Runtime::new()?;
    let component = runtime.load_file(&args.component)?;
    let limits = args.limits.limits();
    let script = match &args.script {
        Some(path) => {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            Some((name, read_script(path)?))
        }
        None => None,
    };
    let mut app = runtime.instantiate(&component, limits)?;

    println!("host       {}", host_line());
    print_component(&args.component, &component);
    print_app(&app);
    if script.is_none() {
        println!("keys       arrows/WASD move, P pause, Q quit (shown in the window's status bar)");
    }

    let options = sdl::Options {
        scale: args.scale,
        script,
        exit_at_end: args.exit_at_end,
        screenshot: args.screenshot.clone(),
        recording: args.record.is_some(),
    };
    let outcome = sdl::play(&mut app, &options)?;
    let ticks = outcome.inputs.ticks();
    let shown = frame_sha256(&outcome.final_frame);
    println!("played     {ticks} ticks; final frame sha256 {shown}");

    // Every session is checked against the headless backend: a fresh instance
    // fed the same inputs must end on the same frame.
    let mut fresh = runtime.instantiate(&component, limits)?;
    headless::replay(&mut fresh, &outcome.inputs)?;
    let replayed = frame_sha256(&fresh.render()?);
    if replayed != shown {
        bail!("the headless replay of this session ended on a different frame ({replayed})");
    }
    println!("headless   replaying the same {ticks} ticks ends on the same frame");

    if let Some(path) = &args.record {
        std::fs::write(path, outcome.inputs.to_text())
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!(
            "recorded   {}; replay it with: portable-host replay {} {}",
            path.display(),
            args.component.display(),
            path.display()
        );
    }
    if let Some(path) = &args.screenshot {
        println!("screenshot {}", path.display());
    }
    Ok(())
}
