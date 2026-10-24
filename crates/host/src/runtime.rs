//! Loading, sandboxing, and calling application components.
//!
//! This module is the application's authority boundary. A component reaches the
//! outside world only through the imports linked here, and [`Limits`] bound the
//! memory and computation of every call. The engine is configured so that the
//! same component given the same inputs produces the same outputs on every host
//! and CPU architecture.

use std::collections::VecDeque;
use std::fmt;
use std::path::Path;

use anyhow::{Context as _, Result, bail, ensure};
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder, Trap};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "application",
    });
}

pub use bindings::portable::platform::log::Level;
pub use bindings::portable::platform::types::{
    AppInfo, Buttons, Color, DrawCommand, FilledRect, Input, Rect, Size,
};

/// The contract version this host implements.
pub const CONTRACT: &str = "portable:platform@0.1.0";

/// The interface every application exports.
pub const APP_EXPORT: &str = "portable:platform/app@0.1.0";

/// The complete authority an application may request. A component that imports
/// anything else is refused before any of its code runs.
pub const GRANTED_IMPORTS: &[&str] = &[
    "portable:platform/log@0.1.0",
    // Type definitions only; it has no functions.
    "portable:platform/types@0.1.0",
];

/// Longest log message kept; longer messages are truncated.
const MAX_LOG_MESSAGE: usize = 4096;
/// Most log entries buffered between drains; the oldest are dropped first.
const MAX_LOG_ENTRIES: usize = 1024;

/// Resource bounds for one application instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Fuel (roughly, WebAssembly instructions) available to each call into the
    /// application. It is counted in instructions rather than time, so a budget
    /// overrun happens at the same point on every machine.
    pub fuel_per_call: u64,
    /// Largest linear memory the application may grow to, in bytes.
    pub max_memory_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel_per_call: 10_000_000,
            max_memory_bytes: 64 << 20,
        }
    }
}

/// A configured engine plus the capabilities granted to every application.
pub struct Runtime {
    engine: Engine,
    linker: Linker<HostState>,
}

/// What a component asks for and offers, read without running any of it.
#[derive(Debug, Clone)]
pub struct ComponentSummary {
    pub sha256: String,
    pub size: usize,
    pub imports: Vec<String>,
    pub exports: Vec<String>,
}

/// A compiled component that passed the authority and contract checks, ready
/// to be instantiated any number of times.
pub struct AppComponent {
    pre: bindings::ApplicationPre<HostState>,
    summary: ComponentSummary,
}

impl AppComponent {
    pub fn summary(&self) -> &ComponentSummary {
        &self.summary
    }
}

/// One message the application wrote to the `log` capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Ticks the application had completed when it wrote the message.
    pub tick: u64,
    pub level: Level,
    pub message: String,
}

impl fmt::Display for LogEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.level {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        };
        write!(f, "tick {} {level}: {}", self.tick, self.message)
    }
}

/// Per-instance host state: everything the granted imports can touch.
struct HostState {
    limits: StoreLimits,
    /// Ticks completed when the current call began; stamps log entries.
    tick: u64,
    log: VecDeque<LogEntry>,
}

impl bindings::portable::platform::log::Host for HostState {
    fn write(&mut self, level: Level, mut message: String) {
        if message.len() > MAX_LOG_MESSAGE {
            let mut end = MAX_LOG_MESSAGE;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
            message.push_str(" [truncated]");
        }
        if self.log.len() == MAX_LOG_ENTRIES {
            self.log.pop_front();
        }
        self.log.push_back(LogEntry {
            tick: self.tick,
            level,
            message,
        });
    }
}

impl bindings::portable::platform::types::Host for HostState {}

impl Runtime {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        // Determinism across hosts and CPU architectures:
        // x86-64 and ARM64 produce different NaN bit patterns; canonicalize them.
        config.cranelift_nan_canonicalization(true);
        // Relaxed-SIMD results are otherwise allowed to differ between CPUs.
        config.relaxed_simd_deterministic(true);
        // Shared-memory threads would make results depend on scheduling; the
        // host is built without Wasmtime's `threads` feature, so they cannot be
        // enabled at all.
        // Bound computation in executed instructions, which are the same on
        // every machine, rather than in wall-clock time, which is not.
        config.consume_fuel(true);

        let engine = Engine::new(&config)?;
        let mut linker = Linker::new(&engine);
        bindings::Application::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;
        Ok(Self { engine, linker })
    }

    /// Compile a component and list what it imports and exports, without
    /// checking either and without running any of its code.
    pub fn describe(&self, bytes: &[u8]) -> Result<(Component, ComponentSummary)> {
        check_is_component(bytes)?;
        let component = Component::from_binary(&self.engine, bytes)
            .map_err(anyhow::Error::from)
            .context("not a valid WebAssembly component")?;
        let ty = component.component_type();
        let summary = ComponentSummary {
            sha256: crate::sha256_hex(bytes),
            size: bytes.len(),
            imports: ty
                .imports(&self.engine)
                .map(|(name, _)| name.to_string())
                .collect(),
            exports: ty
                .exports(&self.engine)
                .map(|(name, _)| name.to_string())
                .collect(),
        };
        Ok((component, summary))
    }

    pub fn load_file(&self, path: &Path) -> Result<AppComponent> {
        let bytes =
            std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        self.load(&bytes)
            .with_context(|| format!("cannot load {}", path.display()))
    }

    /// Compile a component and check it against the contract: it may import
    /// only [`GRANTED_IMPORTS`] and must export [`APP_EXPORT`].
    pub fn load(&self, bytes: &[u8]) -> Result<AppComponent> {
        let (component, summary) = self.describe(bytes)?;

        let refused: Vec<&str> = summary
            .imports
            .iter()
            .map(String::as_str)
            .filter(|name| !GRANTED_IMPORTS.contains(name))
            .collect();
        if !refused.is_empty() {
            bail!(
                "the component requests capabilities this host does not grant:\n{}\n\
                 portable applications may import only: {}",
                refused
                    .iter()
                    .map(|name| format!("  - {name}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                GRANTED_IMPORTS.join(", "),
            );
        }
        if !summary.exports.iter().any(|name| name == APP_EXPORT) {
            bail!(
                "the component does not implement {APP_EXPORT}; it exports: {}",
                if summary.exports.is_empty() {
                    "nothing".to_string()
                } else {
                    summary.exports.join(", ")
                },
            );
        }

        let pre = self
            .linker
            .instantiate_pre(&component)
            .map_err(anyhow::Error::from)
            .context("the component's imports do not match the contract")?;
        let pre = bindings::ApplicationPre::new(pre)
            .map_err(anyhow::Error::from)
            .with_context(|| format!("the component's exports do not match {CONTRACT}"))?;
        Ok(AppComponent { pre, summary })
    }

    /// Start a fresh instance of an application, isolated from every other
    /// instance, and read the facts it declares about itself.
    pub fn instantiate(&self, component: &AppComponent, limits: Limits) -> Result<App> {
        let state = HostState {
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.max_memory_bytes)
                .trap_on_grow_failure(true)
                .build(),
            tick: 0,
            log: VecDeque::new(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);

        store.set_fuel(limits.fuel_per_call)?;
        let bindings = match component.pre.instantiate(&mut store) {
            Ok(bindings) => bindings,
            Err(err) => {
                return Err(guest_error(
                    err,
                    &mut store,
                    "application",
                    "instantiation",
                    limits,
                ));
            }
        };
        store.set_fuel(limits.fuel_per_call)?;
        let info = match bindings.portable_platform_app().call_info(&mut store) {
            Ok(info) => info,
            Err(err) => return Err(guest_error(err, &mut store, "application", "info", limits)),
        };
        validate_info(&info)?;

        Ok(App {
            store,
            bindings,
            info,
            limits,
            ticks: 0,
        })
    }
}

/// A running application instance. Its state lives in its own linear memory,
/// which nothing but its own code can read or write.
///
/// After any call fails, the instance is unusable and should be dropped.
pub struct App {
    store: Store<HostState>,
    bindings: bindings::Application,
    info: AppInfo,
    limits: Limits,
    ticks: u64,
}

impl App {
    /// What the application declared about itself at startup.
    pub fn info(&self) -> &AppInfo {
        &self.info
    }

    /// Ticks simulated so far.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Simulate exactly one fixed tick.
    pub fn update(&mut self, input: Input) -> Result<()> {
        let tick = self.ticks;
        self.store.set_fuel(self.limits.fuel_per_call)?;
        self.store.data_mut().tick = tick;
        let result = self
            .bindings
            .portable_platform_app()
            .call_update(&mut self.store, input);
        if let Err(err) = result {
            return Err(self.fail(err, &format!("update of tick {tick}")));
        }
        self.ticks += 1;
        Ok(())
    }

    /// The application's current frame. Does not advance the simulation.
    pub fn render(&mut self) -> Result<Vec<DrawCommand>> {
        self.store.set_fuel(self.limits.fuel_per_call)?;
        self.store.data_mut().tick = self.ticks;
        let result = self
            .bindings
            .portable_platform_app()
            .call_render(&mut self.store);
        result.map_err(|err| self.fail(err, &format!("render after tick {}", self.ticks)))
    }

    /// Messages the application logged since the last call to this method.
    pub fn take_log(&mut self) -> Vec<LogEntry> {
        self.store.data_mut().log.drain(..).collect()
    }

    fn fail(&mut self, err: wasmtime::Error, during: &str) -> anyhow::Error {
        let name = self.info.name.clone();
        guest_error(err, &mut self.store, &name, during, self.limits)
    }
}

/// Turn a failed call into an error that says which application failed, when,
/// and why, including the last error the application itself reported. The wasm
/// backtrace stays attached as the error's cause.
fn guest_error(
    err: wasmtime::Error,
    store: &mut Store<HostState>,
    name: &str,
    during: &str,
    limits: Limits,
) -> anyhow::Error {
    let mut context = format!("{name} failed during {during}");
    if let Some(Trap::OutOfFuel) = err.downcast_ref::<Trap>() {
        context.push_str(&format!(
            ": it exceeded its budget of {} fuel per call",
            limits.fuel_per_call
        ));
    }
    let reported = store
        .data()
        .log
        .iter()
        .rev()
        .find(|entry| entry.level == Level::Error);
    if let Some(entry) = reported {
        context.push_str(&format!("\nthe application reported: {}", entry.message));
    }
    anyhow::Error::from(err).context(context)
}

/// Distinguish the common mistakes (not WebAssembly at all, or a core module)
/// from a malformed component, so the error can say what to do about them.
fn check_is_component(bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.starts_with(b"\0asm"),
        "not a WebAssembly binary (it does not start with the \\0asm header)"
    );
    // Bytes 4..8 are the version and layer; layer 0 is a core module and
    // layer 1 is a component.
    match bytes.get(6..8) {
        Some([1, 0]) => Ok(()),
        Some([0, 0]) => bail!(
            "this is a core WebAssembly module, not a component; portable applications \
             must be components (wrap the module with `wasm-tools component new`)"
        ),
        _ => bail!("unrecognized WebAssembly binary layer"),
    }
}

/// The host trusts nothing the application declares without checking it.
fn validate_info(info: &AppInfo) -> Result<()> {
    ensure!(
        (1..=64).contains(&info.name.len()),
        "the application's name must be 1 to 64 bytes long, not {}",
        info.name.len()
    );
    let canvas = &info.canvas;
    ensure!(
        (1..=4096).contains(&canvas.width) && (1..=4096).contains(&canvas.height),
        "{}: canvas {}x{} is outside 1x1 to 4096x4096",
        info.name,
        canvas.width,
        canvas.height
    );
    ensure!(
        (1..=1000).contains(&info.ticks_per_second),
        "{}: {} ticks per second is outside 1 to 1000",
        info.name,
        info.ticks_per_second
    );
    Ok(())
}
