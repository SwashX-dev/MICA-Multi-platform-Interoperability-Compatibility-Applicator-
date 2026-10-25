# Compatibility architecture research

Primary sources reviewed on 2026-10-25. These explain design choices, not MICA
end-to-end test results. See [VALIDATION](VALIDATION.md) for those. MICA invokes
installed runtimes; it does not bundle their code or claim to replace them.

| Technology | Role in MICA | Practical boundary / source |
| --- | --- | --- |
| Wine | Windows API runtime discovery, per-app prefixes, EXE/MSI launch | User-space compatibility does not implement arbitrary Windows kernel drivers. [Wine source and license](https://gitlab.winehq.org/wine/wine) (LGPL family); WineHQ website returned HTTP 403 during review. |
| Proton | Steam-compatible runtime selection and command chains | Combines Wine with gaming components; Steam integration and the correct runtime/container environment matter. [Valve Proton](https://github.com/ValveSoftware/Proton). Component licenses differ. |
| DXVK | Discovered graphics needs / runtime guidance | Translates D3D8–11 to Vulkan; driver capabilities and per-game issues remain. MICA does not install DLL overrides automatically. [DXVK](https://github.com/doitsujin/dxvk) (zlib license). |
| VKD3D-Proton | D3D12 guidance through the selected runtime | Vulkan translation depends on GPU and driver features. [VKD3D-Proton](https://github.com/HansKristian-Work/vkd3d-proton) (LGPL-2.1). |
| umu | Preferred discovered Proton launcher outside Steam | Reuses a managed Steam runtime environment; upstream may download runtime components. [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher) (GPL-3.0). |
| QEMU/KVM | VM readiness and setup planning | A full guest OS with memory, storage, device and licensing costs. CPU emulation differs from hardware acceleration. [QEMU documentation](https://www.qemu.org/docs/master/) (primarily GPL-2.0). |
| libvirt | Start an existing user-configured Windows domain | Does not implement seamless guest app launch, automatic VFIO or controller passthrough. [libvirt](https://libvirt.org/) (LGPL). |
| CrossOver | Discover installed runtime and use a named bottle | Commercial product; per-bottle graphics settings belong to CrossOver. [CodeWeavers settings](https://support.codeweavers.com/advanced-settings-in-crossover-mac-26). No binaries or licenses supplied by MICA. |
| Game Porting Toolkit | Detect a locally installed evaluation environment | Apple's evaluation tooling is for evaluating ports, not a general redistribution license. MICA neither downloads nor bundles it. [Apple GPTK](https://developer.apple.com/games/game-porting-toolkit/). Read the license included with the exact downloaded version; individual toolkit components have different terms. |
| Rosetta | Detect availability for Intel code on Apple Silicon | Full availability through macOS 27; macOS 28 narrows it to certain older games. It translates CPU code, not Windows/Linux APIs. [Apple's support policy](https://support.apple.com/en-us/102527). |

License labels above are orientation, not a completed redistribution audit.
Any future bundled runtime release must inventory exact versions, preserve
notices, meet source obligations, and review proprietary component terms. MICA's
BSD-3-Clause license does not relicense those components. No GPTK or CrossOver
redistribution is authorized by this repository.

## Why orchestration first

Inspection, environment probes, evidence validation, policy enforcement and
launch planning are compact, testable Rust responsibilities. Reimplementing
the Windows API, graphics stack or a Windows kernel is not necessary to
coordinate proven runtimes and would vastly increase compatibility risk.
The Portable Host WIT contract remains useful for applications deliberately
compiled to it; it cannot absorb an arbitrary native application.

## Anti-cheat integration

[Valve](https://partner.steamgames.com/doc/steamhardware/proton) documents per-game
EAC/BattlEye opt-in. [Epic's EAC Linux v2 announcement](https://onlineservices.epicgames.com/news/introducing-easy-anti-cheat-linux-v2)
provides a legitimate platform-readiness target, but MICA does not supply Epic's
service or its trust decision. Distribution signing-key recognition, vendor
service release, publisher enablement and authorized multiplayer tests remain
external dependencies. A future partnership would need explicit vendor
integration/signing approval, not a compatibility workaround.

## Performance and future experiments

`mica bench --iterations 100 --json` measures selected MICA operations. It does
not measure game frame pacing, shader compilation, GPU translation, controller
latency or VM overhead. Future comparisons need the same title/build, runtime,
driver, machine, resolution and warm/cold-cache conditions, with repeated runs
and raw measurements. No claim of outperforming Proton, Wine or CrossOver is
supported by this release.

Useful future work includes discovery caching with invalidation, better
installer registration, explicit runtime provenance, physical ARM64 tests,
guest integration, and opt-in remote streaming. VFIO needs IOMMU groups,
appropriate spare devices, host recovery and publisher permission; single-GPU
systems are not promised seamless passthrough.
