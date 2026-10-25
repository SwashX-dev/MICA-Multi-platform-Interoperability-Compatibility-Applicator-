//! `mica`: run programs from other operating systems through the right
//! compatibility backend, and check what publishers' anti-cheat requires.

mod games;
mod programs;
mod security;
mod session;
mod steam;
mod system;
mod ui;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use session::Session;

#[derive(Parser)]
#[command(
    name = "mica",
    version,
    about = "Run Windows, Linux, and macOS programs on other operating systems, through the right compatibility backend",
    after_help = "Start with `mica doctor`. Anti-cheat: `mica compatibility <game>` and `mica security check`.\n\
                  MICA never bypasses, patches, or hides from anti-cheat software."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check this computer: system, graphics, runtimes, and what it can run
    Doctor(JsonArg),
    /// Identify a program, installer, or app bundle without running it
    Inspect {
        path: PathBuf,
        #[command(flatten)]
        json: JsonArg,
    },
    /// Show which backend `mica run` would use, and why
    Plan {
        path: PathBuf,
        #[command(flatten)]
        backend: BackendArg,
        #[command(flatten)]
        json: JsonArg,
    },
    /// Run a program (or an app installed through MICA) with the right backend
    Run(RunArgs),
    /// Install a program from its installer (.exe, .msi, .deb, .rpm, .dmg)
    Install(InstallArgs),
    /// Open a file from the desktop: installers install, programs run
    #[command(hide = true)]
    Open { path: PathBuf },
    /// List or remove applications installed through MICA
    Apps {
        #[command(subcommand)]
        action: Option<AppsAction>,
    },
    /// List the compatibility runtimes MICA found
    Runtimes(JsonArg),
    /// Install runtimes with your system's package manager (asks first)
    Setup(SetupArgs),
    /// List games in your Steam, Epic, and Heroic libraries and their anti-cheat
    Games(JsonArg),
    /// What MICA knows about a game here, and what you can legitimately do
    Compatibility {
        /// A game name, alias, Steam app ID, or executable name
        game: String,
        #[command(flatten)]
        json: JsonArg,
    },
    /// Platform security for publisher-authorized anti-cheat
    Security {
        #[command(subcommand)]
        action: SecurityAction,
    },
    /// Steam integration (launch options and an optional compatibility tool)
    Steam {
        #[command(subcommand)]
        action: SteamAction,
    },
    /// Linux file-manager integration ("Open with MICA")
    Desktop {
        #[command(subcommand)]
        action: DesktopAction,
    },
    /// Windows virtual machines: what this PC supports, and how to set one up
    Vm {
        #[command(subcommand)]
        action: VmAction,
    },
    /// Show or change MICA's settings
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Measure MICA's own overhead (inspection, decisions, launching)
    Bench {
        #[arg(long, default_value_t = 200)]
        iterations: u32,
        #[command(flatten)]
        json: JsonArg,
    },
    /// Remove what MICA created (asks about each item)
    Uninstall {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
struct JsonArg {
    /// Print machine-readable JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct BackendArg {
    /// Use this backend: native, rosetta, portable-host, dos, windows, wsl,
    /// arch-translation, linux-vm, vm
    #[arg(long, value_name = "BACKEND")]
    backend: Option<String>,
}

#[derive(Args)]
struct RunArgs {
    /// The program, or the id or name of an app installed through MICA
    target: String,
    #[command(flatten)]
    backend: BackendArg,
    /// Use this Wine prefix name instead of the program's own
    #[arg(long, value_name = "NAME")]
    prefix: Option<String>,
    /// Show the command without running it
    #[arg(long)]
    dry_run: bool,
    /// Arguments for the program
    #[arg(last = true)]
    args: Vec<OsString>,
}

#[derive(Args)]
struct InstallArgs {
    installer: PathBuf,
    /// The application's name (and its prefix name)
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    dry_run: bool,
    /// Do not ask for confirmation
    #[arg(long)]
    yes: bool,
}

#[derive(Subcommand)]
enum AppsAction {
    /// List applications (the default)
    List(JsonArg),
    /// Forget an application; --delete-prefix also deletes its Wine prefix
    Remove {
        app: String,
        #[arg(long)]
        delete_prefix: bool,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
struct SetupArgs {
    /// What to set up: windows, vulkan, vm, security, linux (default: windows)
    features: Vec<String>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}

#[derive(Subcommand)]
enum SecurityAction {
    /// Check Easy Anti-Cheat Linux v2's requirements (or Windows' anti-cheat prerequisites)
    Check {
        /// Apply a game's policy too (e.g. fortnite forbids virtual machines)
        #[arg(long)]
        game: Option<String>,
        #[command(flatten)]
        json: JsonArg,
    },
    /// Plan, create, or remove a separate anti-cheat boot entry (Linux)
    BootProfile {
        /// Create the entry (needs root; asks first)
        #[arg(long, conflicts_with = "rollback")]
        apply: bool,
        /// Delete every entry MICA created
        #[arg(long)]
        rollback: bool,
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        json: JsonArg,
    },
}

#[derive(Subcommand)]
enum SteamAction {
    /// Show Steam installations, Proton builds, and anti-cheat runtimes
    Status(JsonArg),
    /// Steam launch option wrapper: set a game's launch options to `mica steam launch -- %command%`
    Launch {
        #[arg(last = true, required = true)]
        command: Vec<OsString>,
    },
    /// Add "MICA" to Steam's compatibility tools (restart Steam afterwards)
    InstallTool,
    /// Remove MICA's compatibility tool
    RemoveTool,
    /// Entry point Steam uses for MICA's compatibility tool
    #[command(hide = true)]
    Run {
        verb: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum DesktopAction {
    /// Add "MICA" to the file manager's Open With list for Windows programs
    Install {
        /// Also make MICA the default for them (the previous defaults are recorded)
        #[arg(long)]
        default: bool,
    },
    /// Remove the entry and restore recorded defaults
    Remove,
}

#[derive(Subcommand)]
enum VmAction {
    /// Can this computer run a Windows virtual machine well?
    Check(JsonArg),
    /// Print the commands that create a Windows 11 virtual machine with libvirt
    Plan {
        #[arg(long, default_value = "win11")]
        name: String,
        /// Your own Windows installation ISO
        #[arg(long)]
        iso: Option<PathBuf>,
        #[arg(long, default_value_t = 8)]
        memory_gb: u32,
        #[arg(long, default_value_t = 4)]
        cpus: u32,
        #[arg(long, default_value_t = 80)]
        disk_gb: u32,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print where MICA keeps its files
    Path,
    /// Print the current settings
    Show,
    /// Change a setting, e.g. `mica config set windows.runtime wine`
    Set { key: String, value: String },
}

fn parse_backend(arg: &BackendArg) -> anyhow::Result<Option<mica_engine::Backend>> {
    arg.backend
        .as_deref()
        .map(|id| {
            mica_engine::Backend::parse(id)
                .ok_or_else(|| anyhow::anyhow!("unknown backend {id}; see `mica plan --help`"))
        })
        .transpose()
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let session = match Session::new() {
        Ok(session) => session,
        Err(error) => {
            eprintln!("error: {error:?}");
            return ExitCode::FAILURE;
        }
    };
    let result = match cli.command {
        Command::Doctor(json) => system::doctor(&session, json.json),
        Command::Inspect { path, json } => programs::inspect(&session, &path, json.json),
        Command::Plan { path, backend, json } => parse_backend(&backend)
            .and_then(|backend| programs::plan(&session, &path, backend, json.json)),
        Command::Run(args) => parse_backend(&args.backend).and_then(|backend| {
            programs::run(
                &session,
                &args.target,
                backend,
                args.prefix.as_deref(),
                args.dry_run,
                &args.args,
            )
        }),
        Command::Install(args) => {
            programs::install(&session, &args.installer, args.name.as_deref(), args.dry_run, args.yes)
        }
        Command::Open { path } => programs::open(&session, &path),
        Command::Apps { action } => match action.unwrap_or(AppsAction::List(JsonArg { json: false })) {
            AppsAction::List(json) => programs::apps_list(&session, json.json),
            AppsAction::Remove {
                app,
                delete_prefix,
                yes,
            } => programs::apps_remove(&session, &app, delete_prefix, yes),
        },
        Command::Runtimes(json) => system::runtimes(&session, json.json),
        Command::Setup(args) => system::setup(&session, &args.features, args.dry_run, args.yes),
        Command::Games(json) => games::games(&session, json.json),
        Command::Compatibility { game, json } => games::compatibility(&session, &game, json.json),
        Command::Security { action } => match action {
            SecurityAction::Check { game, json } => security::check(&session, game.as_deref(), json.json),
            SecurityAction::BootProfile {
                apply,
                rollback,
                yes,
                json,
            } => security::boot_profile(&session, apply, rollback, yes, json.json),
        },
        Command::Steam { action } => match action {
            SteamAction::Status(json) => steam::status(&session, json.json),
            SteamAction::Launch { command } => steam::launch(&session, &command),
            SteamAction::InstallTool => steam::install_tool(&session),
            SteamAction::RemoveTool => steam::remove_tool(&session),
            SteamAction::Run { verb, args } => steam::run(&session, &verb, &args),
        },
        Command::Desktop { action } => match action {
            DesktopAction::Install { default } => system::desktop_install(&session, default),
            DesktopAction::Remove => system::desktop_remove(&session),
        },
        Command::Vm { action } => match action {
            VmAction::Check(json) => system::vm_check(&session, json.json),
            VmAction::Plan {
                name,
                iso,
                memory_gb,
                cpus,
                disk_gb,
            } => system::vm_plan(&session, &name, iso.as_deref(), memory_gb, cpus, disk_gb),
        },
        Command::Config { action } => match action {
            ConfigAction::Path => system::config_path(&session),
            ConfigAction::Show => system::config_show(&session),
            ConfigAction::Set { key, value } => system::config_set(&session, &key, &value),
        },
        Command::Bench { iterations, json } => system::bench(&session, iterations, json.json),
        Command::Uninstall { yes } => system::uninstall(&session, yes),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
