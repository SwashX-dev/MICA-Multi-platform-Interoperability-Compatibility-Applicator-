{
  description = "Portable Host v0: one WebAssembly game component, native hosts for x86-64 and ARM64";

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

          # Both hosts replay every script with the same component file and
          # must produce byte-identical traces.
          crossArch =
            pkgs.runCommand "portable-host-cross-arch"
              {
                nativeBuildInputs = [
                  pkgs.nushell
                  pkgs.qemu-user
                ];
              }
              ''
                export HOME=$TMPDIR
                nu ${./tests/cross-arch.nu} ${guestFile} ${./tests/replay} $out \
                  "x86_64=${host}/bin/portable-host" \
                  "aarch64=qemu-aarch64 ${hostAarch64}/bin/portable-host"
              '';

          isX86 = pkgs.stdenv.hostPlatform.isx86_64;
        in
        {
          packages = {
            inherit guest host;
            default = host;
          }
          // lib.optionalAttrs isX86 {
            host-aarch64 = hostAarch64;
            cross-arch-report = crossArch;
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
            inherit hostAarch64 hostAarch64Tests crossArch;
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
    in
    {
      packages = lib.mapAttrs (_: b: b.packages) builds;
      checks = lib.mapAttrs (_: b: b.checks) builds;
      apps = lib.mapAttrs (_: b: b.apps) builds;

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
