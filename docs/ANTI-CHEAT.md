# Anti-cheat: what MICA can and cannot do

Status reviewed on **2026-10-25**. No protected multiplayer session has been
tested by MICA. Opening an executable, detecting a game, or passing a readiness
check does not establish multiplayer compatibility.

Anti-cheat checks whether a game's execution environment meets the publisher's
rules. A Windows kernel driver needs Windows kernel facilities that Wine and
Proton do not supply. An officially supported Linux implementation is a
different route, and the game's publisher must enable it.

## Start here

```sh
mica compatibility fortnite
mica compatibility apex
mica compatibility valorant
mica security check --game fortnite
```

These are diagnostics. They do not install anti-cheat or change firmware.
`met`, `not-met`, `unknown`, and `not-applicable` describe individual findings;
`incomplete` means MICA cannot verify everything. A readiness verdict is a local
assessment, not vendor attestation, enrollment, or permission to play.

## Fortnite and EAC Linux v2

Fortnite **is not available on Linux yet**. Epic has announced a native Linux
launcher preview, but a launcher is not a working Fortnite release. Epic plans
the game's Linux availability around its EAC Linux v2 rollout; no release date
is promised. Sources: [Fortnite Linux status](https://www.epicgames.com/help/c-34254770/c-33726977/can-i-play-fortnite-on-linux-a22610900),
[launcher announcement](https://store.epicgames.com/news/epic-games-launcher-preview).

MICA checks the categories in [Epic's EAC Linux v2 requirements](https://onlineservices.epicgames.com/news/introducing-easy-anti-cheat-linux-v2):

1. A trusted distribution-signed kernel and modules.
2. TPM 2.0 enabled.
3. UEFI Secure Boot with recognized signing keys.
4. Measured boot records.
5. IOMMU enabled.
6. Kernel lockdown in integrity mode or stronger.
7. kexec disabled.
8. BPF LSM available and enabled.

Epic also requires its authorized systemd service. MICA neither supplies that
service nor implements Epic's attestation protocol. Non-systemd Linux can use
MICA's general features, but does not satisfy that service prerequisite.
Self-signing a custom kernel is not equivalent to publisher-recognized trust.

Prefer your distribution's supported kernel, drivers, and security guidance.
MICA's parsers inspect EFI lists, certificate metadata, and TPM event records;
they do not cryptographically prove a boot chain or replace vendor verification.
An inaccessible event log is unknown, not evidence that attestation passed.

Fortnite does not support VMs or Parallels. Installing Windows in a VM does not
remove that restriction. Epic lists **GeForce NOW, Amazon Luna, and Xbox Cloud
Gaming** as remote alternatives; check region, device, and subscription
availability directly with the provider. MICA does not open or authenticate
these services for you. [Epic's VM policy and cloud options](https://www.epicgames.com/help/c-34254770/c-38015632/can-i-run-fortnite-on-windows-parallels-or-other-non-native-operating-systems-on-pc-a25107496).

## Apex Legends and EA Javelin

Apex moved from EAC to **EA Javelin on September 29, 2026**. EAC Linux v2
therefore does not solve Apex compatibility. EA blocks Linux, including Proton
and Steam Deck running Linux. A Steam installation or a detected executable
does not change that policy. Sources: [EA's transition announcement](https://www.ea.com/games/apex-legends/apex-legends/news/javelin-anticheat-announcement),
[EA's Linux policy](https://forums.ea.com/blog/apex-legends-game-info-hub-en/dev-team-update-linux--anti-cheat/9468053).

The database records no explicit permission for Apex in a VM, and MICA does not
offer it for protected multiplayer. Use a supported native Windows installation
(including dual boot where supported). MICA has not confirmed an Apex cloud
catalog entry. EA's announcement says Apex does not currently require Secure
Boot; do not apply another Javelin game's requirements to it automatically.

## EAC and BattlEye under Proton

Both require **per-game developer opt-in**. EAC's Linux module next to the
Windows DLL is a useful detection signal, not proof that the current build
can complete a multiplayer session. BattlEye requires title-specific setup
through its vendor/Valve. [Valve's integration instructions](https://partner.steamgames.com/doc/steamhardware/proton).

Install supported game builds and Proton through their official launcher.
Check the game's current policy and MICA's evidence before launching. A
community report is not an official authorization. MICA refuses a
`verified-playable` database rating without a direct reproducible test, and
protected multiplayer additionally needs an actual multiplayer test.

## Riot Vanguard

VALORANT requires TPM 2.0 and Secure Boot on Windows 11. Vanguard refuses
virtual machines (VAN 9100), and Wine/Proton does not provide its Windows
kernel environment. [Riot's PC requirements](https://support.riotgames.com/en-us/valorant/support-tools/minimum-recommended-pc-specs),
[VM restriction](https://support.riotgames.com/en-us/riot/client/error-van-9100).
Riot's [On-Demand update](https://www.riotgames.com/en/news/vanguard-on-demand)
also means Vanguard is not universally an always-on boot driver: qualifying
Windows configurations can load it on demand. That does not authorize Linux.

League's native Mac client is distinct from running its Windows client through
Wine. MICA has not validated that native client; the Windows Vanguard route
must not be generalized to every native macOS game.

## Gaming Boot Profile (experimental)

This prepares a **separate boot menu entry**, not an anti-cheat bypass. Inspect
the proposed changes first:

```sh
mica security check
mica security boot-profile
```

Where a suitable Boot Loader Specification entry exists, MICA can copy it with
additional kernel parameters and its own marker. It leaves the original entry
and default selection intact. Generated GRUB configurations, NixOS and
rpm-ostree receive manual instructions instead. Booting an actual machine with
this profile has **not been tested**; apply/rollback tests use a fixture sysroot.

Before applying, save disk-encryption recovery keys, confirm you can select
the original entry, and have your distribution's recovery media available.
Review the kernel/initramfs paths and all printed recovery instructions. Only
after deciding to accept those specific changes, run:

```sh
sudo mica security boot-profile --apply
```

MICA asks for confirmation. Reboot and manually select the new entry; then
rerun the security check. If it fails to boot, select the original entry.
Remove MICA's marked entries with:

```sh
sudo mica security boot-profile --rollback
```

If necessary, use recovery media to remove only the exact MICA-created entry
identified in the plan. MICA does not change firmware, enroll/remove Secure
Boot keys, replace kernels, or install anti-cheat services. Firmware or key
changes require the hardware/distribution's own documented procedure and
separate informed consent. No such changes are needed merely to try MICA.

## macOS and legitimate alternatives

Windows kernel anti-cheat cannot run through CrossOver, Wine, or the Game
Porting Toolkit on macOS. A graphics translation layer cannot solve this.
Use an officially supported native client where one exists, or an official
cloud service that offers the title. Dual boot into Windows is an option only
on hardware and OS versions that support it; it is not a general Apple Silicon
solution. Never hide a VM, patch a driver, spoof attestation, or disable
anti-cheat to make a protected game start.

The embedded sources and evidence dates live in
[`anticheat.toml`](../crates/mica-compat/data/anticheat.toml) and
[`games.toml`](../crates/mica-compat/data/games.toml). Policy can change; these
are versioned snapshots, not a live vendor feed. See [SECURITY](SECURITY.md)
for trust boundaries and [VALIDATION](VALIDATION.md) for actual tests.
