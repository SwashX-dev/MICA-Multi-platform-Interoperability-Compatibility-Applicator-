//! Moving square: the smallest interactive portable application.
//!
//! The simulation lives in [`game`] and uses nothing but the contract's data
//! types. This file connects it to the `portable:platform` contract: it exports
//! the `app` interface and imports only the host's `log` capability.

mod game;

wit_bindgen::generate!({
    path: "../../wit",
    world: "application",
    additional_derives: [PartialEq, Eq],
});

use std::cell::RefCell;

use exports::portable::platform::app::Guest;
use game::Game;
use portable::platform::types::{AppInfo, DrawCommand, Input, Size};

thread_local! {
    /// The application's whole state. It lives in the component instance's
    /// memory, so it persists across calls until the host drops the instance.
    static GAME: RefCell<Game> = RefCell::new(Game::new());
}

struct MovingSquare;

impl Guest for MovingSquare {
    fn info() -> AppInfo {
        report_panics_to_host();
        AppInfo {
            name: "moving-square".to_string(),
            canvas: Size {
                width: game::CANVAS_WIDTH,
                height: game::CANVAS_HEIGHT,
            },
            ticks_per_second: game::TICKS_PER_SECOND,
        }
    }

    fn update(input: Input) {
        report_panics_to_host();
        GAME.with_borrow_mut(|game| game.update(input.held));
    }

    fn render() -> Vec<DrawCommand> {
        report_panics_to_host();
        GAME.with_borrow(Game::render)
    }
}

#[cfg(target_arch = "wasm32")]
export!(MovingSquare);

/// Without this, a panic is an anonymous `unreachable` trap. With it, the host
/// can show the panic message next to the trap.
fn report_panics_to_host() {
    #[cfg(target_arch = "wasm32")]
    {
        static INSTALL: std::sync::Once = std::sync::Once::new();
        INSTALL.call_once(|| {
            std::panic::set_hook(Box::new(|info| {
                use portable::platform::log::{Level, write};
                write(Level::Error, &format!("panic: {info}"));
            }));
        });
    }
}
