{
  description = "MICA: the mica compatibility engine, and Portable Host v0 (one WebAssembly game component, native hosts for x86-64 and ARM64)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      crane,
    }:
    let
      lib = nixpkgs.lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      overlays = [ rust-overlay.overlays.default ];
      eachSystem = f: lib.genAttrs systems (system: f (import nixpkgs { inherit system overlays; }));

      # One pinned toolchain (rust-toolchain.toml) for every build.
      toolchainFor = p: p.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

      # The files the Rust builds read, so edits to docs or Nix code do not
      # rebuild the component or the hosts.
      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./.cargo
          ./crates
          ./examples
          ./wit
          ./tests/replay
        ];
      };
      common = {
        inherit src;
        strictDeps = true;
        version = "0.1.0";
      };

      # Builds for one native system. On x86_64-linux this also cross-compiles
      # the headless host for aarch64-linux.
      build =
        pkgs:
        let
          craneLib = (crane.mkLib pkgs).overrideToolchain toolchainFor;

          # The application component. Its bytes do not depend on the machine
          # that builds it, and every host below runs this one file.
          guestArgs = common // {
            pname = "moving-square";
            cargoExtraArgs = "--locked -p moving-square --target wasm32-unknown-unknown";
            CARGO_PROFILE = "guest";
            doCheck = false;
          };
          guest = craneLib.mkCargoDerivation (
            guestArgs
            // {
              cargoArtifacts = craneLib.buildDepsOnly guestArgs;
              nativeBuildInputs = [ pkgs.wasm-tools ];
              buildPhaseCargoCommand = "cargoWithProfile build ${guestArgs.cargoExtraArgs}";
              doInstallCargoArtifacts = false;
              installPhaseCommand = ''
                mkdir -p $out
                wasm-tools component new \
                  target/wasm32-unknown-unknown/guest/moving_square.wasm \
                  -o $out/moving-square.wasm
                wasm-tools validate $out/moving-square.wasm
              '';
            }
          );
          guestFile = "${guest}/moving-square.wasm";

          # The host's test suite replays the component above, so building a
          # host also runs every golden replay and diagnostics test on it.
          hostArgs = common // {
            pname = "portable-host";
            cargoExtraArgs = "--locked -p portable-host";
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [ pkgs.sdl3 ];
            PORTABLE_GUEST = guestFile;
          };
          hostDeps = craneLib.buildDepsOnly hostArgs;
          host = craneLib.buildPackage (hostArgs // { cargoArtifacts = hostDeps; });

          guestLogic = craneLib.cargoTest (
            common
            // {
              pname = "moving-square";
              cargoExtraArgs = "--locked -p moving-square";
              cargoArtifacts = null;
            }
          );

          clippy = craneLib.cargoClippy (
            hostArgs
            // {
              cargoArtifacts = hostDeps;
              cargoExtraArgs = "--locked --workspace";
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            }
          );

          # Real X key events against the SDL window, on a private Xvfb display.
          keyboard =
            pkgs.runCommand "portable-host-keyboard-test"
              {
                nativeBuildInputs = [
                  pkgs.nushell
                  pkgs.xvfb-run
                  pkgs.xdotool
                ];
              }
              ''
                export HOME=$TMPDIR
                xvfb-run -a nu ${./tests/keyboard.nu} ${host}/bin/portable-host ${guestFile} $out
              '';

          # The headless host for ARM64, cross-compiled from x86-64. SDL3 is
          # left out: a cross-built SDL3 is not in the binary cache.
          crossPkgs = import nixpkgs {
            localSystem = pkgs.stdenv.hostPlatform.system;
            crossSystem = "aarch64-linux";
            inherit overlays;
          };
          crossCraneLib = (crane.mkLib crossPkgs).overrideToolchain toolchainFor;
          crossArgs = common // {
            pname = "portable-host";
            cargoExtraArgs = "--locked -p portable-host --no-default-features";
            PORTABLE_GUEST = guestFile;
          };
          crossDeps = crossCraneLib.buildDepsOnly crossArgs;
          hostAarch64 = crossCraneLib.buildPackage (
            crossArgs
            // {
              cargoArtifacts = crossDeps;
              doCheck = false;
            }
          );
          # nixpkgs never runs the check phase of a cross build, so the ARM64
          # test suite (golden replays included) runs as this derivation's
          # build, through the QEMU runner crane configures for cross targets.
          hostAarch64Tests = crossCraneLib.mkCargoDerivation (
            crossArgs
            // {
              pname = "portable-host-aarch64-tests";
              cargoArtifacts = crossDeps;
              buildPhaseCargoCommand = "cargoWithProfile test ${crossArgs.cargoExtraArgs}";
              doInstallCargoArtifacts = false;
            }
          );

          # The Windows x86-64 host, cross-compiled from Linux with MinGW-w64.
          # It keeps the SDL3 backend: nixpkgs builds SDL3 for MinGW.
          winPkgs = import nixpkgs {
            localSystem = pkgs.stdenv.hostPlatform.system;
            crossSystem = lib.systems.examples.mingwW64;
            inherit overlays;
          };
          winCraneLib = (crane.mkLib winPkgs).overrideToolchain toolchainFor;
          winSdl3 = winPkgs.sdl3;
          # SDL3.dll; the import library the linker uses is in another output.
          winSdl3Dlls = "${lib.getBin winSdl3}/bin";
          winArgs = common // {
            pname = "portable-host";
            cargoExtraArgs = "--locked -p portable-host";
            buildInputs = [ winSdl3 ];
            PORTABLE_GUEST = guestFile;
          };
          winDeps = winCraneLib.buildDepsOnly winArgs;
          # portable-host.exe with the DLLs it loads beside it, the first place
          # Windows looks for them.
          hostWindows = winCraneLib.buildPackage (
            winArgs
            // {
              cargoArtifacts = winDeps;
              doCheck = false;
              postInstall = ''
                cp ${winSdl3Dlls}/SDL3.dll $out/bin/
              '';
            }
          );

          # Wine 11 in WoW64 mode runs every Windows binary below; nixpkgs
          # would otherwise pick its minimal wine64 build. Each derivation
          # creates a fresh prefix, without Mono, Gecko, or desktop menu
          # entries.
          wine = pkgs.wineWow64Packages.stable;
          wineSetup = ''
            export HOME=$TMPDIR WINEPREFIX=$TMPDIR/wine WINEDEBUG=-all
            export WINEDLLOVERRIDES='mscoree,mshtml,winemenubuilder.exe=d'
            # Quiet Wine's probes for system fonts (it brings its own) and for
            # Wayland (its windows go to Xvfb).
            export FONTCONFIG_FILE=${pkgs.makeFontsConf { fontDirectories = [ ]; }}
            export XDG_RUNTIME_DIR=$TMPDIR/xdg-runtime
            mkdir -m 700 $XDG_RUNTIME_DIR
            ${wine}/bin/wineboot --init
            ${wine}/bin/wineserver --wait
          '';
          # The Windows test suite, golden replays included, run under Wine as
          # this derivation's build, like the ARM64 suite under QEMU.
          hostWindowsTests = winCraneLib.mkCargoDerivation (
            winArgs
            // {
              pname = "portable-host-windows-tests";
              cargoArtifacts = winDeps;
              CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER = "${wine}/bin/wine";
              # The test binaries in target/ find SDL3.dll on Wine's PATH.
              preBuild = wineSetup + ''
                export WINEPATH="$(${wine}/bin/winepath --windows ${winSdl3Dlls})"
              '';
              # Given a terminal, Wine redraws console output for it, and some
              # builders' terminals lost the text. Through a pipe the tests'
              # output reaches the log unchanged; stdenv's pipefail keeps a
              # failure a failure.
              buildPhaseCargoCommand = "cargoWithProfile test ${winArgs.cargoExtraArgs} 2>&1 | cat";
              doInstallCargoArtifacts = false;
            }
          );

          # The Windows host's graphical backend under Wine, beside the Linux
          # host, on a private Xvfb display: both play the tour script in a
          # window, must end on its golden frame, and must draw the same pixels.
          windowsPlay =
            pkgs.runCommand "portable-host-windows-play"
              {
                nativeBuildInputs = [
                  pkgs.nushell
                  pkgs.xvfb-run
                  pkgs.imagemagick
                  wine
                ];
              }
              ''
                ${wineSetup}
                xvfb-run -a -s '-screen 0 1280x1024x24' nu ${./tests/play-compare.nu} \
                  ${guestFile} ${./tests/replay}/tour.script $out \
                  "linux=${host}/bin/portable-host" \
                  "windows=wine ${hostWindows}/bin/portable-host.exe"
              '';

          # The keyboard test again, with the Windows host under Wine.
          windowsKeyboard =
            pkgs.runCommand "portable-host-windows-keyboard-test"
              {
                nativeBuildInputs = [
                  pkgs.nushell
                  pkgs.xvfb-run
                  pkgs.xdotool
                  wine
                ];
              }
              ''
                ${wineSetup}
                xvfb-run -a -s '-screen 0 1280x1024x24' nu ${./tests/keyboard.nu} \
                  "wine ${hostWindows}/bin/portable-host.exe" ${guestFile} $out
              '';

          # The Windows installer, built on Linux with wixl (msitools): the
          # host, the DLLs it loads, and the moving-square component under
          # Program Files, and a Start-menu shortcut that plays it. Each
          # version gets its own product code, derived from the version, so a
          # newer MSI replaces an older installation.
          productCode =
            let
              hash = builtins.hashString "sha256" "portable-host ${common.version} x64";
              part = start: length: builtins.substring start length hash;
            in
            lib.toUpper "${part 0 8}-${part 8 4}-5${part 13 3}-8${part 17 3}-${part 20 12}";
          msiName = "portable-host-${common.version}-x64.msi";
          msi =
            pkgs.runCommand "portable-host-msi"
              {
                nativeBuildInputs = [ pkgs.msitools ];
              }
              ''
                mkdir $out
                wixl --verbose --arch x64 \
                  -D Version=${common.version} \
                  -D ProductCode=${productCode} \
                  -D HostDir=${hostWindows}/bin \
                  -D Component=${guestFile} \
                  -D License=${./LICENSE} \
                  -o $out/${msiName} ${./packaging/windows/portable-host.wxs}
              '';
          # Install the MSI under Wine, replay with what it installed, open the
          # Start-menu shortcut, and uninstall.
          msiInstall =
            pkgs.runCommand "portable-host-msi-install"
              {
                nativeBuildInputs = [
                  pkgs.nushell
                  pkgs.xvfb-run
                  pkgs.xdotool
                  pkgs.imagemagick
                  wine
                ];
              }
              ''
                ${wineSetup}
                xvfb-run -a -s '-screen 0 1280x1024x24' nu ${./tests/wine-msi.nu} \
                  ${msi}/${msiName} ${hostWindows}/bin ${guestFile} ${./tests/replay} $out
              '';

          # Every host replays every script with the same component file and
          # must produce byte-identical traces.
          crossArch =
            pkgs.runCommand "portable-host-cross-arch"
              {
                nativeBuildInputs = [
                  pkgs.nushell
                  pkgs.qemu-user
                  wine
                ];
              }
              ''
                ${wineSetup}
                nu ${./tests/cross-arch.nu} ${guestFile} ${./tests/replay} $out \
                  "x86_64=${host}/bin/portable-host" \
                  "aarch64=qemu-aarch64 ${hostAarch64}/bin/portable-host" \
                  "windows=wine ${hostWindows}/bin/portable-host.exe"
              '';

          isX86 = pkgs.stdenv.hostPlatform.isx86_64;
        in
        {
          packages = {
            inherit guest host;
            default = host;
          }
          // lib.optionalAttrs isX86 {
            inherit msi;
            host-aarch64 = hostAarch64;
            host-windows = hostWindows;
            cross-arch-report = crossArch;
            windows-play-report = windowsPlay;
            msi-install-report = msiInstall;
          };

          checks = {
            inherit
              guest
              host
              guestLogic
              clippy
              keyboard
              ;
          }
          // lib.optionalAttrs isX86 {
            inherit
              hostAarch64
              hostAarch64Tests
              hostWindows
              hostWindowsTests
              windowsPlay
              windowsKeyboard
              msi
              msiInstall
              crossArch
              ;
          };

          apps.default = {
            type = "app";
            program = "${pkgs.writeShellScript "play" ''
              exec ${host}/bin/portable-host play ${guestFile} "$@"
            ''}";
            meta.description = "Play moving-square in a window";
          };
        };

      builds = eachSystem build;

      # The `mica` command and its tests. They need neither SDL3 nor the
      # cross toolchains, so they build on macOS too, where Portable Host's
      # outputs are not defined.
      micaSystems = systems ++ [
        "aarch64-darwin"
        "x86_64-darwin"
      ];
      micaFor =
        pkgs:
        let
          craneLib = (crane.mkLib pkgs).overrideToolchain toolchainFor;
          micaArgs = common // {
            pname = "mica";
            version = "0.1.0";
            cargoExtraArgs = "--locked -p mica-cli";
            buildInputs = lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];
          };
          micaDeps = craneLib.buildDepsOnly micaArgs;
        in
        {
          mica = craneLib.buildPackage (
            micaArgs
            // {
              cargoArtifacts = micaDeps;
              doCheck = false;
              meta.mainProgram = "mica";
            }
          );
          micaTests = craneLib.cargoTest (
            micaArgs
            // {
              pname = "mica-tests";
              cargoArtifacts = micaDeps;
              cargoExtraArgs = "--locked -p mica-inspect -p mica-probe -p mica-security -p mica-compat -p mica-runtime -p mica-engine -p mica-cli";
            }
          );
        };
      micaBuilds = lib.genAttrs micaSystems (
        system: micaFor (import nixpkgs { inherit system overlays; })
      );
    in
    {
      packages = lib.genAttrs micaSystems (
        system: (builds.${system}.packages or { }) // { inherit (micaBuilds.${system}) mica; }
      );
      checks = lib.genAttrs micaSystems (
        system:
        (builds.${system}.checks or { })
        // {
          inherit (micaBuilds.${system}) mica micaTests;
        }
      );
      apps = lib.genAttrs micaSystems (
        system:
        (builds.${system}.apps or { })
        // {
          mica = {
            type = "app";
            program = "${micaBuilds.${system}.mica}/bin/mica";
            meta.description = "Run programs from other operating systems with the right backend";
          };
        }
      );

      devShells = eachSystem (
        pkgs:
        let
          crossCc = pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc;
          crossTool = pkg: name: "${pkg}/bin/${crossCc.targetPrefix}${name}";
        in
        {
          default = pkgs.mkShell {
            packages = [
              # The pinned toolchain plus editor support. `extensions` replaces
              # the file's component list, so repeat it.
              ((toolchainFor pkgs).override {
                extensions = (lib.importTOML ./rust-toolchain.toml).toolchain.components ++ [
                  "rust-src"
                  "rust-analyzer"
                ];
              })
              pkgs.wasm-tools
              pkgs.pkg-config
              pkgs.sdl3
              pkgs.just
              pkgs.nushell
              # `just test-keyboard`: real key events on a virtual X server.
              pkgs.xvfb-run
              pkgs.xdotool
            ]
            ++ lib.optionals pkgs.stdenv.hostPlatform.isx86_64 [ pkgs.qemu-user ];

            # ARM64 cross builds (`just test-arm64`): link with the cross GCC and
            # run test binaries under QEMU user-mode emulation. The cross
            # compiler is referenced only through these target-scoped variables;
            # adding it to `packages` would make it the default `CC`.
            env = lib.optionalAttrs pkgs.stdenv.hostPlatform.isx86_64 {
              CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER = crossTool crossCc "cc";
              CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER = "qemu-aarch64";
              CC_aarch64_unknown_linux_gnu = crossTool crossCc "cc";
              AR_aarch64_unknown_linux_gnu = crossTool crossCc.bintools "ar";
            };
          };
        }
      );
    };
}
