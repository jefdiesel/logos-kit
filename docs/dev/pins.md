# Pins

Every pinned revision and version used by Logos Kit. Update this file whenever anything is re-pinned, and say why in `PROGRESS.md`.

Recorded 2026-09-26 (S0).

## Logos / LEZ upstream

| Component | Repo | Rev | Date | Notes |
|---|---|---|---|---|
| LEZ | `logos-blockchain/logos-execution-zone` | `db66590ab821a4e142c211017a3866d007f6fa77` | 2026-09-30 | Tag `v0.3.0`, which the official testnet runs. Re-pinned 2026-10-06 from `f7fda38` (`v0.3.0-rc1`): 19 commits, all seven patches applied unchanged, protocol vectors identical. The breaking change for us: `ApplyOutput` gained `chained_calls`, so every guest that uses `apply` (our testimonial program included) had to be rebuilt, and the builtins' images and the privacy circuit changed |
| Basecamp | `logos-co/logos-basecamp` | `2c2022762b397e5c5657a480bab9981466211e35` | 2026-09-22 | A `main` commit from the day of the 0.3.0 release. **Correction (2026-10-14):** tag `0.3.0` is `bbe5da0e` on a release branch, not this commit. Newer releases: `0.3.1` = `aeb81921` (2026-10-01, adds `basecamp://` deep links), `0.3.2` = `f2fae630` (2026-10-06, latest; bundles `storage_module`). Stage R re-tests our modules on 0.3.2. Pinned because some public releases did not discover user modules. **Installed locally:** release asset `LogosBasecamp-Desktop-v0.3.0-bbe5da-aarch64.dmg` (sha256 `82ea4cbdf6b32a7a04610a422f1326347b994984d5a07ef474c4968d2edd6d53`) at `~/Applications/LogosBasecamp.app`. The QML Inspector needs a source dev build at the same rev |
| logos-module-builder | `logos-co/logos-module-builder` | `4b7998272c5ec014bcac4bf1c7dbe7602c63a3c1` | 2026-09-23 | `mkLogosModule` / `mkLogosQmlModule` |
| logos-rust-sdk | `logos-co/logos-rust-sdk` | `bcc36420d7a15fb39cbf8079c85a18650cdae968` | 2026-09-24 | `current_caller()`, codegen trait |
| logos-modules-release-base | `logos-co/logos-modules-release-base` | `33fb564d2069be388ef53724bd5b0a6d026b2250` | 2026-09-21 | Our catalog (`logos-kit-modules`) is a fork of this |
| logos-modules-release-action | `logos-co/logos-modules-release-action` | `bbdc52871dc4305c8007923e1b08738ad2dd6b50` | 2026-09-23 | Called as `@v1` |

## Engine build (root `flake.nix`, 2026-09-26, S1)

Mirrors LEZ's own `flake.nix` at the LEZ rev above. When LEZ re-pins, copy these from its flake and lockfile together.

| Input | Pin | Why |
|---|---|---|
| LEZ source (`lez-src`) | `db66590a…` (`v0.3.0`) tarball + `vendor/lez-patches/0001–0007` | Same tree as `cargo xtask lez-vendor` (git-ignored `vendor/lez`) |
| logos-blockchain-circuits flake | `2846ee7a4cfa24458bb8063412ab2e753b344d2f` | LEZ's pairing for the lockfile's circuits `v0.5.7` (`ebf7ddf5…`) → `LBC_ROOT_DIR` |
| logos-blockchain-rust-rapidsnark flake | `e91187f8ccb5bbfc7bb00dac88169112428da78f` | Same rev as the lockfile → `RAPIDSNARK_LIB_DIR` |
| risc0 recursion zkr | hash read from the locked `risc0-circuit-recursion` crate's `build.rs` | Pre-fetched → `RECURSION_SRC_PATH` (no network in the sandbox) |
| crane / rust-overlay / logos-nix | `flake.lock` | nixpkgs follows `logos-nix`, as LEZ does |
| Apple Metal Toolchain | `17F109` (per-user component) | risc0-sys compiles Metal kernels on macOS; `xcodebuild -downloadComponent MetalToolchain` |

## Proving and toolchain

| Item | Value | Source |
|---|---|---|
| Rust toolchain | `1.98.1` | LEZ `rust-toolchain.toml` |
| risc0-zkvm | `3.0.5` | LEZ `Cargo.lock` |
| RISC Zero guest docker builder | `r0.1.91.1` | LEZ `Justfile:27` (`RISC0_DOCKER_CONTAINER_TAG`) |
| RISC Zero host rust toolchain (`rzup install rust`) | `1.97.0` | cargo-risczero reads it to pick the guest's rust flags even for docker builds; this version reproduced the testimonial image (`.github/workflows/guest-repro.yml`) |
| Nix | `2.35.2` | `/nix/var/nix/profiles/default/bin/nix` (flakes enabled) |
| Qt (Basecamp desktop) | `6.9.2` | note 11 |
| Qt (mobile Basecamp, and the open desktop bump PR) | `6.11.1` | logos-nix #7/#8 |

## JavaScript (latest on 2026-09-26; pinned through pnpm `catalog:`)

| Package | Version |
|---|---|
| next | 16.3.x |
| react / react-dom | 19.3.x |
| react-native | 0.87.x |
| expo | 57.x |
| react-native-passkeys | 0.4.2 |
| fumadocs-core / fumadocs-ui | 16.15.x |
| fumadocs-mdx | 15.4.x |
| fumadocs-twoslash | 4.x |
| fumadocs-typescript | 5.x |
| tsdown | 0.23.x |
| @changesets/cli | 3.x |
| @biomejs/biome | 2.5.x |
| vitest | 5.x |
| zustand | 5.x |
| @tanstack/react-query | 5.104.x |
| @noble/curves, @noble/hashes | 2.4.x |
| @noble/post-quantum | 0.7.x |
| ox | 1.8.x |
| typebox | 1.3.x (the new package name; not `@sinclair/typebox`) |
| @wallet-standard/app | 1.1.x |
| tailwindcss | 4.3.x |

## Proving benchmarks (S0, 2026-09-26)

**Machine:** `sysctl -n machdep.cpu.brand_string hw.ncpu hw.memsize` reports `Apple M1 Pro`, `10`, `17179869184` (16 GiB), on macOS (Darwin 25.2.0).

**What was run:**
- The code is LEZ at the pinned rev `f7fda38a`, in a scratch copy. The build was `cargo test --release -p lee --features prove --lib` with `RISC0_SKIP_BUILD=1`, toolchain 1.98.1 and risc0-zkvm 3.0.5.
- Each run executed the test binary directly under `/usr/bin/time -l` with `--exact`. The tests were `privacy_preserving_transaction::circuit::tests::prove_privacy_preserving_execution_circuit_{fully_private,public_and_private_accounts}`.
- The scratch copy adds only instrumentation to `execute_and_prove`. It adds an `eprintln!` of `prove_info.stats`, the prove time and the proof size. It also adds an optional `BENCH_SEGMENT_PO2` environment variable that calls `env_builder.segment_limit_po2(N)`, which is what E4 uses.

**Prover:** CPU on every run. In risc0-circuit-rv32im 4.0.4 and risc0-circuit-recursion 4.0.4, the Metal branch of `segment_prover()` / `recursion_prover()` is commented out. risc0-circuit-keccak 4.0.5 is the same. As a result, macOS falls through to `hal::cpu`, and nobody set `RISC0_PROVER`, so the default `LocalProver` was used.

**Noise:** Other work was running on the machine during these runs, including an Android emulator, browsers and other agents. The 1-minute load average was **8–58**, and swap sat at 8–12 GB used. Treat the times as upper bounds for an idle M1 Pro.

**Two memory figures:**
- "Peak RSS" is `maximum resident set size`.
- "Footprint" is macOS `peak memory footprint`. It includes compressed and swapped pages, so it is the better estimate of RAM needed on a device without swap.

| Experiment | Variant | Env | Wall time | Peak RSS | Footprint | Segments / total cycles / user cycles | Prover |
|---|---|---|---|---|---|---|---|
| E2 | fully private (1 in, 1 out, private) | default (10 threads, segment po2 20, keccak po2 17) | **469 s** (7.8 min) | 4.26 GB | 9.92 GB | 2 / 2,097,152 / 1,806,640 | cpu |
| E2 | public + private (shield-like) | default | **337 s** (5.6 min) | 4.28 GB | 9.98 GB | 2 / 1,114,112 / 924,182 | cpu |
| KECCAK | fully private | `RISC0_KECCAK_PO2=15` | 380 s | 4.49 GB | 9.92 GB | 2 / 2,097,152 / 1,806,640 | cpu |
| KECCAK | fully private | `RISC0_KECCAK_PO2=14` | 367 s | 5.40 GB | 10.15 GB | 2 / 2,097,152 / 1,811,484 | cpu |
| E3 | fully private | `RAYON_NUM_THREADS=4` | 692 s (11.5 min) | 4.34 GB | 9.87 GB | 2 / 2,097,152 / 1,806,640 | cpu |
| E3 | fully private | `RAYON_NUM_THREADS=2` | 1600 s (26.7 min) | 4.18 GB | 9.91 GB | 2 / 2,097,152 / 1,806,640 | cpu |
| E1 | fully private, executor only | `RISC0_DEV_MODE=1` | 0.09 s | 35 MB | 26 MB | 2 / 2,097,152 / 1,806,640 (paging 204,030, reserved 86,482) | none (dev mode) |
| E1 | public + private, executor only | `RISC0_DEV_MODE=1` | 0.06 s | 34 MB | 26 MB | 2 / 1,114,112 / 924,182 (paging 145,363, reserved 44,567) | none (dev mode) |
| E4 | fully private | segment po2 **16** | 3122 s (52 min) | 3.00 GB | 4.57 GB | **87** / 5,668,864 / 1,806,640 | cpu |
| E4 | fully private | segment po2 **18** | 757 s (12.6 min) | 2.26 GB | 5.09 GB | **10** / 2,490,368 / 1,806,640 | cpu |
| E4 | fully private | segment po2 20 | (not rerun) | — | — | 20 is the risc0 default (`DEFAULT_SEGMENT_LIMIT_PO2 = 20`), so this row equals E2 fully private. The run was stopped at 3 s to stay within budget | cpu |

The succinct proof was 225,883 B for fully private and 224,643 B for public+private, the same for every variant. Every proving run passed its test (`proof.is_valid_for(&output)`).

**Caveat:** these tests pass no dummy inputs and no `ciphertext_padding`. A wallet transaction padded to 7 slots will cost more cycles than this and was not measured here.

**Conclusions:**
- **`RISC0_KECCAK_PO2` does not reduce memory.** At 15, footprint was unchanged (9.9 GB). At 14, peak RSS rose to 5.4 GB and footprint to 10.2 GB. This rules out the "keccak ≈ 4 GB" inference (note 13, spike 1). The peak comes from the 2^20-cycle rv32im segment and recursion, not from keccak.
- **Segment size is the lever that works.** po2 18 roughly halves the footprint (9.9 → 5.1 GB) and cuts peak RSS to 2.3 GB, costing about 1.6× wall time: 10 segments, 757 s against 469 s, under similar load. po2 16 barely saves more footprint (4.6 GB) but produces 87 segments and takes about 6.7× as long. For a phone or low-RAM target, po2 18 is the sweet spot. It needs the one-line `segment_limit_po2` change in `execute_and_prove` (upstream PR or fork).
- **Proving time scales almost linearly with cores.** Total CPU time was about 2,200–2,400 s in every thread setting, so wall time is roughly that divided by the effective cores: 10 threads took 7.8 min, 4 threads 11.5 min and 2 threads 27 min. A phone with 2 to 4 usable big cores should expect **10–30 min** at default settings, and more with po2 18.
- **ETA to show in the proving UI (desktop, M1 Pro-class):**
  - Shield / public→private: "about 5–6 min".
  - Fully private send: "about 6–8 min". On an idle machine it should be faster (the KECCAK runs happened at a similar load and took about 6.2 min).
  - Show an indeterminate spinner with an elapsed timer, and keep proving in the background (lever L10). Do not promise a figure under a minute.
  - The executor dry run (E1, under 0.1 s) is cheap. Run it first to validate inputs and get the cycle count, then scale the ETA from total cycles. At default po2 20, this machine proved about 2.1 M cycles in about 6–8 min on 10 threads; slow it down proportionally for fewer cores.
- **Metal gives no speedup at risc0 3.0.5.** The Metal prover paths are compiled out upstream, so Apple GPU acceleration would need a risc0 upgrade or re-enablement. It is not available by just setting a flag.

## Our programs

| Program | Image id | Source | Built |
|---|---|---|---|
| testimonial | `8308e67d1f7d520f776736955514e3ddbae0aa6d8ce646e69a33da812f74337e` | this repo @ `bb7764af54455eba33ee0a81c05cce94ab64778f`, `programs/testimonial/methods/guest`, docker `r0.1.91.1` | 2026-09-26 (`programs/testimonial/artifacts/build.json`; lockfile seeded from LEZ's, risc0-zkvm 3.0.5) |

## QML engine gate and bundler (2026-09-27, S6)

| Item | Version | Why |
|---|---|---|
| PySide6-Essentials (desktop Basecamp) | 6.9.2 | Qt of Basecamp 0.3.0 desktop; `.qt/q692` via uv (`just qt-setup`) |
| PySide6-Essentials (mobile / next desktop) | 6.11.1 | logos-nix #7/#8; `.qt/q611` |
| esbuild | 0.28.x (catalog) | QML bundle stages A and C |
| @babel/core | 7.29.x (catalog) | Qt V4 `apply(TypedArray)` workaround plugin |
| @noble/curves | 2.4.0 | `@logos-kit/codec/sign` only (BIP-340) |
