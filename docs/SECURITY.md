# Security and privacy

MICA is a launcher and diagnostic tool. Run it as a normal user. Native, Wine,
Proton, WSL and VM applications retain the permissions those environments give
them. A separate Wine prefix separates application state; **it is not a malware
sandbox**. Inspecting a program does not certify that it is safe to execute.
Portable Host's explicit WIT capabilities and Wasmtime limits apply only to
that separate component execution model.

## Consent and reversible operations

`doctor`, `inspect`, `plan`, and `security check` gather local information;
they do not apply security changes. `run --dry-run`, `install --dry-run`, and
`setup --dry-run` expose launch/package plans. `setup` asks before invoking
the distribution package manager; the manager may require elevation.
`--yes` is an explicit choice to skip an interactive confirmation, not the
default behavior.

Steam tool installation and desktop integration are explicit commands. They
do not inject into Steam. Desktop `--default` records previous associations
for removal. Library discovery can be disabled with
`mica config set detect_games false`.

Boot-profile application requires root and confirmation. It adds a separate
marked entry and keeps the original/default entry. Rollback removes marked
MICA entries only. Firmware, keys, kernels and anti-cheat services are never
silently installed or changed. Read the [boot recovery instructions](ANTI-CHEAT.md#gaming-boot-profile-experimental)
before applying; real-machine boot/rollback remains untested.

Uninstallation asks about MICA data, including prefixes that may hold saves.
Back up those files. Removing the binary does not uninstall independent
runtimes or games outside MICA's directories. Remove a Gaming Boot Profile
before removing MICA itself.

## Downloads and local execution

Release installers compare archive hashes with `SHA256SUMS` and refuse a
mismatch. This detects corruption relative to that manifest; it is **not a
publisher signature** and cannot protect against compromise of both files.
No release is published yet. Source fallback uses the selected Git branch and
`Cargo.lock`; a branch is mutable. Review scripts and pin a reviewed commit for
reproducible deployment. Runtime installation is delegated to the OS package
manager and its repository verification. Third-party launchers may make their
own downloads under their own policies.

Commands are constructed as argument arrays rather than concatenated shell
input. Binary parsers have truncation/invalid-offset tests, but the project has
not undergone a comprehensive security audit. Avoid running unknown binaries
or installers. PATH, runtime overrides, compatibility files, and local config
are trusted user-controlled inputs, not safe inputs from strangers.

## Privacy

MICA has **no telemetry implementation**. Diagnostics, configuration, discovered
libraries and launch logs stay local unless you share them. MICA does not need
Steam/Epic passwords and does not implement account login. Network use by
install scripts, package managers, runtimes and games follows those programs'
own policies; no-telemetry does not mean every launched program is offline.

`mica config path` lists the actual directories. `MICA_HOME` can place config,
data, state and cache under a dedicated test directory. Logs may contain user
names, paths, command arguments, environment details and application output.
Review and redact them before submitting a report.

## Anti-cheat boundary

MICA does not patch protected binaries, disable anti-cheat, spoof responses,
forge attestation, hide virtualization, or bypass publisher restrictions.
Its security report is not a vendor-signed trust verdict. EFI/X.509/TPM parsers
and the versioned key-name list are diagnostic aids, not certificate-chain
verification or a remote attestation service. The publisher's authorized
service makes the final decision.

To report a problem, include the MICA commit/version, OS, affected command, and
redacted logs. Use the repository's private vulnerability reporting facility
if available for exploitable security issues; do not publish credentials or
private recovery keys in an issue.
