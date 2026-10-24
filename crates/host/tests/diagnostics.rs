//! What the host says when a component is wrong or misbehaves, and the limits
//! it enforces. Misbehaving applications are built here from small core
//! WebAssembly modules, wrapped into components for the `application` world.

mod common;

use std::path::Path;

use portable_host::runtime::{Buttons, Input, Level, Limits, Runtime};

fn load_error(bytes: &[u8]) -> String {
    match Runtime::new().unwrap().load(bytes) {
        Ok(_) => panic!("the component should have been refused"),
        Err(error) => format!("{error:#}"),
    }
}

/// A component for the `application` world whose `info` declares the given
/// values and whose `update` runs `update_body`. It logs nothing by itself and
/// renders an empty frame.
fn fake_app(name: &str, canvas: (u32, u32), ticks_per_second: u32, update_body: &str) -> Vec<u8> {
    let core = format!(
        r#"(module
            (import "portable:platform/log@0.1.0" "write" (func $write (param i32 i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 16) "{name}")
            (data (i32.const 128) "the square escaped")
            ;; app-info is returned through memory: name (ptr, len), canvas (w, h), rate.
            (func (export "portable:platform/app@0.1.0#info") (result i32)
                (i32.store (i32.const 64) (i32.const 16))
                (i32.store (i32.const 68) (i32.const {name_len}))
                (i32.store (i32.const 72) (i32.const {width}))
                (i32.store (i32.const 76) (i32.const {height}))
                (i32.store (i32.const 80) (i32.const {ticks_per_second}))
                (i32.const 64))
            (func (export "portable:platform/app@0.1.0#update") (param i32)
                {update_body})
            ;; An empty list: (ptr, len) = (0, 0).
            (func (export "portable:platform/app@0.1.0#render") (result i32)
                (i32.store (i32.const 96) (i32.const 0))
                (i32.store (i32.const 100) (i32.const 0))
                (i32.const 96))
            (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
                unreachable)
        )"#,
        name_len = name.len(),
        width = canvas.0,
        height = canvas.1,
    );
    let mut module = wat::parse_str(core).unwrap();

    let mut resolve = wit_parser::Resolve::default();
    let wit = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wit");
    let (package, _) = resolve.push_path(wit).unwrap();
    let world = resolve
        .select_world(&[package], Some("application"))
        .unwrap();
    // Full `@0.1.0` interface names, as `wasm-tools component new` produces.
    let canonical_names = false;
    wit_component::embed_component_metadata(
        &mut module,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
        canonical_names,
    )
    .unwrap();
    wit_component::ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}

const SPIN_FOREVER: &str = "(loop $forever (br $forever))";
/// Log an error (level 3) with the message at offset 128, then trap.
const REPORT_AND_TRAP: &str =
    "(call $write (i32.const 3) (i32.const 128) (i32.const 18)) unreachable";

#[test]
fn refuses_files_that_are_not_webassembly() {
    assert!(load_error(b"#!/bin/sh\necho hi\n").contains("not a WebAssembly binary"));
}

#[test]
fn refuses_core_modules_and_says_how_to_wrap_them() {
    let error = load_error(&wat::parse_str("(module)").unwrap());
    assert!(
        error.contains("core WebAssembly module, not a component"),
        "{error}"
    );
    assert!(error.contains("wasm-tools component new"), "{error}");
}

#[test]
fn refuses_components_that_request_ungranted_capabilities() {
    let component = wat::parse_str(
        r#"(component
            (import "wasi:filesystem/preopens@0.2.0" (instance))
            (import "wasi:sockets/tcp@0.2.0" (instance))
        )"#,
    )
    .unwrap();
    let error = load_error(&component);
    assert!(
        error.contains("capabilities this host does not grant"),
        "{error}"
    );
    assert!(error.contains("wasi:filesystem/preopens@0.2.0"), "{error}");
    assert!(error.contains("wasi:sockets/tcp@0.2.0"), "{error}");
    assert!(
        error.contains("may import only: portable:platform/log@0.1.0"),
        "{error}"
    );
}

#[test]
fn refuses_components_that_do_not_implement_the_contract() {
    let error = load_error(&wat::parse_str("(component)").unwrap());
    assert!(
        error.contains("does not implement portable:platform/app@0.1.0; it exports: nothing"),
        "{error}"
    );
}

#[test]
fn a_trap_names_the_tick_and_the_applications_last_report() {
    let runtime = Runtime::new().unwrap();
    let component = runtime
        .load(&fake_app("broken", (320, 180), 60, REPORT_AND_TRAP))
        .unwrap();
    let mut app = runtime.instantiate(&component, Limits::default()).unwrap();
    let error = app
        .update(Input {
            held: Buttons::empty(),
        })
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("broken failed during update of tick 0"),
        "{message}"
    );
    assert!(
        message.contains("the application reported: the square escaped"),
        "{message}"
    );
    assert!(message.contains("unreachable"), "{message}");

    let log = app.take_log();
    assert_eq!(log.len(), 1);
    assert_eq!((log[0].tick, log[0].level), (0, Level::Error));
}

#[test]
fn a_runaway_tick_is_stopped_by_its_fuel_budget() {
    let runtime = Runtime::new().unwrap();
    let component = runtime
        .load(&fake_app("spinner", (320, 180), 60, SPIN_FOREVER))
        .unwrap();
    let limits = Limits {
        fuel_per_call: 1_000_000,
        ..Limits::default()
    };
    let mut app = runtime.instantiate(&component, limits).unwrap();
    let message = format!("{:#}", app.update(Input { held: Buttons::UP }).unwrap_err());
    assert!(
        message.contains("spinner failed during update of tick 0: it exceeded its budget of 1000000 fuel per call"),
        "{message}"
    );
}

#[test]
fn declared_info_is_validated() {
    let runtime = Runtime::new().unwrap();
    for (canvas, rate, expected) in [
        ((320, 180), 0, "0 ticks per second is outside 1 to 1000"),
        ((0, 180), 60, "canvas 0x180 is outside"),
        ((5000, 180), 60, "canvas 5000x180 is outside"),
    ] {
        let component = runtime.load(&fake_app("odd", canvas, rate, "")).unwrap();
        let error = match runtime.instantiate(&component, Limits::default()) {
            Ok(_) => panic!("accepted {canvas:?} at {rate} ticks/s"),
            Err(error) => format!("{error:#}"),
        };
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn the_real_guest_is_bounded_by_fuel_and_memory_limits() {
    let guest = common::guest();
    let starved = Limits {
        fuel_per_call: 10,
        ..Limits::default()
    };
    let error = match guest.runtime.instantiate(&guest.component, starved) {
        Ok(_) => panic!("10 fuel should not be enough to describe the app"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        error.contains("exceeded its budget of 10 fuel per call"),
        "{error}"
    );

    let cramped = Limits {
        max_memory_bytes: 64 * 1024,
        ..Limits::default()
    };
    let error = match guest.runtime.instantiate(&guest.component, cramped) {
        Ok(_) => panic!("64 KiB should not be enough memory"),
        Err(error) => format!("{error:#}"),
    };
    assert!(error.contains("failed during instantiation"), "{error}");
    assert!(error.contains("memory"), "{error}");
}
