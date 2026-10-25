//! `mica security check` and `mica security boot-profile`.

use std::process::ExitCode;

use anyhow::{Result, bail};
use serde::Serialize;

use mica_compat::{Authorization, Route};
use mica_probe::Os;
use mica_security::boot_profile;
use mica_security::{Policy, Readiness, Status, Verdict, eac_linux_v2};

use crate::session::Session;
use crate::ui::{self, line, para};

fn policy_for(session: &Session, game: Option<&str>) -> Result<Policy> {
    let Some(query) = game else {
        return Ok(Policy::default());
    };
    let Some(game) = session.database().find(query) else {
        bail!("{query} is not in MICA's compatibility database");
    };
    Ok(Policy {
        forbid_virtual_machines: game
            .route(Route::LinuxVm)
            .is_some_and(|route| route.authorization == Authorization::Forbidden),
    })
}

fn print_readiness(readiness: &Readiness) {
    line("profile", "Easy Anti-Cheat Linux v2 (Epic, announced 2026-10-14)");
    line(
        "verdict",
        match readiness.verdict {
            Verdict::Ready => "READY: every requirement MICA can check is met",
            Verdict::Incomplete => "INCOMPLETE: nothing failed, but some requirements could not be checked",
            Verdict::NotReady => "NOT READY: see the failed requirements below",
            Verdict::NotApplicable => "not applicable on this operating system",
        },
    );
    println!();
    for finding in &readiness.findings {
        println!("{} {}", ui::mark(finding.status), finding.title);
        println!("       Epic: {}", ui::wrap(finding.requirement, 13, 100));
        for detail in &finding.details {
            println!("       {}", ui::wrap(detail, 7, 100));
        }
        if let Some(fix) = &finding.fix {
            println!("       fix ({:?}): {}", fix.kind, ui::wrap(&fix.summary, 13, 100));
            for step in &fix.steps {
                println!("         - {}", ui::wrap(step, 11, 100));
            }
        }
    }
    let (add, remove) = readiness.kernel_parameters();
    if !add.is_empty() || !remove.is_empty() {
        println!();
        line("kernel", format!("add: {}", add.join(" ")));
        if !remove.is_empty() {
            line("", format!("remove: {}", remove.join(" ")));
        }
        para(
            "",
            "`mica security boot-profile` shows how to put these on a separate boot entry, \
             leaving your normal boot untouched.",
        );
    }
    println!();
    for note in &readiness.notes {
        para("note", note);
    }
    line("source", readiness.source);
}

#[derive(Serialize)]
struct WindowsCheck {
    secure_boot: Option<bool>,
    uefi: Option<bool>,
    tpm_present: Option<bool>,
    tpm_version: Option<u8>,
    virtual_machine: Option<String>,
    notes: Vec<&'static str>,
}

pub fn check(session: &Session, game: Option<&str>, json: bool) -> Result<ExitCode> {
    let host = session.host();
    match session.os() {
        Os::Linux => {
            let readiness = eac_linux_v2(host, policy_for(session, game)?);
            if json {
                ui::json(&readiness)?;
            } else {
                print_readiness(&readiness);
            }
            Ok(match readiness.verdict {
                Verdict::Ready => ExitCode::SUCCESS,
                Verdict::Incomplete => ExitCode::from(3),
                _ => ExitCode::from(1),
            })
        }
        Os::Windows => {
            let security = &host.security;
            let check = WindowsCheck {
                secure_boot: security.secure_boot,
                uefi: security.uefi,
                tpm_present: security.tpm_present,
                tpm_version: security.tpm_version,
                virtual_machine: host.virtualization.guest_of.clone(),
                notes: vec![
                    "Riot Vanguard requires TPM 2.0 (error VAN 9001: https://support.riotgames.com/en-us/riot/client/error-van-9001) \
                     and refuses virtual machines (VAN 9100: https://support.riotgames.com/en-us/riot/client/error-van-9100).",
                    "Some EA Javelin titles ask for Secure Boot; EA says Apex Legends does not at this time \
                     (https://www.ea.com/games/apex-legends/apex-legends/news/javelin-anticheat-announcement).",
                    "Windows anti-cheat runs natively here; MICA only reports these settings.",
                ],
            };
            if json {
                ui::json(&check)?;
            } else {
                line("platform", "Windows: anti-cheat runs natively, through each publisher's own client");
                line("UEFI", ui::yes_no(check.uefi));
                line("SecureBoot", ui::yes_no(check.secure_boot));
                line(
                    "TPM",
                    match (check.tpm_present, check.tpm_version) {
                        (Some(true), Some(version)) => format!("present, version {version}"),
                        (Some(true), None) => "present".into(),
                        (Some(false), _) => "not found".into(),
                        (None, _) => "unknown".into(),
                    },
                );
                line(
                    "VM",
                    check
                        .virtual_machine
                        .clone()
                        .unwrap_or_else(|| "no (bare metal)".into()),
                );
                println!();
                for note in &check.notes {
                    para("note", note);
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Os::MacOs => {
            para(
                "macOS",
                "Windows kernel-level anti-cheat (Easy Anti-Cheat's Windows driver, BattlEye, Riot \
                 Vanguard, EA Javelin) cannot run on macOS: not under Wine or CrossOver, and Riot \
                 and Epic forbid virtual machines such as Parallels for their games. Games that \
                 ship a native macOS client use the publisher's own macOS anti-cheat.",
            );
            Ok(ExitCode::SUCCESS)
        }
        Os::Other => bail!("security checks are not available on this operating system"),
    }
}

pub fn boot_profile(session: &Session, apply: bool, rollback: bool, yes: bool, json: bool) -> Result<ExitCode> {
    if session.os() != Os::Linux {
        bail!("boot profiles are for Linux");
    }
    if rollback {
        let removed = boot_profile::rollback(&session.context)?;
        if removed.is_empty() {
            println!("No boot entries created by MICA were found.");
        }
        for path in removed {
            println!("deleted    {}", path.display());
        }
        return Ok(ExitCode::SUCCESS);
    }
    let readiness = eac_linux_v2(session.host(), Policy::default());
    let (add, remove) = readiness.kernel_parameters();
    let plan = boot_profile::plan(&session.context, session.host(), &add, &remove);
    if json {
        ui::json(&plan)?;
        return Ok(ExitCode::SUCCESS);
    }
    if plan.nothing_to_do() {
        println!("The running kernel command line already has every parameter MICA can set.");
        let other: Vec<&str> = readiness
            .findings
            .iter()
            .filter(|finding| finding.status == Status::NotMet)
            .map(|finding| finding.title)
            .collect();
        if !other.is_empty() {
            println!("Still failing, and not fixable by kernel parameters: {}", other.join(", "));
        }
        return Ok(ExitCode::SUCCESS);
    }
    line("loader", format!("{:?}", plan.loader));
    line("add", plan.add.join(" "));
    if !plan.remove.is_empty() {
        line("remove", plan.remove.join(" "));
    }
    if let Some(base) = &plan.base_entry {
        line("copies", base);
    }
    println!();
    for step in &plan.steps {
        println!("  {step}");
    }
    if let Some((file, contents)) = &plan.new_entry {
        println!();
        println!("--- {file} ---");
        print!("{contents}");
        println!("---");
    }
    for warning in &plan.warnings {
        para("note", warning);
    }
    println!();
    for step in &plan.recovery {
        para("undo", step);
    }
    if !apply {
        if plan.automatic {
            println!();
            println!("Nothing was changed. To create this entry: sudo mica security boot-profile --apply");
        }
        return Ok(ExitCode::SUCCESS);
    }
    if !plan.automatic {
        bail!("MICA cannot create this entry automatically here; follow the steps above");
    }
    if !ui::confirm("Create this boot entry?", yes) {
        println!("Nothing was changed.");
        return Ok(ExitCode::SUCCESS);
    }
    match boot_profile::apply(&session.context, &plan) {
        Ok(path) => {
            println!("created    {}", path.display());
            for step in &plan.use_it {
                para("next", step);
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            bail!("permission denied: run it as root (sudo mica security boot-profile --apply)")
        }
        Err(error) => Err(error.into()),
    }
}
