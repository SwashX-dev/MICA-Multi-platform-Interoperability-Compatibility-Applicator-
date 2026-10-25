//! From a decision to a command line.

use std::ffi::OsString;
use std::path::PathBuf;

use mica_inspect::{Format, Installer};
use mica_runtime::launch::{self, LaunchPlan, WindowsTarget};
use mica_runtime::{Dirs, Runtimes, config};

use crate::decide::{Backend, Candidate, Runner, Target};

/// Build the command for `candidate`. `prefix_name` overrides the
/// per-application prefix; Windows programs otherwise get a private prefix
/// named after the program and its location.
pub fn prepare(
    candidate: &Candidate,
    target: &Target,
    args: &[OsString],
    runtimes: &Runtimes,
    dirs: &Dirs,
    prefix_name: Option<&str>,
    extra_env: &[String],
) -> Result<LaunchPlan, String> {
    let runner = candidate
        .runner
        .as_ref()
        .ok_or_else(|| format!("{} has no runtime to use", candidate.backend))?;
    let path = &target.path;
    let mut plan = match runner {
        Runner::Direct => {
            if matches!(target.report.format, Format::AppBundle(_)) {
                LaunchPlan::new("macOS (open)", "open")
                    .arg(path)
                    .arg("--args")
                    .args(args.iter().cloned())
            } else {
                launch::native(path, args)
            }
        }
        Runner::PortableHost { path: host } => launch::portable_host(host, path),
        Runner::DosBox { path: dosbox } => launch::dosbox(dosbox, path),
        Runner::Wsl { path: wsl } => launch::wsl(wsl, path, args),
        Runner::Translator {
            path: translator,
            name,
        } => launch::translated(translator, name, path, args),
        Runner::Lima { path: lima } => LaunchPlan::new("Lima (Linux virtual machine)", lima)
            .args(["shell", "default", "--"])
            .arg(path)
            .args(args.iter().cloned()),
        Runner::Libvirt { domain, uri } => {
            let mut plan = LaunchPlan::new("libvirt", "virsh").args(["-c", uri, "start", domain]);
            plan.notes.push(format!(
                "After the virtual machine starts, open it with: virt-viewer -c {uri} {domain}. \
                 Copy or share {} into it to run it there.",
                path.display()
            ));
            plan
        }
        Runner::Cloud { providers } => {
            return Err(format!(
                "remote play happens in the provider's own app or website ({}); MICA does not \
                 start it for you",
                providers.join(", ")
            ));
        }
        Runner::Umu { .. }
        | Runner::Proton { .. }
        | Runner::Wine { .. }
        | Runner::Crossover { .. }
        | Runner::GamePortingToolkit { .. } => {
            let windows = windows_target(target, args, runtimes, dirs, prefix_name, extra_env);
            match runner {
                Runner::Umu { path: umu, proton } => launch::umu(umu, proton.as_ref(), &windows),
                Runner::Proton { tool } => {
                    let install = runtimes
                        .steam
                        .iter()
                        .find(|install| {
                            tool.dir.starts_with(&install.root)
                                || install
                                    .libraries
                                    .iter()
                                    .any(|library| tool.dir.starts_with(library))
                        })
                        .or_else(|| runtimes.steam.first())
                        .ok_or("Proton needs a Steam installation")?;
                    launch::proton(install, tool, &windows)?
                }
                Runner::Wine { path: wine } => launch::wine(wine, &windows),
                Runner::Crossover { path: wine, bottle } => {
                    launch::crossover(wine, bottle, &windows)
                }
                Runner::GamePortingToolkit { path: wine } => {
                    let mut plan = launch::wine(wine, &windows);
                    plan.runner = "Apple Game Porting Toolkit (for evaluating ports)".into();
                    plan
                }
                _ => unreachable!("matched above"),
            }
        }
    };
    if candidate.backend == Backend::Rosetta {
        plan.runner = "native (Rosetta 2 translates it)".into();
    }
    plan.notes.extend(candidate.warnings.iter().cloned());
    Ok(plan)
}

fn windows_target(
    target: &Target,
    args: &[OsString],
    runtimes: &Runtimes,
    dirs: &Dirs,
    prefix_name: Option<&str>,
    extra_env: &[String],
) -> WindowsTarget {
    let name = target
        .game
        .as_ref()
        .map(|game| game.name.clone())
        .or_else(|| {
            target
                .path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "app".into());
    let prefix: PathBuf = match prefix_name {
        Some(name) => dirs.prefixes().join(name),
        None => dirs.prefixes().join(config::app_id(
            &name,
            target.path.parent().unwrap_or(&target.path),
        )),
    };
    let anticheats = target
        .scan
        .as_ref()
        .map(|scan| scan.detected())
        .unwrap_or_default();
    let eac = anticheats.contains(&mica_inspect::AntiCheat::EasyAntiCheat)
        && target
            .scan
            .as_ref()
            .is_some_and(|scan| scan.eac_proton_library);
    let battleye = anticheats.contains(&mica_inspect::AntiCheat::BattlEye);
    WindowsTarget {
        executable: target.path.clone(),
        args: args.to_vec(),
        msi: target.report.installer == Some(Installer::WindowsInstaller),
        prefix,
        steam_app_id: target.steam_app_id.clone(),
        eac_runtime: if eac {
            runtimes.anticheat.eac.clone()
        } else {
            None
        },
        battleye_runtime: if battleye {
            runtimes.anticheat.battleye.clone()
        } else {
            None
        },
        extra_env: extra_env
            .iter()
            .filter_map(|pair| pair.split_once('='))
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
        debug: std::env::var_os("MICA_DEBUG").is_some(),
    }
}
