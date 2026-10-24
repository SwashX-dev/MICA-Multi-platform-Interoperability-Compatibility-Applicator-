//! End-to-end tests against the real moving-square component: golden traces,
//! determinism, isolation, and the contract surface.

mod common;

use std::path::PathBuf;

use portable_host::runtime::{APP_EXPORT, Buttons, GRANTED_IMPORTS, Input, Runtime};
use portable_host::script::Script;
use portable_host::trace::{Trace, first_difference, format_frame};

use common::{instantiate, replay, replay_dir, script};

/// Every `tests/replay/*.script` must reproduce its `.trace` golden file
/// exactly. Run with `BLESS=1` (or `just bless`) to rewrite the golden files
/// after an intentional change.
#[test]
fn scripts_reproduce_their_golden_traces() {
    let bless = std::env::var_os("BLESS").is_some();
    let mut scripts: Vec<PathBuf> = std::fs::read_dir(replay_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "script"))
        .collect();
    scripts.sort();
    assert!(!scripts.is_empty(), "no replay scripts found");

    let mut failures = Vec::new();
    for path in &scripts {
        let script = Script::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        let trace = replay(&script);
        let golden = path.with_extension("trace");
        if bless {
            std::fs::write(&golden, trace.text()).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
            panic!(
                "missing golden trace {}; run `just bless`",
                golden.display()
            )
        });
        if let Some((line, want, got)) = first_difference(&expected, trace.text()) {
            failures.push(format!(
                "{} line {line}\n  expected: {want}\n  actual:   {got}",
                golden.display()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "traces differ from their golden files (run `just bless` if intended):\n{}",
        failures.join("\n")
    );
}

#[test]
fn identical_inputs_give_identical_traces() {
    let tour = script("tour.script");
    let first = replay(&tour);
    // A second instance from the same compiled component...
    let second = replay(&tour);
    // ...and one from an independently created engine and compilation.
    let runtime = Runtime::new().unwrap();
    let path = std::env::var_os("PORTABLE_GUEST")
        .map(PathBuf::from)
        .unwrap();
    let component = runtime.load_file(&path).unwrap();
    let mut app = runtime.instantiate(&component, Default::default()).unwrap();
    let third = portable_host::headless::replay(&mut app, &tour).unwrap();

    assert_eq!(first.text(), second.text());
    assert_eq!(first.text(), third.text());
    assert_eq!(first.sha256(), third.sha256());
    assert_eq!(first.frames(), 300);
}

#[test]
fn rendering_does_not_change_the_simulation() {
    let tour = script("tour.script");
    let mut app = instantiate();
    let mut trace = Trace::new();
    for held in tour.inputs() {
        let tick = app.ticks();
        for _ in 0..3 {
            app.render().unwrap();
        }
        app.update(Input { held }).unwrap();
        let frame = app.render().unwrap();
        assert_eq!(format_frame(&frame), format_frame(&app.render().unwrap()));
        trace.record(tick, held, &frame);
    }
    assert_eq!(trace.text(), replay(&tour).text());
}

#[test]
fn state_persists_across_ticks_and_instances_are_isolated() {
    let mut left = instantiate();
    let mut right = instantiate();
    for _ in 0..20 {
        left.update(Input {
            held: Buttons::LEFT,
        })
        .unwrap();
        right
            .update(Input {
                held: Buttons::RIGHT,
            })
            .unwrap();
    }
    // 20 ticks at 2 pixels per tick from x = 152.
    let square = |x: i32| format!("clear #14161c; fill {x},82 16x16 #f2c14e");
    assert_eq!(format_frame(&left.render().unwrap()), square(112));
    assert_eq!(format_frame(&right.render().unwrap()), square(192));
    assert_eq!((left.ticks(), right.ticks()), (20, 20));
}

#[test]
fn application_declares_its_canvas_and_tick_rate() {
    let app = instantiate();
    let info = app.info();
    assert_eq!(info.name, "moving-square");
    assert_eq!((info.canvas.width, info.canvas.height), (320, 180));
    assert_eq!(info.ticks_per_second, 60);
}

#[test]
fn component_requests_only_granted_capabilities() {
    let summary = common::guest().component.summary();
    assert!(
        summary
            .imports
            .iter()
            .any(|name| name == "portable:platform/log@0.1.0")
    );
    for import in &summary.imports {
        assert!(
            GRANTED_IMPORTS.contains(&import.as_str()),
            "unexpected import {import}"
        );
    }
    assert_eq!(summary.exports, [APP_EXPORT]);
}

#[test]
fn golden_traces_start_from_the_centre() {
    let text = replay(&script("idle.script")).text().to_string();
    let lines: Vec<&str> = text.lines().skip(1).collect();
    assert_eq!(lines.len(), 30);
    assert!(
        lines
            .iter()
            .all(|line| line.ends_with("| clear #14161c; fill 152,82 16x16 #f2c14e"))
    );
}
