# Logos Kit: progress

The single place to resume from after a context clear.

**Resume order:**
1. memory `MEMORY.md`
2. `docs/dev/status-2026-10-14.md` (§0 "Start here")
3. this file
4. `docs/dev/PLAN-0.4.md` (the plan in force; the current stage's section)
5. `docs/dev/pins.md`
6. the stage's "copy from" files

**Rules:**
- Tick an item only after its proving command passes, and paste the proof next to it.
- One branch per stage (`stage/NN-slug`).
- Nothing is pushed, published or deployed without the user's go-ahead.
- Tests are not a deliverable; only the integration-confidence tests listed in the plan.

**Next action (2026-10-14):** Logos moved every prize to testnet 0.4 and reset the live testnet onto LEZ 0.3.1, which removed the token, ATA and AMM programs (`docs/dev/status-2026-10-14.md`). The plan in force is `docs/dev/PLAN-0.4.md` v2, reviewed by Codex (`docs/reviews/plan-0.4/`). Next: merge PR #10 (stage 0), then stage R (`main` back on 0.3.1, native LGO) and stage X (transaction states + explorer), with the F1 protocol spikes and the N3a SDK contract in parallel. Check the Discord thread "LEZ 0.3 → 0.4: why the model is changing" for replies to our two posts first.

---

## Stage S0 · Foundation, branch `stage/00-foundation`

**Status:** in progress (started 2026-09-26)

### Repo skeleton
- [x] Git repo, directory layout, `LICENSE-MIT`, `LICENSE-APACHE`
- [x] `docs/dev/PLAN.md` (copy of the approved plan), `docs/dev/pins.md`, `PROGRESS.md`
- [x] Memory pointer to `PLAN.md` / `PROGRESS.md`; the 0.3 date is labelled a target
- [x] `README.md`, `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`, `.gitignore`, `.editorconfig`

### Tooling
- [x] TS tooling: pnpm 12.6 (`packageManager`), catalog + `minimumReleaseAge 1440` + `allowBuilds`, Biome 2.5 (migrated to `preset`), TypeScript 7.0.2, changesets 3, `tsconfig.base.json` (`isolatedDeclarations`), `tsconfig.qml.json` (ES2017). Proof: `pnpm check` → "No fixes applied"; `pnpm check:repo` → "No issues found". (`knip.json` deferred until packages exist)
- [x] Rust workspace: `Cargo.toml` (resolver 3, `modules/` and `programs/` excluded), `rust-toolchain.toml` 1.98.1, `crates/xtask`. Proof: `cargo clippy -p xtask -- -D warnings` clean; `cargo fmt --check` ok
- [x] `justfile` with Nix on PATH (check, check-rust, fingerprint, lgx, lgx-dev, build-modules, basecamp-reset)
- [x] Logos AI skills copied into `.claude/skills/` and **git-ignored** (upstream has no licence)

### Modules and probes
- [x] `modules/logos_kit_wallet` (rust-module template; `ping`, `whoami`, `pinged` event). The `path:../..` pattern is deferred to S1 (external-staticlib pattern from widespread)
- [x] `modules/logos_kit_wallet_ui` (imports Logos.Theme/Controls, paints its own bg, provides `lez.wallet.connect`, answers with probe data). A dependency needs a flake input named like the dep (`logos_kit_wallet.url = "path:../logos_kit_wallet"`) because the builder reads its LIDL
- [x] `modules/probe_dapp` (throwaway; direct call, intent, sandbox probes)
- [x] Identity-hop probe: `requesterName` vs `current_caller()`. Result and grant-key rule under *Probe results*
- [x] QML sandbox probes: Canvas paint, OS-opener path, bundled SVG, `textFormat` (results below; screenshots in `docs/reviews/s0/`)

### QA tooling
- [x] Build `logos-qt-mcp` (`MCP EXIT 0`; driven from Node via `test-framework/framework.mjs` over TCP :3768)
- [x] ~~Run the design-system storybook (before S7)~~ superseded (2026-09-30): the QML kit gallery `sdk/qml/gallery/` renders every component in light and dark (`docs/reviews/revamp/kit/`), and `apps/design-lab` is the web reference
- [x] Wallet design v2: `apps/design-lab` (React + 21st.dev/Motion Primitives/NumberFlow/Vaul components) with three working directions (Veil, Tray, Ledger); references study in `docs/design/references.md` + `docs/design/refs/`. Published for picks (artifact db collection `picks`).

### Design and chain tooling
- [x] Brand assets: official marks and lockups from guide.logos.co → `assets/logos/logos/`, recorded in `docs/design/brand.md`
- [x] Drafts of `docs/design/parity-ledger.md` and `docs/design/ux-spec.md` (v0)
- [x] `xtask fingerprint`. Proof (2026-09-26): testnet reports `"version": "0.2.x"`, lastBlockId 25947, programs [amm, authenticated_transfer, pinata, privacy_preserving_circuit, token], `getFeeState` → -32601
- [x] Proving benchmarks E1–E4 + `RISC0_KECCAK_PO2`, in `pins.md` ("Proving benchmarks"). M1 Pro 10-core/16 GB, CPU prover (risc0 3.0.5 has Metal commented out), load 8–58 so times are upper bounds. Fully private 469 s / 9.9 GB footprint; shield-like 337 s; 4 threads 692 s; 2 threads 1600 s; dev-mode executor 0.09 s. **KECCAK_PO2 does not cut memory** (worse at 14). **Segment po2 18: footprint 5.1 GB (from 9.9) for ~1.6× time**; po2 16 barely lower and 6.7× slower.

### Adoption
- [x] A-track drafts in `adoption/drafts/` (01 forum intro, 02 Discord, 03 signature-authorized private spends, 04 compact notes feed) plus `adoption/tracker.md`: **awaiting user approval to post**

### Exit criteria
- [x] `nix build ./modules/logos_kit_wallet#lgx-portable` succeeds on darwin-arm64. Proof: `EXIT 0`, `Added variant 'darwin-arm64' … logos-logos_kit_wallet-module-lib.lgx` (first build about 25 min). `logos_kit_wallet_ui` and `probe_dapp` lgx-portable builds also exit 0
- [x] The module loads in Basecamp and answers `ping` via `logos.callModuleAsync`. Proof: wallet UI shows `core: {"module":"logos_kit_wallet","ok":true,"version":"0.1.0"}` (`docs/reviews/s0/bc-wallet.png`)
- [x] The probe results are recorded below
- [x] `pnpm i && pnpm check` is green
- [x] Reviews run and findings recorded: S2/S3 security + code reviews (below), Codex milestone reviews `docs/reviews/s8/codex-review.md` and `docs/reviews/full-1/codex-review.md`

### Decisions and deviations
- 2026-09-26: the brand is Logos Kit (D12). The repo-local git identity is `Blockchain-Oracle <blockchainoracle.dev@gmail.com>`.
- 2026-09-26: **pnpm 12.6 and TypeScript 7.0.2** (the latest majors, verified in Context7). pnpm 11+ settings live in `pnpm-workspace.yaml`, and `allowBuilds` replaces `onlyBuiltDependencies`. tsdown uses tsgo/oxc for `.d.ts`. The docs app may need TS 6 for twoslash; check in S8.
- 2026-09-26: `minimumReleaseAge` blocked `@types/node@26.6.3` (under 24h old), so the range was widened to `^26.0.0` rather than weakening the guard.
- 2026-09-26: Proof UX numbers from the benchmarks: desktop ETA shield ≈5–6 min, fully private ≈6–8 min (elapsed timer + background proving; run the <0.1 s executor dry-run first to validate inputs and count cycles). Phones/paired provers: a LEZ patch **0006 (segment po2 via config)** is the memory lever; add it when S15 needs it. Real wallet txs pad inputs, so expect more cycles than the benchmark tests.
- 2026-09-26: **Design direction locked: Tray, light + dark (D14).** Tokens in `docs/design/brand.md`; reference implementation `apps/design-lab`. PLAN.md updated (rule 6, QML design system, S6 theme, S7 UI, S11 presets).
- 2026-09-26: Intent `params` types accepted by the shell are exactly `string | number | bool | object | array` (`IntentBroker.cpp:470-485`). Note it's **`bool`, not `boolean`**.

### Probe results
- **Headless core (logoscore 0.3.0, macOS arm64).**
  - Commands: `logosctl --config-dir <tmp> package install -y <lgx>` → `module load logos_kit_wallet` → `call logos_kit_wallet ping` / `whoami`.
  - Result: installed (unsigned, 0.1.0); loaded; `ping` → `{"module":"logos_kit_wallet","ok":true,"version":"0.1.0"}`; `whoami` → `{"kind":"host"}`.
  - Conclusion: `current_caller()` works through the scaffold. A CLI caller shows up as `host`.
- **Basecamp run (2026-09-26, inspector build `bin-bundle-dir-inspector` @2c20227, portable, macOS arm64).**
  - Install: `lgpm --modules-dir /tmp/lk-bc/modules --ui-plugins-dir /tmp/lk-bc/plugins --allow-unsigned install --file <lgx-portable>` ×3, then `LogosBasecamp --user-dir /tmp/lk-bc`. Both apps appear in the sidebar (user-installed modules **are** discovered by this build). Inspector port is up ~57 s after launch.
- **Identity hop.**
  - dApp → core directly: core sees `{"kind":"module","name":"probe_dapp"}`.
  - dApp → `lez.wallet.connect` intent → shell chooser ("Use this app?", provider listed by module id) → wallet UI gets `requesterName = "probe_dapp"`. When the wallet UI then calls the core, the core sees `{"kind":"module","name":"logos_kit_wallet_ui"}`.
  - After `respond`, the shell returns focus to the requester.
  - **Grant-key rule (S3):** `requester` = the module name. The shell's `requesterName` and `current_caller()` use the same namespace, so a grant made via an intent also matches that dApp's direct calls. The core can't see the original dApp on an intent hop; it trusts the `requester` only when relayed by `current_caller() == logos_kit_wallet_ui` (WalletUi). Any other module calling sensitive methods directly is `Module(name)` and needs a grant; `ui_*` methods are WalletUi-only.
- **Sandbox.**
  - `Qt.openUrlExternally("https://…")` → `false`; log: `Blocked URL import … scheme "https" is not allowed`. Links go through a core method (plan unchanged).
  - Bundled SVG (`qml/logo.svg`) → `Image.Ready`, renders.
  - **Canvas paints** in this build (green square visible). Deviation from the plan's caution: Canvas is allowed for QR/identicons on macOS; re-check on Linux in S7 before relying on it (Rectangle-grid fallback stays documented).
  - `textFormat: Text.PlainText` shows `<b>bold?</b>` literally. The rule stands.
  - `Logos.Theme/Controls/Icons` imports are redirected to the vetted design-system dir (log `Redirected Logos.Theme probe away from plugin tree`), confirming "import, never bundle".

### Local tools installed (user-level, `~/.local/share/logos-tools/`)
- `lgpm` 0.2.1 (sha256 `56523ecb…d52cc`)
- `logosctl` / `logoscore` 0.3.0 (sha256 `a49f7a41…402d7`)
- Basecamp 0.3.0 release app in `~/Applications`
- `basecamp-inspector` (bin-bundle-dir-inspector @2c20227) and `logos-qt-mcp`
- Apple Metal Toolchain 17F109 (`xcodebuild -downloadComponent MetalToolchain`; required by risc0-sys on macOS, per LEZ `flake.nix`)

### Review
(pending)

---

## Stage S1 · Protocol + engine links LEZ 0.3 + vectors, branch `stage/01-protocol`

**Status:** done (2026-09-26)

- [x] `@logos-kit/protocol` (LWS-0): TypeBox schemas → `schema/{lws0.schema.json,methods.json,intents.json}`; 12 methods, 6 notifications, 6 intents; `LezError` + codes. Proof: `publint --strict` and `attw --profile esm-only` pass; `dist/index.js` has no TypeBox import (`a97a3d1`)
- [x] `crates/wallet-engine` links `wallet`, `lee` (`prove`), `lee_core`, `key_protocol`, `common` at the pinned rev. Proof: `cargo test -p wallet-engine` builds with real Metal kernels, 5 tests pass (`aff4502`)
- [x] `cargo xtask vectors` → `protocol/vectors/{public_tx,keys}.json`: LEZ's 4 pinned message layouts, a signed native transfer (fixed aux rand, verified by LEZ's public verifier) incl. the `sendTransaction` base64 param and tx hash, public nodes `/`, `/0`, `/1`, `/0/0` (the wallet's layered order) and private nodes `/0`, `/1`
- [x] Integration test #1 (Rust side): `crates/wallet-engine/tests/vectors.rs` decodes the committed bytes with LEZ and re-verifies. Proof: `cargo test -p wallet-engine` → `vectors_*` 4 passed
- [x] LEZ fork patches in `vendor/lez-patches/` 0001–0007 (StorageBackend, SyncObserver, prepare/sign split, prove split, keycard feature, remove_label, no plaintext note on stdout)
- [x] typify → `crates/lwsp-types`: `cargo xtask types` (typify lib + pinned rustfmt). Emit now writes `$ref`s for named schemas (55), so types are named once (2.2k lines, was 7.6k). Proof: `cargo clippy -p lwsp-types -- -D warnings` clean (`17303d9`). CI diff lands with S9 workflows
- [x] Engine C ABI for the module: `lk_engine_info` / `lk_engine_call` / `lk_engine_free` (`include/wallet_engine.h`), JSON dispatch with `info` and `derivePublicAccounts`. Proof: exported symbols in `libwallet_engine.dylib`; dispatch test matches vectors (`b3e1ff8`)
- [x] Nix build of the module with the engine (macOS). Root `flake.nix` builds `wallet-engine` (crane, LEZ tarball + patches, circuits 2846ee7 ↔ v0.5.7, rapidsnark e91187f8, pre-fetched recursion zkr, Metal xcrun stub); the module takes it via `externalLibInputs` + CMake `EXTERNAL_LIBS` (a dylib, so the two Rust std copies never collide). Proof: `logosctl … call logos_kit_wallet engine_info` → `{"ok":true,"result":{"engine":"0.1.0","lezRev":"f7fda38a…"}}` (`ed0b5f6`). Lesson: without `EXTERNAL_LIBS` the plugin still links (dynamic lookup) and segfaults on the first engine call.
- [x] Same build on Linux (agari-box `nixos/nix` container, 4 GB/3 CPU): lgx `linux-amd64` with `logos_kit_wallet_plugin.so` + `libwallet_engine.so`. Proof (plain Debian, no Nix): plugin `NEEDED libwallet_engine.so`, `RPATH $ORIGIN`; engine exports `lk_engine_{info,call,free}`; ctypes call → `{"ok":true,"result":{"engine":"0.1.0","lezRev":"f7fda38a…"}}`. sha256 `6cc95e6a1fdc5344…`
- [x] LEZ patch series `vendor/lez-patches/0001–0005` (StorageBackend, SyncObserver, public prepare/sign, private prepare/prove/submit, keycard feature); verified in LEZ with check (default + no-default-features), clippy, 69 wallet tests (`b03bb7e`). Review note for S3: the keycard PIN is held in memory between prepare and sign. **Mechanism decided:** `vendor/lez/` = LEZ at the pinned rev + patches, generated by `cargo xtask lez-vendor`, and every LEZ crate comes from that path (never mixed git/path copies).

- [x] Linux builds (agari-box, `nixos/nix` container, 4 GB/3 CPU cap): all three modules `lgx-portable` for `linux-amd64` (variant name plain `linux-amd64`; plugin `logos_kit_wallet_plugin.so`). sha256 wallet `768a1a54…f0b2`, ui `f5c3f5a7…052a`, probe `f24b8590…7be2`. Note: UI flakes' `path:../logos_kit_wallet` needs a git checkout (a `git archive` copy fails in pure eval). Wallet cold build 420 s.
- [x] Code review (S0+S1) → 11 findings fixed in `4fa7cac` (QML gate `check:qml`, generated `provides`, vector coverage, rev/prefix self-checks, payload caps).

### Exit criteria
- [x] Module lgx with the engine builds and runs on darwin-arm64 and linux-amd64 (proofs above). **S1 complete (2026-09-26).**
- [x] `cargo test -p wallet-engine vectors` passes

### Decisions and deviations
- 2026-09-26: macOS builds of `lee/prove` need the Metal Toolchain (risc0-sys always compiles Metal kernels on macOS). Installed per LEZ's `flake.nix` note. `RISC0_SKIP_BUILD_KERNELS=1` is fine for `cargo check` only.
- 2026-09-26: vectors derive keys through LEZ's real `KeyTree` API, not our own path walk. The official wallet's layered order is `/0`, `/1`, `/0/0`, which a naive derivation would get wrong.
- 2026-09-26: the `keys.json` private entries embed LEZ's serde form of `KeyChain`, so the shape tracks upstream. Test mnemonic only (`abandon … about`), never funded.

---

## Stage S2 · Keystore, accounts, zones, sync, E2E harness, branch `stage/02-keystore`

**Status:** done (2026-09-26)

- [x] `vault.v1` (`crates/wallet-engine/src/vault.rs`): Argon2id (64 MiB/t3/p1; bounds on load 19 MiB–1 GiB, t 2–10, p 1–8) → XChaCha20-Poly1305, header as AAD, lock file + staged 0600 + fsync + rename + dir fsync, `Zeroizing`. `EncryptedBackend` = LEZ `StorageBackend` (patch 0001) with debounced saves + flush. Proof: `tests/keystore.rs` 5 passed (round trip, wrong password, header tamper/bounds, crash mid-write, password change, debounce) (`87db62c`)
- [x] Session + zones (`session.rs`): `keys/` vault holds the phrase, `zones/<id>/` holds LEZ Storage per zone; offline-first (create/restore/unlock/accounts/new account without a sequencer; `connect()` builds `WalletCore`, stays offline and usable on failure). Proof: `tests/session.rs` 2 passed (`ec81a10`)
- [x] E2E harness `e2e/standalone.sh` (standalone `sequencer_service` from vendor/lez, `RISC0_DEV_MODE=1`, debug genesis, loopback :3040; needs `r0vm` 3.0.5 via rzup). `just e2e`. Fingerprint: version 0.3
- [x] Integration-confidence #2 (network half): `tests/e2e_sync.rs` creates public + private accounts, connects, syncs (observer `start 1..=3`, `finish 3`), sync position survives lock/unlock (`9653513`)
- [x] **Key hierarchy** (`vault.rs`, `session.rs`): password → Argon2id **once** → keys vault (phrase + a random key per zone + authenticated wallet metadata). Zone vaults are keyed (`"kdf":{"alg":"none","context":"zone:<id>"}`, context in the AAD, so a swapped file fails). Password change rewrites one file. One exclusive `.session.lock` per wallet dir. Closes the follow-ups "derive once per unlock", "authenticate zones.json" (the keys-vault zone list is authoritative; a zone whose URL changed is refused) and the cross-zone swap (`34ea69b`)
- [x] Labels in LEZ's own label map (official CLI sees them) via LEZ patch **0006 `Storage::remove_label`**, mirrored in keys-vault meta so new zones get them; unique, trimmed, ≤32 chars
- [x] Restore date: new wallets = creation time, restore = a date or genesis; first sync of a zone binary-searches block timestamps (ms) and skips older blocks (1-day margin)
- [x] Persisted indexes + restore discovery: layered account counts in meta, replayed on every new zone (same accounts everywhere); restore derives a depth-6 tree (LEZ `restore-keys` semantics), prunes unused after the first sync
- [x] Auto-lock (`auto_lock.rs`, 1 min–24 h, default 15 min; checked on every use + host timer `tick`), `reveal_phrase(password)` (3 free tries, then doubling waits ≤5 min), `change_password`
- [x] Network drops: `NetStatus` Idle/Online{tip}/Offline{attempts, retryInMs, error} for the banner; capped exponential backoff + jitter (1 s → 60 s), fail-fast inside the window, `retry_now`. Proof: `cargo test -p wallet-engine` 19 passed; E2E on standalone: `e2e_create_accounts_and_sync` ok (fresh wallet scan starts at block 2: genesis carries an old timestamp), `e2e_restore_skips_blocks_before_birthday_and_discovers` ok (`mid 3 … start 3..=7`; `discovery tree 64 accounts → 2`). A used account surviving discovery gets its proof in S3 once we can send
- [x] Security review (S2 keystore): 12 findings, none critical. **Fixed:** one session per vault (exclusive `.session.lock`; blocks CLI/Basecamp lost updates and a password change being undone), pending saves kept until written + flush on drop + flush after a failed sync, all writes via random-name exclusive temp files (0600) + atomic rename, lock opened `O_NOFOLLOW`, data dirs 0700, zones/config written atomically, `Zeroizing` around serialized storage, `bip39`/`argon2` zeroize features, LEZ's plaintext `FileBackend` pointed at a never-read path, zone URL validated before any write + rollback of a failed create, KDF ceiling 256 MiB/t6/p4, phrase-taking FFI method test-only. Proof: engine tests 16 passed incl. 2 new regressions; E2E sync passes
- [ ] Security follow-ups (tracked): write counter in the AAD (rollback to an older vault file), zeroize-on-drop for LEZ `Storage` (LEZ patch), Windows owner-only ACL + exclusive lock. (Zone-id binding, authenticated zone list and one Argon2 per unlock are done above.)
- [x] Code review + security review of the S2 additions (`34ea69b`), 14 findings, all fixed or documented in the fix commit:
  - **security high:** the "orphan vault" cleanup could delete a real zone vault (case-alias zone id on case-insensitive disks; keys vault rolled back). Now zone ids are lowercase-only, the zone key is recorded *before* its vault is written, and a keyless vault is refused, never deleted (regression test `zone_state_is_never_deleted_on_a_guess`)
  - **security medium:** `change_password` was an unthrottled password oracle → one throttled `verify_current` for reveal + change
  - **security low:** `ZoneKey` zeroize-on-drop; keys record serialized into an exact-capacity zeroizing buffer
  - **code:** no new accounts while discovery runs; after cleanup, top up to the wallet's known counts (accounts match across zones); counts = distinct tree paths; auto-lock also measures wall time (sleep counts); only network failures enter backoff; label uniqueness across zones; `AutoLock::set` keeps the new session if the old flush fails; restore-date caveat documented; connect keeps bytes, not a second `Storage`
  - Accepted: a password change does not rotate zone keys (an old keys-vault copy + old password still opens zone vaults; it already holds the phrase). Proof: `cargo test -p wallet-engine` 21 passed; E2E 2 passed (`start 3..=6`, `64 → 2`)

### Exit criteria
- [x] Logos Kit creates accounts and syncs against the standalone sequencer (proof above). **S2 complete (2026-09-26).**

### Decisions and deviations
- 2026-09-26: **Offline-first session.** `WalletCore::new` fails with "Failed to find leader" when no sequencer answers, so the session keeps LEZ `Storage` itself until `connect()`. Creating, restoring and unlocking a wallet never need the network.
- 2026-09-26: Keys and per-zone state are separate vaults under one password: adding a zone restores the same phrase, so accounts match across zones (LP-0022).
- 2026-09-26: **Key hierarchy instead of one password per vault.** With password-sealed zone vaults a password change had to rewrite every vault (a crash mid-way splits them across two passwords) and each unlock ran Argon2 up to 3×. Zone vaults now use random keys held in the keys vault. Nothing is released, so there is no migration.
- 2026-09-26: `xtask lez-export` keeps patch file names and uses `--zero-commit`, so exports are deterministic (the 0001–0005 diffs in `29fb76e` are header-only).
- 2026-09-26: The engine now depends on LEZ's `sequencer_service_rpc` (client) for discovery's account lookups; the Nix engine build picks it up from the same vendored tree (re-verify with the next lgx build).

---

## Stage S3 · Policy authority, approvals, tx build/prove/sign, preflight, CLI, branch `stage/03-policy`

**Status:** done (2026-09-26)

- [x] `policy.rs`: `Caller = WalletUi | LocalOwner | Module(name) | Host | Bridge | Unknown` (the last three fail closed); grants `(zone, requester, account, capability)` stored in the **encrypted keys vault**; LWS-0 error codes (`code_of`)
- [x] `tx.rs`: native public transfer + shield via LEZ patches 0003/0004. Request hash = SHA-256(domain, chain|zone, requester, intent, message hash or signer nonce). Sign-time nonce re-fetch → `StaleApproval`; a proved private tx must touch exactly the approved public account + nonce. Review carries sender balance, fee cap/payer and the `getFeeState` base fee. Preflight: balance ≥ amount (+ max_fee when the sender pays; payer balance otherwise)
- [x] `engine.rs`: one pending request; echoed hash (mismatch cancels); atomic take; re-auth for private spends, amounts ≥ threshold, and connects; deadline 5 min; expiry on lock/zone switch/restart via an **epoch + zone** bound into each request; owner requests replace an app's; 30 s app cooldown; statuses readable by requester + owner only (apps get `errorCode`, not text); cancel during proving; proving on a blocking worker **outside** the wallet lock, prover slot RAII-guarded, no auto-lock mid-proof; shield outcome from the own-account invariant
- [x] `logos-kit` CLI (`crates/logos-kit-cli`, `LocalOwner`): init, restore (`--from-date`/`--from-genesis`), account list/new/label/import, balance, sync, send, shield, status, zones, reveal, password, auto-lock. The TTY review shows the decoded request, fee cap and request hash; `--yes` only with `LOGOS_KIT_PASSWORD`; `--json` needs `--yes`
- [x] LEZ patch **0007**: no plaintext private note on stdout (it went to module logs), no `unwrap` on note decode. LEZ wallet tests on the patched tree: 69 passed
- [x] Integration-confidence #3 `tests/authz.rs`: 7 passed, incl. the chain-backed `e2e_status_is_private_and_stale_approvals_are_refused` (wallet B's send moves the nonce; A's approval → 6106 `StaleApproval`, status `dropped`)
- [x] Exit proof, dev-mode proofs: `just e2e-cli` (`e2e/cli.sh`) → public send (block 44, recipient 500000) and shield with progress (`proving → signing → submitted → included`, block 48, outcome **Success** via own-account invariant, private balance 1234). `OK: public send + shield (debug, dev-mode proofs)`
- [x] Exit proof, **real proofs**: `LK_REAL_PROOF=1 e2e/cli.sh` (release, real RISC Zero proof on this Mac): shield `proving` 0 s → `signing` **267 s** → `included` 279 s (block 96), outcome **Success**; `OK: public send + shield (release, proofs)`; whole script 297 s, max RSS 4.27 GB
- [x] Code review + security review: 15 findings, fixed in `b43ad96` (details in the commit): zone-switch/lock races (epoch), app slot hogging (owner priority, cooldown, status cap), prover slot leak, auto-lock killing a proof, inclusion vs bookkeeping errors, sponsor fee check, zone id in hashes, error text to apps, CLI `--json` blind approve, date validation
- [x] (done in S4/S5) Deferred to S4 by scope: token send/balance, deshield/private send, faucet, verify-program, backup/restore commands, program-header re-check at sign time (S4 source verification); `testimonial` to S5

### Exit criteria
- [x] A public send and a shield (with progress) via `logos-kit` on standalone (dev-mode and real proofs, above). **S3 complete (2026-09-26).**

### Decisions and deviations
- 2026-09-26: **Private approval binds to accounts + nonces, not decoded effects.** A shield's proved message is checked to touch exactly the approved public account with the approved nonce. Full `expectedEffects` equality needs effect decoders (S4); for engine-built native transfers the circuit input is ours, so the binding holds.
- 2026-09-26: Approvals run the pipeline inline in `approve()` (progress via a callback) with proving outside the wallet lock. S7's module shim calls it from a worker thread and serves status reads meanwhile.
- 2026-09-26: LEZ still prints a few connect notices ("Statistics not found…") on stdout; JSON consumers read the last line. To silence at the source before S7 (another small LEZ patch).
- 2026-09-26: Imported public keys live in one zone's storage (not derived from the phrase, so not replayed on new zones).

---

## Stage S4 · Decoders, tokens, source verification, faucet, branch `stage/04-decoders`

**Status:** done (2026-09-26)

- [x] `decode.rs`: `Summary{title, program, lines, outflows, inflows, authorities, signers, unknown}` decoded from the signed message for native, token (all 7 instructions) and ATA (create/transfer/burn). Token authority use (create → mint authority, mint, print) is flagged; any row selecting an unexpected shard, trailing bytes, or an ATA instruction naming a non-builtin token program → `unknown`, which needs an explicit acknowledgement (`approve(.., acknowledged_unknown)`, CLI `--ack-unknown`) and the password. `(program account, decoder)` list is configurable (`Decoders::with`). Testimonial decoder lands with the program in S5
- [x] Private-tx public effects decoded (`private_effects`: native Debit/Credit, token Withdraw/Deposit, else raw `Other`). **Closes the S3 deviation**: the proved message must have exactly the approved effects, nonces and program-image claims (count bounded; disclosed claims must name the pinned program)
- [x] One `Transfer` intent, route from the accounts (public / shield / unshield / private), native or `token`; foreign private recipients via `toKeys` (npk/vpk, LEZ `show-keys` format) with a random identifier chosen at prepare and bound in the request hash; tokens held in an ATA are sent through the ATA program. Generic `Call` intent for dApps (every signing account needs the app's grant)
- [x] Preflight per program: native balance; token holding amount, recipient token slot (empty or same definition); fee payer vs `max_fee`. Sign-time re-check of signer nonces and of an upgradeable program's header (`StaleApproval`)
- [x] Tokens (`tokens.rs`): holdings in each account's own token shard (public live, private synced) + ATAs of tracked definitions (`token track`, auto-tracked on `token create`); names from definition accounts. `logos-kit token create` makes the demo token (replaces the planned `xtask demo-token`)
- [x] Source verification (`verify.rs`): live header at the program account's loader shard (`0xFE…`); builtin → compare with the image compiled from pinned LEZ (+ evidence file), registry-lite `registry/programs.json` → `claimed`, local rebuild cache `verified.json` by (zone, account, image) → `verified_local`, else `unknown`; `mismatch` when the live image differs. `logos-kit verify-program` clones the exact commit and runs `cargo risczero build` in docker `r0.1.91.1` (needs `rzup install rust`); `logos-kit program <addr|builtin name>`
- [x] Builtin evidence: `just verify-builtins` → `registry/builtins.json`. All 13 builtins rebuilt from LEZ `f7fda38a` in docker `r0.1.91.1` reproduce the pinned images (35 min incl. the 1.5 GB image pull). `e2e/tokens.sh` now requires the token program to be `verified_local` (was `claimed`); `just e2e-tokens` passes (5 min). Fixes found on the way: cargo-risczero printed image ids on stdout into the evidence JSON (build stdout now goes to stderr); `e2e/standalone.sh` reported "up" when a leftover sequencer held the port, and the run used its stale chain (faucet `outcome_unknown`). It now stops its previous instance and refuses a port someone else answers on
- [x] Faucet (`faucet.rs`): `FaucetBackend` → `funded | rate_limited | rejected | outcome_unknown` (protocol `RequestFundsResult` shape). `KeyFaucet` (funded key; per-account rate limit, idempotent request keys, one drop in flight, balance-attributed outcome à la `lez-faucet-ffi` `classify()`) = e2e `GenesisSupply` and the core of a self-hosted drip; `HttpFaucet` client. `Engine::request_funds`: private target = fund a public account of ours, then queue the shield for approval
- [x] Backup (`backup.rs`): `backup export|import`, the encrypted vault files byte for byte (nothing decrypted), import validated up front into an empty dir, password checked, partial writes removed
- [x] CLI: `send|shield|deshield` (`--token`, `--to-keys`/`--to-npk --to-vpk`), `token list|track|ata|create`, `faucet`, `program`, `verify-program`, `call`, `backup`, `account keys`, `balance --token`
- [x] Exit proof `just e2e-tokens` (`e2e/tokens.sh`, dev-mode proofs, standalone LEZ 0.3): faucet `funded` (public) and faucet → shield (private 5000); DEMO token created (1,000,000), public send 100, token shield 50, private token send 20, token unshield 5, native unshield 1234, **private payment to another wallet's keys (B finds 700 by syncing)**, ATA send 3, holdings 6, token program header `claimed`, backup restores (999843). `OK: S4 faucet + demo token on every route + backup`
- [x] Regression: `just e2e-cli` OK; engine tests with the sequencer: 10 unit + authz 7 + e2e_sync 2 + keystore 7 + session 6 + vectors 4 pass; clippy `-D warnings` clean
- [x] Security review (1 high, 4 medium, 6 low, 2 info) + code review (12): all fixed or documented in `7e0949e`. High: a dApp `Call` could make another wallet account sign (grant only checked on `from`)

### Faucet decision (plan decision tree, recorded 2026-09-26)
- LEZ 0.3 code has no faucet: no piñata program, `authenticated_transfer` gone; `lez-faucet-ffi` (pinned v0.2.2) and `token_mint_authority` FaucetMint (v0.2.4) target 0.2 and fail 0.3's fingerprint/account shape. The official faucet is public-only.
- **Launch-day action:** run `xtask fingerprint` on the 0.3 testnet. If an official faucet exists → an `HttpFaucet`-style backend for it; otherwise option 4: the disclosed drip service = `KeyFaucet` behind `POST /fund` on agari-box, funded from our testnet account. Every option sits behind `FaucetBackend`; the mini-app doesn't change.

### Decisions and deviations
- 2026-09-26: **Transfers are one route-aware intent** (the S3 `Shield` variant is gone): the route follows from which accounts are ours and private; `shield`/`deshield` CLI commands refuse a request whose route differs. Request-hash domain bumped to `approval/v2`.
- 2026-09-26: Re-auth now also for any token outflow and any undecodable call (security review), besides private routes, native ≥ threshold and authority use.
- 2026-09-26: A private payment to foreign keys lands under a fresh account id (npk, vpk, random identifier); the receiver's sync finds it as a new private account (LEZ's own behaviour). Surfacing that as "received" in the UI is S7 work.
- 2026-09-26: Program `image_id` hex = the 8 u32 words little-endian (`verify::image_hex`).
- Tracked: `Undisclosed` image claims are accepted (count-bounded) only when the approved program is immutable; binding them to the pinned image needs the mirror-commitment membership check. The drip-service HTTP server itself is launch-day work.

---

## Stage S5 · Testimonial program + deploy + evidence, branch `stage/05-testimonial`

**Status:** local exit proof passes (2026-09-27); testnet deploy waits for the 0.3 launch

- [x] Program (`programs/testimonial/`, own workspace, lockfile seeded from LEZ's so risc0-zkvm is 3.0.5): `core` (types, seeds, PDAs via sha2, checks, date math; no `lee_core`, so the engine links it) + `methods/guest` (`plan`/`apply` lib, 3-line `testimonial` bin). Rules: author signs and selects our shard; **top-level only** (`caller_account_id` must be `None`); text 1–280 B, username 1–32 B, submission 1–32 printable ASCII; control, direction, invisible and line-separator characters refused; `timestamp_window [ts − 2 min, ts + 10 min)`; `Posted` event (public data only). Proof: `cargo test` in `programs/testimonial` → 9 passed, incl. `pda_matches_lee_core`
- [x] Reproducible build: `logos-kit testimonial build` (= `verify::build` of a commit in docker `r0.1.91.1`) → `programs/testimonial/artifacts/{testimonial.bin, build.json}` image `8308e67d1f7d520f776736955514e3ddbae0aa6d8ce646e69a33da812f74337e` from commit `bb7764a`, `testimonial.bin` sha256 `aca042ea…62f6` (409 KB, 4 segments). The CLI build matched a separate manual `cargo risczero build` byte for byte (the second hit docker's layer cache)
- [x] Engine (`testimonial.rs`): the program is recognised by **image**, and trusted only when the header can't change (immutable, or the registry's deployment). Then: `verify` says `verified_local`, the testimonial decoder attaches (Call or `Testimonial` intent), posts are allowed. `Intent::Testimonial {from, program?, submission=LP-0021/logos-kit, username?, text}`: the wallet adds time, page and derived accounts; public author only; preflight input → trusted program → not posted yet. Outcome read back from the record (exact match incl. timestamp; `failure` when nothing was written). `Session::deploy_program` (LEZ program loader, header + segment keys derived from the phrase)
- [x] Evidence: `logos-kit testimonial evidence [--program …] [--snapshot dir]`: pages → records → per-month **new distinct authors** (earliest post across programs), LEZ-side tally cross-check, each author's other signed txs and balance, `target {total 150, perMonth 30, monthsMet (2 consecutive), immutable, met}`, tip block read first. Also `testimonial list`, `post`, `deploy` (immutable unless `--upgradeable`)
- [x] Exit proof `just e2e-testimonial` → immutable deploy `verified_local`; an `--upgradeable` copy of the same image is `claimed` and refused; posts from 3 accounts in 2 wallets, first post outcome `success` (read back from its record); second post, private author, direction override refused; `list` 3, `evidence` 3 distinct, tally consistent, `target.immutable` true, snapshot written. `OK: S5 testimonial deploy + posts + evidence (dev-mode proofs)`
- [x] Regression `just e2e-cli` OK, `just e2e-tokens` OK, `just e2e` OK; engine with the sequencer: unit 11 + authz 7 + e2e_sync 2 + keystore 7 + session 6 + vectors 4 pass; program workspace 9 pass; clippy `-D warnings` clean. Fixed on the way: S2's restore test asserted the fresh chain already had 4 blocks (true only after a cold build); it now waits for them
- [x] Security review (2026-09-27, 3 medium, 4 low, 2 info) + code review (2 high, 3 medium/low); all fixed or tracked here:
  - **medium:** a program the user signs for could chain-call the testimonial and post in their name (a signer's authorization reaches every program in a chained call; `execution_state.rs::authorize`) → posts must be top-level
  - **medium:** an upgradeable copy of our image at any address got `verified_local` + the decoder; a public tx names the account, not the image → trusted only if immutable or the registry's
  - **medium:** a 3000-author cap per submission could be filled by a griefer → paged stats (1000/page, page p opens only after p − 1 is full, checked by a read-only `Full` effect)
  - **high (code):** a failed duplicate post read back as success (only text/username compared) → the watch is decoded from the signed message and compared in full, timestamp included; a raw Call is pre-checked for an existing record too
  - **high (code):** `testimonial build` records the public repo URL but builds the local commit → warns when the commit is on no remote branch (push before publishing build.json)
  - evidence counted a repeat author in the month of the first *listed* program → earliest post; repeat authors deduped; `immutable` per program and in `target.met`
  - sheet: text with newlines could fake sheet lines → quoted, newlines shown as ⏎; more invisible characters refused; `list` escapes control characters
  - public reproducibility: `cargo risczero build` of `bb7764a` fetched from **github.com/logos-kit/logos-kit** in docker `r0.1.91.1` → image `8308e67d…337e`, binary sha256 `aca042ea…62f6`, identical to the committed artifact (2026-09-27)
  - tracked: evidence reads are sequential (fine at 150 authors; parallelise if it grows); a failed deploy leaves its uploaded segments orphaned (LEZ loader has no resume, upstream FIXME); `otherTxs` counts only txs the author signed (a rough activity signal)
- [ ] Staging deploy on the 0.3 testnet under a test submission id, then production `testimonial deploy` (immutable) + registry entry `{name: testimonial, chain: lez:testnet, account, imageId, source}`; `verify-program` from the public repo once pushed. **Blocked on the 0.3 testnet launch** (target 2026-09-30) and the maintainer's go-ahead to deploy and push

### Decisions and deviations
- 2026-09-26: **Records keyed by (submission, author), not by index.** In 0.3 `plan()` can't read the count, so an index-addressed record races every concurrent post and the user would re-approve. Record `(sub, author)` + authors listed in stats pages: no race inside a page, one-per-author falls out of the empty-record rule, enumeration needs no indexer. Replaces the plan's `tm(sub, i)`/`author(sub, author)` PDAs and the `index == count` retry.
- 2026-09-26: **Window −2 min / +10 min** instead of ±2 min: the approval sheet can stay open up to the 5-min deadline before the tx is sent, so the block may land well after the signed time.
- 2026-09-27: **Trust by image *and* fixity.** Production deploys immutable. A staging deploy may stay upgradeable only if the registry names it.
- 2026-09-27: The LWS-0 protocol is unchanged: a dApp proposes a generic instruction and the engine decodes it by image. S6's client gets a `postTestimonial` helper (borsh + sha256 PDAs + open page in TS).
- 2026-09-27: The `xtask deploy-testimonial` of the plan became `logos-kit testimonial deploy` (the engine holds the keys); it prints the registry entry instead of editing the file.

---

## Stage S6 · TS codec + client + minimal theme + QML SDK bundle, branch `stage/06-codec-client`

**Status:** done (2026-09-27)

- [x] `@logos-kit/codec`. The root is QML-safe (ES2017 output, no BigInt, no bignum library): borsh reader/writer, u64/u128 as decimal strings (own decimal ↔ LE math: bn.js was 45 KB of the QML bundle), hex/base58/base64/strict UTF-8, own SHA-256 (`@noble/hashes/sha2.js` builds SHA-512 constants with BigInt at import, which throws in Qt V4), account ids (public, PDA, builtins), public message/transaction encode/decode, message and tx hashes, the `sendTransaction` param, and builders for native, token and testimonial. `./sign`: BIP-340 via `@noble/curves` 2.4 (not QML-safe). `cargo xtask vectors` now also writes `protocol/vectors/programs.json` (builtin ids, native/token data, testimonial accounts + data). Proof: 17 tests pass against the Rust vectors, incl. the signed native transfer (signature, tx bytes, hash, RPC param) and 600+ random u128 values checked against BigInt
- [x] `@logos-kit/client` ("lez-viem"): `createClient({transport, chain}).extend(actions)`, with transports `http`, `custom` and `basecampModule`. Lossless JSON (integers past 2^53 stay strings; strict RFC 8259 numbers). Node actions: block number, block (header + public txs), account/view/balance/nonces, fee state, tx, `sendRawTransaction` (the hash is computed locally, never retried), `waitForTransaction`, testimonial open page. Wallet actions (LWS-0) incl. `readAccount`, `sendCall`, `postTestimonial` (resolves the page), `waitForTransactionStatus`. Backoff polling pauses while hidden and can be stopped. `./local`: raw-key `sendLocalCall`, one at a time per account. Proof: 12 tests on responses recorded from a LEZ 0.3 sequencer (`test/fixtures`, plus two labelled u128::MAX synthetic ones); the recorded block's tx hashes back to the wallet's hash and its signature verifies
- [x] `@logos-kit/theme`: Tray light + dark (D14), `cssVars`, and `toQmlTokens`/`qmlTokensModule` (colours as `#AARRGGBB`, one font family, numeric radii) → `sdk/qml/LogosKit/Tokens.js`. Proof: 2 tests
- [x] `packages/qml-bundle` → `sdk/qml/LogosKit/{logoskit.js (31.8 KB, target < 40 KB), Tokens.js, LogosKit.qml, qmldir}`. The research pipeline is ported (esbuild → Qt V4 babel fix → esbuild ES2016 IIFE, `.pragma library`), and the build refuses a bundle containing BigInt. `createLogosKit({callModuleAsync, openIntent})`: connect/send/sign/sign-in/faucet as intents with the **45 s timeout** and the **busy guard**. A timeout keeps the guard closed until the wallet really answers, the late answer goes to `onLateResult`, and sends carry an `id`. Reads go through the module; `transfer`, `sendCall`, `postTestimonial` and `watchTransaction` (backoff, pauses when hidden) are included. A bare SDK load in Qt takes 18–27 ms
- [x] **QML engine gate** (`just qt-setup` once, then `just qml-gate`): 32 checks **identical in Node, Qt 6.9.2 and Qt 6.11.1**: LEZ vectors, testimonial bytes, u128, lossless JSON, tokens, shims (`hasOwn`, `flat`, `fromEntries`), facade intents/busy guard/timeout + late answer/testimonial page read/watch
- [x] Exit proof on a live chain, `just e2e-client`: a native transfer built, signed (BIP-340) and submitted entirely in TypeScript lands on the standalone LEZ 0.3 sequencer (recipient 0 → 4242, the block lists the tx, the hash matches)
- [x] LWS-0 grows one read: `lez_readAccount {chain, account, program} → {nonce, data}` (chain state for dApps, a prize criterion); schemas re-emitted (13 methods), `crates/lwsp-types` regenerated
- [x] Security review (2 medium, 13 low/info) + code review (3 high, 2 medium), all fixed in `9207fbf`. Double-send risks: an intent timeout freed the busy guard while the sheet was open; http retried `sendTransaction`. Also: the testimonial page was stuck at 0, the transport guessed JSON from a string's first character, the hash came from the node, no bridge timeout, lax JSON/UTF-8/borsh ranges, Object.hasOwn recursion (see below), shard key prototype, decimals/kind validation, and poll counting hidden time

### Decisions and deviations
- 2026-09-27: **No bn.js.** LEZ amounts need only decimal ↔ LE bytes, compare and add; unit formatting is string work. This saved 45 KB of the QML bundle and a dependency everywhere.
- 2026-09-27: **Own SHA-256 in the codec root**, because noble's sha2 module touches BigInt at import. `./sign` keeps noble.
- 2026-09-27: **Biome must not touch the ported QML shims.** Its auto-fix turned `hasOwnProperty.call` into `Object.hasOwn`, which inside the shim that defines `Object.hasOwn` is infinite recursion. It also reformatted tested code. The research files are restored verbatim and excluded, and `noPrototypeBuiltins` is off for the QML-safe packages (`Object.hasOwn` is ES2022).
- 2026-09-27: **Intent timeout semantics.** The plan's 45 s timeout stays, but it can't mean "not sent" once the wallet sheet is open, so it rejects without freeing the guard. The module-shim contract for S7: `{"value": <json>}` with no double encoding, plus `id` dedupe.
- 2026-09-27: The `module` transport has its own JS-side timeout (bridge timeout + 1 s): a restarted module may never call back.

---

## Stage S7 · Basecamp core-module shim + wallet UI + first catalog release, branch `stage/07-basecamp`

**Status:** in progress (2026-09-27)

- [x] Engine service (`crates/wallet-engine/src/service.rs`): the JSON dispatcher fronts a long-lived service (one tokio runtime, background sync + auto-lock, portfolio snapshot, faucet jobs, proposal-id dedupe `5720`, event queue drained by the shim). LWS-0 reads + `lez_signAndSendTransaction` for modules; `ui_*` for the wallet UI only. Approvals answer at acceptance (password checked off the wallet lock first); inclusion is awaited without the lock. BIP-340 tagged-hash message signing and SIWE-shaped sign-in (`message.rs`), keyed per-app private handles, `lezpriv1:` receive codes (base64url npk‖vpk, fits one QR at ECC-L). Proof: `just e2e-service` (connect with wrong-password retry, app proposal, 6104 bad proposal, 5720 dedupe with handle, 4100 for ungranted signer whether ours or not, handle isolation, ui_* guard, signature verifies, wrong approval password keeps the request, shield proves and lands, public send outcome = success via own-account invariant, lock → 4900)
- [x] Module shim `logos_kit_wallet`: **contract-first LIDL** (`logos_kit_wallet.lidl`; a Rust-first trait can't express camelCase wire names), answers `{"value"}`/`{"error"}` as JSON values (a Rust `String` reaches `callModuleAsync` double-encoded; the client transport also unwraps one extra layer), `concurrency: "multi"` so reads never queue behind an approval. Proof: `logosctl call logos_kit_wallet lez_getCapabilities '{}'` → `{"value":{…}}`; host caller gets 4100 on `ui`
- [x] Wallet UI `logos_kit_wallet_ui` (Tray light/dark, Onest bundled, Rectangle QR/identicons, Shapes glyphs/ring): onboarding (create/confirm/restore/unlock + 5-try cooldown), home (account pill, network badge, balance card, tokens, chips, recent, faucet row, offline/syncing/empty), keypad send → review (balance change first, visibility, proof ETA, fee, source/upgrade badges, unknown-effects ack, password, 500 ms arm) → proof ring/phases → outcome, receive (public + private code/QR/fingerprint), accounts, settings (zones, privacy/endpoints, auto-lock, reveal phrase, connected apps/revoke, lock), proof island. Intents: connect (account picker, private consent), transaction send, message sign, sign-in (site acknowledgement), request funds, open
- [x] Dev harness (`modules/logos_kit_wallet_ui/dev/harness.py`): the real QML on the real engine without Basecamp; scripted QA `qa_onboarding.py`, `qa_wallet.py`, `qa_private.py` (shield, dApp intents, private→private by receive code, 360/680/1024, light). Screenshots in `docs/reviews/s7/`
- [x] Integration flow in a real Basecamp: `just bc-flow` (`e2e/basecamp.sh` + `tests/intent-flow.mjs` over the QML Inspector): wallet first run → probe dApp connect via the shell chooser → send → approve → handle → status `included / success` → balance read. Green 3× (screenshots `docs/reviews/s7/basecamp/`)
- [x] `lez.*` intent spec + upstream draft: `docs/protocol/intents.md`
- [x] Code review + security review, all findings fixed (`eb2fe8f`)
- [x] Catalog `logos-kit/logos-kit-modules` (public, from the `logos-modules-release-base` template): `submodules/logos-kit` + `modules.json` (monorepo module dirs; `release-all` reads it, `scripts/add-module-dir.sh` adds one), per-module workflows, **inline signing** (`lgx keygen` key → `LOGOS_SIGNING_KEY` secret, DID in `trustedSigners`), variants darwin-arm64/linux-amd64/linux-arm64, `release-on-merge.yml` gate (pointer on logos-kit `main`, equal versions, core → UI, self-healing), one concurrency group. Rehearsed: both modules `nix build .#lgx-portable` from the submodule layout; `validate-repo` OK; actionlint clean. Runbook `docs/dev/releasing.md`. **Released 0.1.0** (run 36303242296: 13/13 jobs green, ~40 min cold; core `.lgx` 223 MB for 3 variants, UI 475 KB); index lists both, signed by the release DID. **0.1.1** (run 36306093020, 35 min) carries the real app icon (index icon 6.7 KB, was the 1.5 KB template placeholder); clean install on fresh ubuntu:24.04 (agari-box): `CATALOG INSTALL OK (Linux x86_64)`, core reports 0.1.1
- [x] Linux lgx builds (catalog CI: linux-amd64 + linux-arm64 native runners) + clean installs from the catalog's `logos-repo.json`. Proof (0.1.0), `e2e/catalog-install.sh`: stock logosctl 0.3.0, empty session → catalog add → install UI (pulls core) → both signed by the release DID → core loads, `lez_getCapabilities` → `{"value":…}`: **`CATALOG INSTALL OK (Linux x86_64)`** (fresh ubuntu:24.04 on agari-box), **`(Linux aarch64)`** (fresh ubuntu:24.04, arm64), **`(Darwin arm64)`** (fresh session). GUI, `e2e/catalog-install-gui.sh`: Basecamp with an empty profile → Settings → Package Repositories → Add → Applications → tile → Install (resolves 2 required packages) → Launch → wallet welcome: **`CATALOG GUI INSTALL OK`** (screenshots `docs/reviews/s7/catalog/`). No macOS VM (28 GB free); the portable darwin bundle links only system libs, `@rpath` Qt/iconv/intl from the host app, no `/nix/store`
- [ ] Public/private native + token flows on testnet 0.3 (testnet still 0.2)

### Decisions and deviations
- 2026-09-27: **S7 code merged to `main` (`4a00950`) before the stage's distribution items**, so the catalog releases only commits on `main`. Catalog, Linux installs and testnet-0.3 flows continue on `main`.
- 2026-09-27: The catalog is a **template copy, not a fork** (forks start with Actions and scheduled workflows off; the index rebuild is scheduled). Windows is dropped from the variants (our root flake has no mingw target).
- 2026-09-27: Signing is supported (`signing_mode: inline`), so releases are signed. Clients default to WARN for unknown signers; the install check asserts the release DID rather than a refusal.
- 2026-09-27: Clean-machine installs are scripted with a stock `logosctl` (`e2e/catalog-install.sh [--docker]`: catalog add → install UI (pulls core) → signer check → load → call). Only 28 GB of disk is free here, so there's no macOS VM; macOS is tested with a fresh session plus a no-`/nix/store` link check on the portable bundle.
- 2026-09-27: **Package size.** The core `.lgx` is 223 MB because one `.lgx` carries every variant and each `libwallet_engine` is ~100–110 MB (71 MB `.rodata` = embedded proving artifacts; already stripped, `strip` saves 6 MB; ~69 MB gzipped per platform). Users download all three engines to use one; that is the lgx format, not ours. Candidate upstream ask: per-variant downloads in `package_downloader`. On a slow link Basecamp's install takes minutes (this Mac hit 44 KB/s from GitHub on 2026-09-27 while agari-box got 27 MB/s), and the GUI re-run for 0.1.1 timed out on that download, not on the package.
- 2026-09-27: Observed once: `catalog add` timed out (20 s) while three installs ran on this Mac at once; alone it takes < 1 s. Not reproduced; noted, not patched.
- 2026-09-27: **Events are emitted by the shim at the start of each call**, never from engine threads (the host emit callback isn't documented thread-safe); the UI also polls (2.5 s; 1 s while something runs).
- 2026-09-27: **Intent errors are the shell's six codes only**; anything else becomes `failed` with no detail. The wallet shows the reason to the user; `@logos-kit/protocol` maps `failed`.
- 2026-09-27: Apps propose **one public instruction** (`maxInstructions: 1`); private transfers start in the wallet. Relayed intent proposals don't need a grant (the user approves each); direct module proposals do.
- 2026-09-27: A receive code pays a fresh identifier (LEZ); the recipient's sync finds it. Paying your **own** code routes to that account (the sender wallet never rescans it).
- 2026-09-27: Canvas stays unused (Rectangle QR/identicons, Shapes for glyphs and the ring).
- Gotchas: onReqChanged runs before sibling bindings update (read `req` directly); Repeater delegates have no QObject parent (walk `childItems()`); Biome must skip `modules/*/qml` (Qt V4 JS); debug-build engines make private sync slow enough to trip the 15 s call budget (harness uses the release dylib).

---

## Stage S8 · Mini-apps + dApp template + conformance kit + minimal docs, branch `stage/08-apps`

**Status:** in progress (started 2026-09-27)

- [x] Signing-key backups (the user asked Claude to set them up): age-encrypted to the maintainer's SSH keys in `~/.config/logos/keys-backup/` and on agari-box `/root/backups/logos-kit/`, plus base64 in the macOS Keychain; each checked against the key's SHA-256. Restore steps in `docs/dev/releasing.md`
- [x] `sdk/qml/LogosKitUi`: Tray components for any Basecamp app (Theme, Txt, Btn, Card, Field, TextBox with a UTF-8 byte counter, Skeleton, Spinner, Tag, Notice, Identicon, LogosMark, Glyph, InfoRow). `just qml-vendor` copies `LogosKit/` + `LogosKitUi/` into every app and the template
- [x] SDK additions: codec `decodeTestimonial`, `decodeTestimonialStats`, limits, `TESTIMONIAL_PROGRAMS` (filled at deploy); client `readTestimonials`; facade `getTestimonials`, `getTestimonial`, `openExplorer`
- [x] LWS-0 `lez_openExplorer {chain, txHash|account}`: the wallet builds the explorer URL (testnet only; one per second), so sandboxed apps get links without being able to open arbitrary URLs
- [x] Dev harness for apps: `modules/logos_kit_wallet_ui/dev/app_harness.py` (app + wallet windows on one release engine; intents routed like the shell, the app's module calls carry its own caller identity)
- [x] **`logos_kit_testimonial`**: connect (public) → compose (must name Logos Kit; byte and hidden-character rules mirror `testimonial_core`) → approve in the wallet → pending steps → done + explorer. States: no program on chain, checking (skeletons), already posted, no funds (in-flow faucet), fresh-account nudge, approving, failed (rechecks the record: a rejected post may have landed). Live feed: count, this month, newest 25. Proof: `just e2e-testimonial-app` (deploy, seed, connect, faucet, validation, post, included in block 15, count 1 → 2, already-posted view; `docs/reviews/s8/testimonial/`). `lgx-portable` builds
- [x] **`logos_kit_faucet`**: connect (public + private) → pick → request. States: funded, rate-limited countdown, outcome unknown (watches the balance, never re-asks), declined, failed, and the private path (faucet → public, the user approves the shield in the wallet, proving timer, included). Proof: `e2e/faucet-app.sh` (public +1e9, second claim rate-limited to 0:00, private funded via an approved, proved shield; `docs/reviews/s8/faucet/`). `lgx-portable` builds
- [x] **`templates/basecamp-dapp`** (`nix flake init -t github:logos-kit/logos-kit#dapp`): teaching app (connect → balance → in-flow faucet → transfer → receipt), README, icon. The wallet contract is vendored (`dependency_overrides` → `logos_kit_wallet.lidl`), so the template's only input is the builder. Proof: `e2e/template-app.sh` (receipt included, block 167); `nix flake init -t git+file://…#dapp` in an empty directory + `nix build .#lgx-portable` → `logos-my_lez_dapp-module.lgx` (EXIT 0)
- [x] Both apps in a real Basecamp: `e2e/basecamp-apps.sh` (inspector build, lgpm installs of the four modules, local sequencer, CLI deploy): testimonial connects through the shell chooser, posts, approved, included (block 11, count 1); faucet's first claim rate-limited (onboarding had funded the account), countdown, second claim funded. `docs/reviews/s8/basecamp/`. Catalog install of the apps comes with the release
- [x] LWS-0 `lez_chainId` + SDK follows the wallet's network (start, shown, every 5 s; `followWallet: false` pins); apps reset on change. All three harness flows green (template 3 of 4 runs; one stall at the approval step, not reproduced)
- [x] **Testimonial page race** (`26e3cd0`): a post names its stats page when built (by the wallet or the app's SDK); if the page filled during approval, the program refused it and still charged the fee. `submit_public` re-reads the pages and rebuilds the same post (author, text, claimed time, nonce) for the open page, only for the trusted program, and the fee cap may not rise. Proof: `e2e/testimonial.sh` sends a post aimed at page 1 through `logos-kit call`; it lands on page 0 (the program alone would refuse it as "previous page not full")
- [x] `logos-kit call` always panicked (the global `--data` wallet-dir flag clashed with the call's `--data`); the wallet-dir flag is now `--home` (env `LOGOS_KIT_HOME` unchanged)
- [x] App identity on approval sheets: `ui_appInfo` reads the requester's `display_name` + PNG icon from `<user dir>/plugins/<app>/metadata.json` and sends the icon as a 40×40 `#AARRGGBB` grid (Basecamp's sandbox loads only `qrc:` and the wallet UI's own files, so no `data:` URL or foreign file path); `AppAvatar` draws it with rectangles. The attested module name stays under the app's own claims
- [x] Codex milestone review: `docs/reviews/s8/codex-review.md` (verdict: not a first release yet; 7 bugs, docs mismatches, conformance kit missing)
- [x] Codex bugs 2–7 fixed (`438e2cc`): testimonial flow frozen per post (no account switch in flight); Done only on `outcome: success` in all three apps, else re-read evidence then "not confirmed yet" + Check again; faucet watch always ends (30 tries / 2 min); private claims return `fundedAccount` (the public account paid); per-app network epoch drops late answers; `LogosKit.qml` `lateResult` (template keeps waiting past 45 s); wallet queues one request behind an answered sheet; `lez_openExplorer` needs a grant, 1/s overall + 20 per app per 10 min. Proof: `just e2e-testimonial-app`, `e2e/faucet-app.sh`, `e2e/template-app.sh` (now assert success + `fundedAccount` + unconfirmed state), `e2e/basecamp-apps.sh` green in real Basecamp; app icon renders in the real sandbox (no "Blocked" lines)
- [x] **Conformance kit** (`75dbd42`): `modules/logos_kit_wallet_fake` (11 fixed scenarios, no keys/chain, same module name, call log with caller identity), `just conformance <dapp>` (headless real QML per scenario, protocol-level checks), `just conformance-basecamp <dapp>` (real Basecamp chooser → fake; macOS), `scripts/capability-matrix.py` → `docs/protocol/capability-matrix.{md,json}` (`--check`). Proof: committed template failed `timeout` + `unknown_outcome` (Codex bugs 7, 5); fixed template (fresh `nix flake init`) 11/11; Basecamp mode 4/4. Guide: `docs/dev/conformance.md`
- [x] Docs review fixes (`4f35a7b`): lifecycle/outcome truth, faucet states, security claims (deployment pending, precise program gate, re-page, explorer limits), `onLateResult`, hand-written method notes merged into the generated reference, twoslash + `fumadocs-typescript` (`sdk/client`), `guides/other-modules` (wallet + `storage_module` receipt; not yet run in Basecamp), `metadataBase`. `pnpm build` green
- [x] Docs `apps/docs` (Fumadocs 16.15 / Next 16.3): intro, quickstart, testimonial + faucet worked examples, QML SDK, UI kit, errors, intents, security model, method reference generated from `protocol/schema` + LIDL; `llms.txt`/`llms-full.txt`, `.md` routes, search, OG; Tray landing with real screenshots; `context7.json`. `pnpm build` green. Not deployed yet (needs a domain decision)
- [x] **Preview network, don't wait for testnet 0.3** (maintainer's call, 2026-09-28): the official testnet still runs 0.2 (fingerprint: head doesn't decode with our 0.3 code), so we run a public LEZ `v0.3.0-rc1` network on agari-box via Coolify (project `logos-kit`, app `yb4jahtqekkpei3aqrajc8td`, compose `deploy/preview-net`): sequencer `https://lez.84.46.247.92.sslip.io`, faucet `https://lez-drip.84.46.247.92.sslip.io` (`crates/logos-kit-drip`: KeyFaucet + per-IP budget; 1 LEZ, 1/h per account, 5/h per IP). Own genesis (treasury `BGFjY2…8Uth`, ops `BtFkk8…Z2xX`), real proof verification, CORS + per-IP rate limit via our own Traefik routers. Binaries: GitHub release `preview-net-v0.1.0` (sha256-pinned in the Dockerfiles). Secrets `~/.config/logos/preview-net/secrets.env`, backed up age-encrypted (`keys-backup/logos-kit-preview-net.tar.age`, agari-box `/root/backups/logos-kit/`). Wallet zone `lez-preview` / `lez:preview` is the default (CLI + Basecamp), URL permanent (a real domain becomes an alias)
- [x] Testimonial program on `lez:preview`: immutable `4vjENywUCfC3h85mjNGFPV7R9DqvUjV2xhMkCZbJR8XK` (image `8308e67d…`, `verified_local`) in `registry/programs.json` + `TESTIMONIAL_PROGRAMS`. Posts there don't count for LP-0021 adoption (official testnet only); redeploy when it runs 0.3
- [x] **Prize flows on the live preview network, real proofs** (`e2e/preview-flows.sh`): drip faucet, public send (block 61), shield (103, ~7 min proof), private→public (150), token public send (178), token private send (218), testimonial post (221). Found + fixed: public token sends always ended "outcome unknown" (token-balance invariant added); CLI `--key-env` lost to the preview's default drip
- [x] **Catalog release** (2026-09-28): `logos_kit_wallet` + `logos_kit_wallet_ui` 0.1.2, `logos_kit_testimonial` + `logos_kit_faucet` 0.1.0 (catalog `dbf3e9e`, logos-kit main `9d952ca`; `release-on-merge.yml` now releases the apps after the wallet). Nix rehearsal of all four `lgx-portable` builds first. Clean installs of all four, each signed by the release DID: `e2e/catalog-install.sh` on macOS arm64, `--docker` on Linux arm64 and (agari-box) Linux x86_64
- [x] Docs live: https://logos-kit-docs.vercel.app (Vercel project `logos-kit-docs`, root `apps/docs`, `.vercelignore` keeps only the pnpm workspace + schema + `.lidl`)
- [ ] Testimonial program on the official testnet when it runs 0.3 (redeploy + registry entry)
- [ ] Demo takes, usability sessions (2–3 non-expert users; needs the maintainer to line up people)
- [ ] Follow-ups: `capability-matrix-check` in CI (S9); conformance Basecamp mode on Linux; run the other-modules example in Basecamp

### Decisions and deviations
- 2026-09-27: **Bugs found by driving the apps, fixed:** the local genesis faucet was rebuilt per request, so its per-account limit never applied (now kept per sequencer; window 60 s); `ui_requestFunds` didn't resolve an app's `pvt_…` handle (private claims failed); the wallet cleared an answered faucet request, leaving an empty sheet with no Done; a request that arrived while another sheet was open (the shield after a private claim) never showed.
- 2026-09-27: Wallet UI follow-ups from the first Basecamp run: requester initials → real app icon + name (done above). The fee "≤ 134,400,000 LEZ" is correct, not a unit bug: native LEZ has no decimals, so fees and balances are both whole LEZ.
- 2026-09-27: The Basecamp sidebar shows "LO" initials for lgpm-installed dev modules; check icons after a catalog install before calling it a bug.
- 2026-09-27 (checked 18:00): **testnet 0.3 isn't live.** `testnet.lez.logos.co` runs 0.2.x (head block fresh). LEZ `v0.3.0-rc1` (2026-09-26) is exactly our pin `f7fda38`, with no commits since. `devnet.lez.logos.co` is 0.2.5-rc3 and halted at block 604 since 2026-09-15 12:52 UTC (the old fingerprint called it "0.3"; fixed to decode the head block). status-im/infra-logos has no 0.3 deploy yet; the 30 Sep date is still only the 25 Sep X post. Release waits.
- 2026-09-27: Local faucet rate limit is 60 s per account; a wallet's first-run funding counts, so an app's first claim right after onboarding is rate-limited (correct).
- 2026-09-27: Testimonials must name "Logos Kit" (the prize counts texts that identify this wallet). Starter phrases fill the rest.
- Gotchas: `short` is reserved in QML JS; `\u` escapes typed into a file can arrive as the real characters (U+2028 breaks a QML line), so keep regex escapes ASCII; relative `source:` strings in a component resolve against the *using* file, so `LogosKitUi` uses `Qt.resolvedUrl`; the SDK's `kit.api` is null until `LogosKit` completes (`onApiChanged`); private accounts need an explicit tick in the connect sheet.

---

## After S8 · Full review, redesign, first public release (2026-09-29/30), `main`

- [x] Codex full review, prompt + result in `docs/reviews/full-1/` (verdict: real preview release; prize gates were canonical 0.3, CI, evaluator README/demo, adoption evidence)
- [x] **UI redesign** (merged from `ui/revamp`, `48a106d`): `sdk/qml/LogosKitUi` v2, ~30 QML components each ported from a named 21st.dev source (`docs/design/revamp-picks.md`, brief `docs/design/revamp.md`); wallet, both apps and the template rebuilt on it; approval sheet leads with asset + amount + full destination; focusable controls, contrast ≥ 4.5:1, reduced motion, integer LEZ units. Proof: harness flows (testimonial, faucet, template) and **real Basecamp** `e2e/basecamp-apps.sh` green on the 0.1.3/0.1.1 bundles, 0 sandbox "Blocked" lines. Test fixes: `tests/bc-lib.mjs` scrolls items into view before clicking; exact "Local" network pick; waits for onboarding funds or uses the in-flow faucet
- [x] **Docs + README rewrite** (40+ pages, Getting started / Concepts / Guides / SDK / Wallet / Reference / Help; landing from 21st.dev components; `pnpm shots` pipeline); every claim checked against code; deployed to https://logos-kit-docs.vercel.app
- [x] **CI** on `main` (green): `rust.yml`, `ts.yml` (incl. capability-matrix check, docs build), `e2e.yml` (standalone sequencer, dev proofs), `nix.yml` (manual/weekly), `changesets.yml` (npm), `daily.yml` (fingerprints both networks, opens the cutover issue on 0.3, evidence export artifacts). `scripts/lez-vendor.sh` makes clean clones build
- [x] **Evaluator contract** `e2e/demo.sh`: `--local` 16/16 (dev proofs), `--preview` 14/14 on the live network with real proofs (faucet, public, shield, private→public, token create/public/private, testimonial, evidence, 3 refusals)
- [x] **Drip faucet v2** live: durable ledger (restart-safe, never pays a key twice), per-client IP limits behind Traefik, 200/h global budget, `Retry-After`; sequencer key on tmpfs, deleted after start
- [x] **Evidence exporter**: prior activity from blocks before the post (`priorTxs`, `hasPriorActivity`, qualified counts)
- [x] Engine: public token send and token create settle as `success` from balances; approval sheet drops a single transfer's duplicate line
- [x] **Catalog release**: `logos_kit_wallet` + `_ui` 0.1.3, `logos_kit_testimonial` + `logos_kit_faucet` 0.1.1 (catalog `ef56f77`, all three platforms)
- [x] **npm**: `@logos-kit/protocol`, `codec`, `client`, `theme` 0.1.0 with READMEs/licenses (`1abe336`); publint + attw clean; installs and runs from npm (client read block 12,655 off the preview network). npm held `@logos-kit/codec@0.1.0` in its staged-release review for a few hours (latest pointed at a `0.0.0-stage` placeholder); it cleared the same day. Future releases: `changesets.yml` with trusted publishing (setup in `docs/dev/releasing.md`)

### Open
- [x] Clean catalog installs of 0.1.3 (2026-09-30): `e2e/catalog-install.sh` on macOS arm64, `--docker` Linux arm64 and (agari-box) Linux x86_64 install wallet 0.1.3 + apps 0.1.1, each signed by the release key; `e2e/catalog-install-gui.sh` installs from the catalog in real Basecamp and the wallet opens on "Preview network · LEZ 0.3" (`docs/reviews/s7/catalog/`). The GUI test now retries screenshots and waits up to 120 s for the first launch
- [x] Performance, first launch (measured 2026-09-30): the wallet's first open in the Basecamp GUI took 16–40 s (over 2 min once, on a flaky connection). Not ours: on a fresh catalog install `logosctl module load logos_kit_wallet` takes 1.1 s, and the engine library loads + inits in ~0.01 s warm (1.9 s the first time macOS scans a new 100 MB binary). In the GUI the gap sits between "Loading core dependencies" and "Module loaded", where Basecamp calls its `package_downloader` (dependency resolution over the network). Worth an upstream note to Basecamp; nothing to cut on our side beyond the binary size
- [x] npm trusted publishing configured for all four packages (`changesets.yml`, publish + stage publish; needed npm 11.19+ and `--allow-publish`)
- [ ] GitHub org setting "Allow GitHub Actions to create and approve pull requests" (maintainer), so `changesets.yml` can open the version PR
- [x] Extra CI (merged from `ci/extra`): `valid-proof.yml` nightly real-proof E2E (a shield proved on the runner's CPU and verified by a real-proof sequencer; first run 61 min, green), `guest-repro.yml` (testimonial program image id reproducible from the recorded commit; on program changes + weekly, green), `qml-gate.yml` (QML bundle up to date + engine gate; on SDK changes, green)
- [ ] Official testnet 0.3 cutover (blocked; `docs/dev/cutover-0.3.md`)
- [ ] Domain (sslip.io had a multi-minute DNS outage during a demo run)
- [ ] Adoption: drafts in `adoption/drafts/` await the maintainer's approval; 10 independent developers; testimonials on the official network
- [ ] Narrated demo video; 2–3 non-expert usability sessions
- [ ] Security follow-ups from S2 (vault write counter, zeroize LEZ `Storage`, Windows ACL); Logos Storage encrypted backup and performance budgets (S9)
- [ ] Linux GUI Basecamp run; conformance Basecamp mode on Linux; run the "other Logos modules" pattern in Basecamp
- [x] Docs polish: the live network pill shows on the desktop landing (hero content was centred in a fixed-height box and pushed under the nav). The site stays dark by default on purpose (Tray dark, like Basecamp); the toggle switches to light


## Official testnet 0.3 cutover (2026-10-06), PR #3 → `main` (`97f1f6c`)
- [x] §0 fingerprint `https://testnet.lez.logos.co`: version 0.3, head decodes with our pin, not halted, `getFeeState` answers (height ~11.8k; chain reset 2026-10-01)
- [x] §1 re-pin `f7fda38` → `db66590` (v0.3.0): 7 patches unchanged, vectors identical; `ApplyOutput.chained_calls` forced a testimonial rebuild
- [x] §4 testimonial `5YoH3xjhgeKt2mcJXW7c31bqDNCWWA4CRJxVdvzFvVef` (image `61b22436…`, immutable, `verified_local`); 13 builtins reproduced from v0.3.0 (`registry/builtins.json`)
- [x] §5 drip `https://lez-testnet-drip.84.46.247.92.sslip.io` + CORS relay `https://lez-testnet.84.46.247.92.sslip.io` (`deploy/testnet`, Coolify `oly3k82m9ngfw4twsacpz8wu`); treasury funded from the testnet's public genesis accounts (secrets `~/.config/logos/testnet/`, age backup)
- [x] §6 testnet is the default (engine, CLI, SDK); §8 docs; units shown as LGO (1 LGO = 10^9 lepta)
- [x] §7 catalog 0.2.0 (wallet core + UI, testimonial, faucet): pointer moved (`logos-kit-modules` `13196be`)
- [x] §9 every route on the official testnet with real proofs (`LK_ZONE=lez-testnet e2e/preview-flows.sh`, fresh wallet, drip-funded): public send (block 11969), shield (11978), private→public (11987), token create (11991), token public send (11994), token private send (12002), **testimonial (12004)**
- Network note: the testnet stalled for ~20 min (block 11960) during the run; transactions waited minutes for inclusion. The engine now waits ~6 min before "not seen"; the drip and flows treat `outcome_unknown` as pending.
- [ ] §10 daily evidence snapshot workflow; launch posts (`adoption/drafts/05-testnet-launch.md`) await the maintainer's go-ahead

## Combined plan (`docs/dev/PLAN-LP0001.md`): LP-0021 + LP-0001 + Market, one launch

### Stage A · Quiet release 0.3.0 (2026-10-06), done, no posts
- [x] PR #4 (Ledger design, fee display, approval fixes, 0.3.0 metadata) merged (`eb598ea`); plan, token UX research and `.gitignore` for AI workspace folders on main (`cf15ae3`)
- [x] Catalog gate hardened (`logos-kit-modules` `2950966`): a release missing a platform (`missingVariants` in `sidecar.json`) is rebuilt with `force_build` instead of counting as done, and a failed run dispatches one retry. 0.2.0 had shipped core on two platforms only (the linux-amd64 leg couldn't download the Qt source) and skipped the UI and apps
- [x] Catalog 0.3.0: `logos_kit_wallet`, `_ui`, `logos_kit_testimonial`, `logos_kit_faucet`, each on darwin-arm64, linux-amd64 and linux-arm64 (`Release on merge` run 37441509837)
- [x] Clean installs of 0.3.0, every package signed by the release key, core loads and answers: `EXPECT_VERSION=0.3.0 e2e/catalog-install.sh` on macOS arm64, `--docker` on Linux aarch64 (this Mac) and Linux x86_64 (agari-box); real Basecamp `e2e/catalog-install-gui.sh`: repository added, Logos Kit Wallet v0.3.0 + core installed from Applications, the wallet opens on the testnet (`docs/reviews/a/catalog/`)
- [x] Docs redeployed to production (`logos-kit-docs.vercel.app`), testnet default, LGO units
- `e2e/catalog-install.sh` now checks the installed version (`EXPECT_VERSION`); the GUI test takes `LK_SHOTS`

### Stage W · Wallet polish, README, `demo.sh --testnet` (2026-10-06), PR #6
- [x] First run lands on the public account; dev networks behind Advanced; the legacy preview network hidden
- [x] Activity persisted per network in an encrypted history vault; incoming payments found by reading blocks (no indexer on the testnet); faucet payments shown once and labelled; proofs that stopped when the wallet closed say so ("Nothing was sent")
- [x] Recipient checks (a token ID pasted as an address, lookalikes of addresses you paid, first-time addresses); readable errors, including LEZ's insufficient-funds error; MAX leaves the fee cap
- [x] Account names, hidden system accounts, explorer links, a private-account explainer, Settings (network, password, backup), backup and restore, cold start, programs you named
- [x] Low-memory proving (LEZ patch 0008, segment po2 18): about half the memory (5.5 GB peak instead of 9.9 GB), slower; the wallet refuses to start a proof below the memory floor and says why. `--low-memory` / `LOGOS_KIT_LOW_MEMORY=1` in the CLI. Budgets measured in `docs/dev/perf.md`
- [x] README on the RainbowKit/wagmi pattern with screenshots from the real wallet (`qa_readme.py`); internals moved to CONTRIBUTING, CHANGELOG and package READMEs
- [x] `e2e/demo.sh --testnet`: real proofs on the official testnet, drip-funded, explorer links, every route (public, shield, private → public, private → private, token create/send, token from private, testimonial). Spends only once the drop shows in the balance; retries once when a request never reached the node
- [x] Faucet client: a drip that answers after the client's 180 s cutoff is "outcome unknown" (it may still pay), not an internal error
- [x] **Evaluator demo on the official testnet, real proofs** (`LOGOS_KIT_LOW_MEMORY=1 e2e/demo.sh --testnet`, `docs/reviews/demo/testnet-20261006T1833Z.log`): 15/16 in 6032 s. Faucet, public send (block 13232), shield (13254), private → public (13297), private → private (13312), token create, token public send (13331), token into a private account (13346), token from private (13360), testimonial (13362), and the three refusals. The evidence export timed out: the node couldn't serve blocks 11901..=12000 (private transactions carry ~225 KB proofs) in one request. Fixed in `4d1e55b` (a range that times out is read in halves) and re-run: 1 distinct author, the post at block 13362, prior activity found (`…-evidence-rerun.log`)
- Earlier testnet runs on 2026-10-06 failed for reasons outside the wallet, each log kept locally (`docs/reviews/demo/`, gitignored) and labelled: sequencer outage, a 20-minute stall, funds not visible behind a lagging replica, a DNS failure, a request timeout, the drip answering late. The drip's host was overloaded (load ~80 on 4 cores, partly from thousands of health-check processes left as zombies by other apps)

### Stage T · Tokens: tiers, spam, token accounts, sample token (2026-10-06), PR #8
- [x] Token accounts (LEZ 0.3 ATAs): every public token payment lands in the recipient's token account, created on arrival without their signature, so an account holds any number of tokens; balances add the account's own slot and its token account. `kit.sdk.associatedTokenAccount(owner, definition)` in the codec, checked against LEZ's own derivation (vector) and in both Qt engines
- [x] Incoming tokens found by reading blocks (`crates/chain-index`: a cursor with the block hash, chunks of 100, complete and hash-chained replies only, a lagging node means wait, a different block means reset); the scan starts at the wallet's creation time
- [x] Trust tiers: Hidden > Verified (the Logos Kit token list, `registry/tokens/`) > Added > Spam > Unknown. Spam rules on the name as people see it (NFKC, case, UTS-39 skeletons): links, airdrop bait, invisible characters, more than 32 characters, lookalikes of a listed token or of LGO/LOGOS. Unknown and spam tokens are folded under "Hidden · Unknown", never in totals, never load an image, stay nameless in Activity
- [x] Token screens: the home list, Unknown and Spam tabs, Manage tokens (show/hide, pin), Add a token by ID (preview, decimals, a hard warning for imitations), token details with a trust panel, Create a test token, token-aware Send (a recipient that may itself be a token account needs a confirmation). CLI parity: `logos-kit token list|add|remove|hide|unhide|pin|unpin|info|create`
- [x] `lez_getTokens` for apps (protocol, lidl, forwarder, fake wallet, client `getWalletTokens`, QML facade, capability matrix, reference docs, changeset): verified, added and unknown tokens of an account the app may read; spam and hidden never
- [x] No "Clear slot": LEZ can't release a token slot, and token accounts make it moot (D4 revised)
- [x] LKT, the sample token: created on the testnet (`HPhjnu…7HSs`, block 13192, held by the drip treasury), on the token list with its logo. The drip sends 100.00 LKT after each landed LGO drop (`LK_DRIP_TOKEN`), into the account's token account; a new wallet finds it by itself (`e2e/drip-token.sh`)
- [x] Review (correctness + security agents), all fixed in `6253ddb`: disguised spam names, unknown names in sent rows, recipients that are token accounts, private receipts matched to our own transfers once, the block cursor, stale state after a chain reset, history rewritten on idle syncs, name cache per network
- [x] Checks: `e2e/tokens.sh` (27 checks incl. a stranger's token is Unknown, bait is Spam, add by ID, hide, three tokens on one account), `e2e/drip-token.sh`, `qa_tokens.py` (15 screens, dark and light), engine unit tests, QML gate (34 checks, Node = Qt 6.9.2 = Qt 6.11.1)
