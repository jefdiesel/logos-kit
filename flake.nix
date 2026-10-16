{
  description = "Logos Kit: wallet engine (C ABI) for the Basecamp module, plus dev shell";

  # Ported from LEZ's own flake at the pinned rev (logos-execution-zone
  # v0.3.1 1a003bf, flake.nix): same crane build, same circuit/rapidsnark pins, the
  # pre-fetched risc0 recursion artifact and the Metal xcrun stub. Keep these
  # pins in step with that file whenever the LEZ rev moves (docs/dev/pins.md).
  inputs = {
    logos-nix.url = "github:logos-co/logos-nix";
    nixpkgs.follows = "logos-nix/nixpkgs";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    # Same revs LEZ pins for Cargo.lock's circuits v0.5.7 and rapidsnark e91187f8.
    logos-blockchain-circuits.url = "github:logos-blockchain/logos-blockchain-circuits/2846ee7a4cfa24458bb8063412ab2e753b344d2f";
    rust-rapidsnark.url = "github:logos-blockchain/logos-blockchain-rust-rapidsnark/e91187f8ccb5bbfc7bb00dac88169112428da78f";
    # LEZ at LEZ_REV (crates/wallet-engine/src/lib.rs). Patched below with
    # vendor/lez-patches into the same tree `cargo xtask lez-vendor` produces.
    lez-src = {
      url = "github:logos-blockchain/logos-execution-zone/1a003bfbfb89fcf987f62975cd08a401169e1b8c";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      logos-nix,
      rust-overlay,
      crane,
      logos-blockchain-circuits,
      rust-rapidsnark,
      lez-src,
      ...
    }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll = nixpkgs.lib.genAttrs systems;
      mkPkgs =
        system:
        import nixpkgs {
          inherit system;
          overlays = logos-nix.lib.nativeOverlays ++ [ rust-overlay.overlays.default ];
        };
    in
    {
      packages = forAll (
        system:
        let
          pkgs = mkPkgs system;
          lib = pkgs.lib;
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
          cargoLock = builtins.fromTOML (builtins.readFile ./Cargo.lock);

          # Our Rust sources (manifests, lockfile, .rs, the C header, the JSON
          # the engine compiles in) plus the
          # patched LEZ tree at vendor/lez, which the workspace path-depends on.
          ownSrc = lib.cleanSourceWith {
            src = ./.;
            name = "logos-kit-rust";
            filter =
              path: type:
              (craneLib.filterCargoSources path type)
              || (lib.hasSuffix ".h" path)
              || (lib.hasInfix "/.cargo/" path)
              # Compiled into the engine with include_str! (verify.rs, testimonial.rs).
              || (type == "directory" && (lib.hasSuffix "/registry" path || lib.hasSuffix "/artifacts" path))
              || (lib.hasInfix "/registry/" path && lib.hasSuffix ".json" path)
              || (lib.hasSuffix "/programs/testimonial/artifacts/build.json" path);
          };
          lezPatched = pkgs.applyPatches {
            name = "lez-patched";
            src = lez-src;
            patches = lib.sort (a: b: a < b) (
              map (f: ./vendor/lez-patches + "/${f}") (
                builtins.filter (f: lib.hasSuffix ".patch" f) (builtins.attrNames (builtins.readDir ./vendor/lez-patches))
              )
            );
          };
          src = pkgs.runCommand "logos-kit-src" { } ''
            cp -R ${ownSrc} $out
            chmod -R u+w $out
            mkdir -p $out/vendor
            cp -R ${lezPatched} $out/vendor/lez
          '';

          # risc0-circuit-recursion downloads a zkr zip from its build script;
          # pre-fetch it (hash read from the locked crate, as LEZ does).
          recursion = builtins.head (builtins.filter (p: p.name == "risc0-circuit-recursion") cargoLock.package);
          recursionCrate = pkgs.fetchurl {
            url = "https://static.crates.io/crates/risc0-circuit-recursion/${recursion.version}/download";
            sha256 = recursion.checksum;
            name = "risc0-circuit-recursion-${recursion.version}.crate";
          };
          recursionZkrHash =
            let
              hashFile = pkgs.runCommand "risc0-recursion-zkr-hash" { nativeBuildInputs = [ pkgs.gnutar ]; } ''
                tmp=$(mktemp -d)
                tar xf ${recursionCrate} -C "$tmp"
                grep -o '"[0-9a-f]\{64\}"' "$tmp/risc0-circuit-recursion-${recursion.version}/build.rs" | head -1 | tr -d '"' | tr -d '\n' > $out
              '';
            in
            builtins.readFile hashFile;
          recursionZkr = pkgs.fetchurl {
            url = "https://risc0-artifacts.s3.us-west-2.amazonaws.com/zkr/${recursionZkrHash}.zip";
            sha256 = recursionZkrHash;
          };

          # risc0-sys always compiles Metal kernels on macOS; under nix, xcrun
          # can't find the per-user Metal Toolchain. Same wrapper as LEZ's
          # flake: resolve metal/metallib from the toolchain's cryptex mount.
          # Needs `xcodebuild -downloadComponent MetalToolchain` once.
          metalStub = pkgs.writeShellScriptBin "xcrun" ''
            orig=("$@")
            sdk=
            tool=
            args=()
            while [ $# -gt 0 ]; do
              case "$1" in
                --sdk) sdk=$2; shift 2 ;;
                metal|metallib)
                  if [ -z "$tool" ]; then tool=$1; else args+=("$1"); fi
                  shift
                  ;;
                *) args+=("$1"); shift ;;
              esac
            done
            if [ -n "$tool" ]; then
              for cand in /var/run/com.apple.security.cryptexd/mnt/*/Metal.xctoolchain/usr/bin/"$tool"; do
                [ -x "$cand" ] || continue
                if [ "$tool" = metal ] && [ -n "$sdk" ]; then
                  sysroot=$(/usr/bin/xcrun --sdk "$sdk" --show-sdk-path 2>/dev/null || true)
                  if [ -n "$sysroot" ]; then
                    exec "$cand" -isysroot "$sysroot" "''${args[@]}"
                  fi
                fi
                exec "$cand" "''${args[@]}"
              done
              unset DEVELOPER_DIR SDKROOT
              export xcrun_nocache=1
            fi
            exec /usr/bin/xcrun "''${orig[@]}"
          '';

          # LEZ's build_utils finds `artifacts/` by walking up from the crate
          # dir; with the whole patched tree at vendor/lez it finds
          # vendor/lez/artifacts, so no vendoring override is needed.
          cargoVendorDir = craneLib.vendorCargoDeps { inherit src; };

          commonArgs = {
            inherit src cargoVendorDir;
            pname = "wallet-engine";
            version = "0.1.0";
            strictDeps = true;
            buildInputs = [
              pkgs.openssl
              pkgs.pcsclite
            ];
            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.clang
              pkgs.llvmPackages.libclang.lib
              pkgs.gnutar
              pkgs.python3
            ];
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            LBC_ROOT_DIR = logos-blockchain-circuits.packages.${system}.default;
            RAPIDSNARK_LIB_DIR = rust-rapidsnark.packages.${system}.rapidsnark;
            RECURSION_SRC_PATH = "${recursionZkr}";
            cargoExtraArgs = "-p wallet-engine --lib";
            doCheck = false;
            preBuild = ''
              export HOME=$(mktemp -d)
            ''
            + lib.optionalString pkgs.stdenv.isDarwin ''
              export PATH="${metalStub}/bin:$PATH:/usr/bin"
            '';
          };

          walletEngine = craneLib.buildPackage (
            commonArgs
            // {
              cargoArtifacts = craneLib.buildDepsOnly commonArgs;
              # buildPackage installs binaries only; ship the C ABI library + header.
              installPhaseCommand = ''
                mkdir -p $out/lib $out/include
                cp crates/wallet-engine/include/wallet_engine.h $out/include/
                find target -maxdepth 3 \( -name 'libwallet_engine.dylib' -o -name 'libwallet_engine.so' \) -path '*/release/*' -exec cp {} $out/lib/ \;
                ls $out/lib/libwallet_engine.* >/dev/null
              ''
              + lib.optionalString pkgs.stdenv.isDarwin ''
                install_name_tool -id @rpath/libwallet_engine.dylib $out/lib/libwallet_engine.dylib
              '';
            }
          );
        in
        {
          wallet-engine = walletEngine;
          default = walletEngine;
        }
      );

      # `nix flake init -t github:logos-kit/logos-kit#dapp`
      templates.dapp = {
        path = ./templates/basecamp-dapp;
        description = "A Basecamp app (ui_qml) on the Logos Execution Zone, with the Logos Kit SDK";
        welcomeText = ''
          Your Basecamp app is ready. Rename it in metadata.json, then:
            nix build .#lgx-portable
          and install result-portable/*.lgx in Basecamp. README.md has the rest.
        '';
      };

      devShells = forAll (system: {
        default = (mkPkgs system).mkShell { inputsFrom = [ self.packages.${system}.wallet-engine ]; };
      });
    };
}
