# Logos Kit: the build to testnet 0.4 (wallet + tokens + NFTs + private proofs + Market + explorer)

Written 2026-10-14 after three verified research passes (LEZ 0.3.1 port scope, the LEZ 0.4 actor model in PR #935, and a map of our own code), plus Discord, `lambda-prize`, the live testnet and explorers, and Refero. It supersedes the stage order in `docs/dev/PLAN-LP0001.md`. Product design §4–§5 there still holds except where §3 below changes it. Facts: `docs/dev/status-2026-10-14.md`.

## 1. Context: what changed and why this plan

**Verified changes:**
- **The prizes moved.** LP-0021 and LP-0001 now target testnet 0.4 (commit `ad608bf`, merged 2026-10-13). 0.4 is announced for the end of November; programs become actors that handle messages. LP-0021 still requires "own and transfer fungible token-program assets on LEZ, from both public and private accounts", using "the existing/reference implementation" of the token program. It points at `logos-blockchain/lez-programs`.
- **The live testnet is LEZ 0.3.1, and it was reset (~2026-10-13).** Our wallet can't decode its blocks: two header fields were removed (`producer`, `signature`).
- **0.3.1 deleted the token, ATA and AMM programs.** `getProgramIds` shows only the privacy circuit. `lez-programs` still pins LEZ 0.2.4. So **today's testnet can carry only native LGO (public and private). Tokens and NFTs can't run there at all.**
- **The new chain's genesis doesn't fund the old test keys, and our drip treasury is empty.** New LGO arrives through the bridge from the Logos blockchain.
- **The 0.4 model is PR #935** (open, 388 files, 107 commits behind `dev`). Its genesis still has actor ports of token, ATA and AMM. There's no `Plan::inspect`, private accounts have no identifiers, and incoming private assets arrive as pending messages the wallet claims with a proof. The AMM in #935 is a working template for an escrow market.
- **Our code carries over at the top.** The UI, approvals, policy, history, Basecamp shim, LWS-0 method names, QML kit, theme, apps and docs are chain-neutral. The chain-specific parts are 15 engine files, `chain-index`, the TS codec, client node actions, facade `transfer()` and the testimonial program.

**Decision (owner: me, per the user, 2026-10-14):**
- **Two lines.**
  - **Line 1:** port `main` to 0.3.1 with *native LGO only*, so the released wallet works on the official network again. It's cheap, and its reset detection and per-zone capabilities are needed for 0.4 anyway.
  - **Line 2:** build everything that needs tokens on an integration branch `next/lez-0.4`, pinned to PR #935: tokens, NFTs, gate, Market, Pass and testimonial-on-actors. Develop and prove it on a local standalone sequencer from that tree, then cut over when Logos launches testnet 0.4.
- **No hosted copy of the token programs on 0.3.1.** It would be thrown away at 0.4.

**"The revamp."** All the revamps the user asked for earlier are done: UI kit v2 from 21st.dev (`48a106d`), the Ledger design (PR #4), the README rebuild (`c81710c`) and the docs rewrite. The re-architecture still outstanding is this port to Logos' 0.4 model. React/RN/passkeys (PLAN.md S10–S16) stay after a separate discussion with the user, as agreed earlier.

**Standing rules:**
- one combined launch, with promotion only after the user's hands-on test;
- no deadlines;
- CLI parity;
- UX states are first-class;
- 21st.dev components, real logos, one Refero-grounded design (Ledger), light and dark;
- tests only for complex integrations;
- merge each sub-stage when green;
- **every claim verified before it's written**;
- no Discord or GitHub posts without the user's OK.

## 2. Stages

| # | Stage | Branch | Exit proof |
|---|---|---|---|
| **0** | Clean slate | `main` | Daily CI green; pins correct |
| **R** | Back on the live testnet (LEZ 0.3.1, native LGO) | `main` | Fresh catalog install on Basecamp 0.3.2 (macOS + Linux): fund → public and private send → testimonial → explorer opens |
| **X** | Activity, transaction detail, explorer | `main` | Harness walks light/dark for sent, received, pending, failed and private; CLI `history` and `tx show`; "Open LEZ Explorer" in real Basecamp |
| **F0** | 0.4 line bootstrap (PR #935 pin) | `next/lez-0.4` | Local standalone: public and private native sends; a pending private receipt claimed; restore finds vaults |
| **F1** | Spikes on actors | `next/lez-0.4` | `docs/dev/lez04-spikes.md` with VERIFIED results; gate, Market and token-program decisions recorded |
| **F2** | Testimonial on actors | `next/lez-0.4` | `e2e-testimonial` on standalone; guest-repro |
| **F3** | Tokens on actors (stage T re-done) | `next/lez-0.4` | `e2e/tokens.sh` on the actor model (stranger's token Unknown, add by ID, hide, three tokens on one account, private receive + claim) |
| **N2** | NFTs | `next/lez-0.4` | `e2e/nft.sh` + one real private→private NFT send; restore finds NFTs |
| **N3** | Capabilities, protocol 0.2.0, SDK | `main` for the chain-neutral parts, then `next` | `authz.rs` proof-only session tests; capability matrix |
| **N4** | Gate, proofs, verifiers | `next/lez-0.4` | Real-proof gate on standalone; off-chain and on-chain agree; security review before deploy |
| **P** | Pass | `next/lez-0.4` | A fresh install claims, makes private, proves |
| **N6** | Members' Wall + Collection Studio + template | `next/lez-0.4` | `e2e/wall-app.sh`, `e2e/studio-app.sh`, real Basecamp |
| **M** | Market v0 (AMM-pattern escrow) | `next/lez-0.4` | `e2e/market.sh` list → buy (LGO, private deferred) → cancel → racing buyers |
| **N8** | Docs, demo, benchmarks, evidence exporters | both | `demo.sh --local/--nft` from a clean clone, macOS + Linux |
| **C** | Cutover to official testnet 0.4 | `next` → `main` | `demo.sh --testnet --nft` on 0.4 with real proofs; programs immutable + registry |
| **L** | Combined launch | `main` | User sign-off → the user runs promotion |

**How the two branches work:**
- Every merge to `main` is merged into `next/lez-0.4` straight away.
- Sub-stages land on `next` as their own PRs, merged when green, never stacked.
- `next` re-pins LEZ whenever #935 moves or merges.
- At C there is one cutover PR, merged with a merge commit (the `cutover/0.3` precedent).

### Stage 0: clean slate
- Remove the preview-network check from `.github/workflows/daily.yml`; the network was retired and hidden in the wallet. Drop `PREVIEW` from the job env.
- Fix `docs/dev/pins.md`: Basecamp tag `0.3.0` is `bbe5da0`, not `2c20227`. Record 0.3.2 (`f2fae63`) as the test target.
- Status doc and memory: done.

### Stage R: native LGO on 0.3.1 (agent report: port list)
1. **Re-pin LEZ** to `v0.3.1` = `1a003bfb`: `crates/xtask/src/main.rs:22`, `crates/wallet-engine/src/lib.rs:32`, `flake.nix:22` + `flake.lock`. Circuits and rapidsnark are unchanged.
2. **Rebase the patches** in `vendor/lez-patches`:
   - 0001: `new_with_storage_backend` becomes sync, with no `statistics_path`.
   - 0002: context refresh (`self.poller()`).
   - 0004: submit via `sequencer_client.send_transaction`.
   - 0005: refresh the Cargo.toml and `cli/mod.rs` hunks.
   - 0003 and 0006–0008: unchanged.
   - Update the patches README.
3. **Engine compile fixes:**
   - `session.rs:212`: `sequencers` → `sequencer`.
   - `session.rs:833-843`: sync constructor, no statistics.
   - `helm_owned` → `client_owned` (`session.rs:1246`, `activity.rs:58`, `tx.rs:652`, `engine.rs:637`).
   - `engine.rs:1721-1724`: `poller_vec` + `multi_poll` → `poller` + `poll_tx`.
4. **Connect probe:** `Session::connect` calls `get_last_block_id` explicitly, because the v0.3.1 constructor never fails on network.
5. **Chain-reset detection:**
   - LEZ storage: `last_synced_block > tip`, or a changed block-1 hash, triggers a zone rebuild of private state.
   - chain-index: the cursor carries the genesis hash (`crates/chain-index/src/lib.rs:95-100`).
   - Copy: "This network was reset. Your keys are safe; balances on the old chain are gone."
6. **Per-zone capabilities.** The engine probes for the token and ATA programs (program header) and reports `zone.capabilities.tokens`.
   - Where there's no token program: the token and NFT UI shows "This network doesn't have a token program yet", the CLI `token …` commands say the same, and the token decoders and builtins entries are gated.
   - Remove the `token_core` / `associated_token_account_core` path deps from the 0.3.1 build (cfg-gate the code) on `main` only; `next` keeps them.
7. **TS client and codec:**
   - `packages/client/src/decode.ts:56,76-77`: drop `producer` and the 64-byte signature; re-record `getBlock.json` and `getTransaction.json`; breaking changeset.
   - Rebuild the QML bundle.
   - `cargo xtask vectors` (only `lezRev` should change).
   - Gate the codec's token/ATA helpers per zone.
8. **xtask fingerprint:** label 0.3.1.
9. **`just verify-builtins`:** regenerate `registry/builtins.json` (10 builtins) and check it against the live headers (fee `0c9bad5d…`, clock `c2e82612…`, bridge `f56c08c7…`, sequencer_stake `e7ff6b67…`).
10. **Funding (needs the user):**
    - (a) The user posts, or OKs a post, asking the Logos team for testnet LGO for our drip treasury `FCRP…Y5sh` and our deployer.
    - (b) In parallel, a spike on the L1 path. The Logos blockchain devnet (`testnet.blockchain.logos.co`, with a `/web/faucet` route) gives L1 funds through faucet or EmPoWer, then a bridge `Deposit` moves them to LEZ (`bridge/core/src/lib.rs:22`). This becomes the Faucet app's second path: "Earn test LGO", which also fits lambda-prize PR #181 and LP-0022.
11. **Redeploy the testimonial program**, at the same address `5YoH…` if the old header key works. Keep the `lee_core` rev at `db66590` so the image stays `61b22436…`. Update `registry/programs.json`, `packages/codec/src/programs.ts:94` and the docs.
12. **Drip v2:** redeploy once funded (LGO only; v3's LKT path waits for F3).
13. **Basecamp 0.3.2:**
    - Re-test catalog installs (`e2e/catalog-install.sh`, `--docker`, `catalog-install-gui.sh`) and re-measure the first open (`package_downloader` is now lazy).
    - Confirm that `basecamp://intent/lez.wallet.open` reaches the wallet.
14. **Rehearse:** `e2e/standalone.sh`, `demo.sh --local`, one `--real-proofs` run, then `demo.sh --testnet` with native-only steps; commit the logs.
15. **Quiet release** of the catalog build (wallet core + UI + apps), with no posts; confirm with the user right before it, as in stage A.
16. **Docs:** `pins.md`, `networks.mdx`, the `deploy/testnet` header, the cutover runbook, README network notes.

### Stage X: activity, transaction detail, explorer (`main`)
**References (Refero, viewed):**
- Phantom "Sent" `e250c084`: token icon + direction badge, signed amount, rows (Date, Status, To, Network, Network fee), bottom "View on Etherscan".
- Family "Received" `680c3a6e`: "Received from …" header, status pill with time, General rows.
- Glow failed `ac2866f9`: "This transaction didn't go through" + fee-still-paid copy, "View Raw Transaction ↗".
- Family history `bf1a4ad7`: date-grouped list.

**Engine:**
- `TxStatus` (`engine.rs:89-139`) gains:
  - `kind`: send, receive, shield, unshield, private-send, token-send, token-receive, faucet, testimonial, connect, later nft and market. This replaces the title-text guessing in `TxRow.qml:19-23`.
  - `submittedAtMs`, plus `blockTimeMs` taken from the block header via `chain-index` on inclusion (today only incoming rows have a time, `engine.rs:725`).
  - `feePaid` for token sends: the fee payer's LGO balance change, the same method as `engine.rs:1811`.
- `Prefs.explorers: HashMap<zone, url>`, copying `Prefs.faucets` (`service.rs:82-83`).
  - `ui_setExplorer`; `explorer_base` (`service.rs:2606`) reads prefs, then the built-in map.
  - `open_url` (`service.rs:2614-2630`) also allows `http://` on loopback, for local explorers.
- `lez_getCapabilities` gains `explorer: bool`. The wallet still builds every URL. The faucet and testimonial apps drop their hardcoded `lez:testnet` checks.
- Installed check for the Basecamp app `lez_explorer_ui`, using the plugins-dir lookup in `app_info` (`service.rs:2733-2755`). It reports `state.explorerApp`.
- Routes are already correct and checked live: `/transaction/<hex>`, `/account/<base58>`, `/block/<id>`.

**UI** (`modules/logos_kit_wallet_ui/qml/LogosKitWallet/`):
- **New `TxDetail.qml`**, opened from every activity row (Home, token detail at `TokenDetail.qml:168`, the new Activity screen):
  - hero: token logo + direction badge, signed amount, status pill (Pending / Succeeded / Failed + time);
  - rows: Date, From, To, Network, Privacy/route, Fee, Block, App, Transaction (full hash, copy);
  - primary button "View on explorer";
  - secondary "Open in LEZ Explorer" when `state.explorerApp`, through `basecamp.apps.launch {app: "lez_explorer_ui"}`;
  - failed state with Glow copy; private sends say what the explorer can and can't show.
  - `ProofView` keeps the live proving progress and hands over to `TxDetail` once settled.
- **New `ActivityView.qml`:** "See all" from Home (`Home.qml:48-56` shows 12), date-grouped, filter by account and token.
- **Toasts** (`ToastHost` already supports `action`): settled sends get "View"; incoming payments toast too.
- **Settings:** Explorer row per network (show / edit / reset).
- **`metadata.json`** of the wallet UI: `"uses": [{"intent":"basecamp.apps.launch"}]`, plus a `Store.qml` `launchApp()` helper.
- **CLI parity:** `logos-kit history [--account] [--token] [--limit] [--json]` and `logos-kit tx show <hash|handle> [--json]`, both printing the explorer URL.
- **Upstream (only with the user's OK):** propose a `provides: lez.explorer.open {tx|account|block}` intent on `logos-blockchain/lez-explorer-ui`, so we can open the app *at* a transaction.

### Stage F0: the 0.4 line (`next/lez-0.4`)
- Vendor LEZ at the PR #935 head (`93e6cbf`) in `vendor/lez` via `scripts/lez-vendor.sh`, with the rev in `pins.md`. Port the patches:
  - 0001 storage backend, 0002 sync observer, 0005 keycard, 0007, 0008;
  - 0003/0004 become a prepare / prove / submit split around `send_tx` / `send_proven` (`935:lez/wallet/src/lib.rs:914-977,1126-1269`).
- **Engine port:**
  - drop `Identifier`, `AccDecodeData` and `PreparedPrivateTx` (`tx.rs:23,31-34,475,876-879,976`);
  - build transactions as root message + declared actors + admission evidence;
  - fetch the public actor states the simulation needs.
- **Decoders and chain-index:** the effect decoder reads the root message plus the predicted boundary; `chain-index` indexes `public_context.actors` and publications (`getPublications`).
- **Vaults:** one HD key node per private holding, replacing the PRF identifiers (§4.2). Restore uses `restore_keys(depth)`.
- **Incoming → Claim** (new, wallet-wide):
  - scan publications (`owned_pending_messages`);
  - show "Incoming · N" with the sender as plain text and the body before claiming;
  - claim = one receipt proof (5–8 min class), batched one at a time with a queue;
  - receipts unauthorized by default;
  - CLI `pending list|claim`.
- **Recovery bindings:** the first send to a new private address is proven and creates the binding; later ones reuse it. Explained on the review screen.
- **Public recipients that have never been used** need admission evidence (their public key). Decide the receive format in F1(h).
- `e2e/standalone.sh` builds the #935 standalone sequencer (`--features standalone`).

### Stage F1: spikes on actors (each result VERIFIED in `docs/dev/lez04-spikes.md`)
- (a) **Gate PoC:** a private self-transfer of an `NftPrintedCopy` with `notify → gate`; the gate checks `from.program == token`, `descriptor == {C, NftPrintedCopy}`, `amount == 1`. Measure time and RAM.
- (b) **Off-chain verify:** `Receipt::verify` + boundary-step parse; the verifier's root source.
- (c) **Market:** escrow PDA + `M.Buy` root paying native LGO, private deferred buy (Clear publication claim), racing buyers, squatted-cell reset.
- (d) **Pass:** print to an ATA → make private.
- (e) **Media:** images in QML sandboxes on Basecamp 0.3.2, and `storage_module` (bundled in 0.3.2) upload and download.
- (f) **Where proofs run:** `logos_host` subprocess vs the shell, and what an OOM kills. This sets the copy and decides whether "proving in a child process" is still needed.
- (g) **Token program for 0.4:** the #935 builtin vs a `lez-programs` actor port. Token program ID becomes per-zone config. Watch `lez-programs` and #935.
- (h) **Address formats:** what a public receive code must carry (public key for admission); per-destination private receive codes.

### F2–M on the actor model (design changes against PLAN-LP0001 §4)
- **F2 testimonial:**
  - `actor_entrypoint!` with root actor `(author, testimonial)`;
  - checks `from.is_none()` and `is_authorized`;
  - stats actor appends to live state; pages kept;
  - `timestamp_window` for the time check.
- **F3 tokens:** sends become `Transfer{to, descriptor, amount, notify}` with an ATA default. Trust tiers, spam rules, the token list and logos are unchanged. A private receipt shows under Incoming → Claim. LKT and drip v3 return here.
- **N2 NFTs:** the §4.3 UX as designed (Collectibles, collection, detail, missing media, Send / Make private / Prove / Sell). A private vault = one key node. Incoming NFTs to private codes need a claim.
- **N4 gate:** the F1(a) design.
  - It replaces the `Plan::inspect` + nullifier-keyed PDA design.
  - Stale/refresh is inherent, since every proof updates the vault.
  - Transport is unchanged: handle → status → parts.
  - No membership oracle; separate approval per proof.
- **M Market:** the AMM pattern (`935:lez/programs/amm/src/swap.rs:36-80`) with NFT-escrow and pay-vault PDAs. v0 order: native-LGO buy, then private deferred buy, then same-transaction private buy. Its own security review.
- **N3 protocol 0.2.0:**
  - `getNfts`, `getCollections`, `transferNft`, `requestMembershipProof`, `getProofStatus/Bundle`, `verifyMembershipProof`;
  - new `pendingIncoming` + `claim` intent;
  - raw `Instruction` gains an actor/message form.
  - Session, grants and high-level intents stay stable for third-party apps.

### Stage C: cutover to official testnet 0.4
- Re-pin to the 0.4 release tag.
- Deploy the testimonial, gate and Market programs (staging, then immutable), with registry entries; create the Pass collection and LKT; fund the drip.
- `demo.sh --testnet --nft` with real proofs; logs committed.
- PR `next/lez-0.4` → `main` (merge commit); catalog release; docs; criteria audit refreshed for LP-0021 and LP-0001.

### Stage L: combined launch (unchanged from PLAN-LP0001 L)
Codex full review against both prizes, the user's hands-on test and fix round, the narrated video, `solutions/` drafts, and launch-post drafts. The user runs promotion.

## 3. Verification
- **Each stage:** its exit proof above; CI green on both branches (rust, ts + capability matrix, e2e, qml-gate, guest-repro, nightly real proof); harness walks offscreen in light and dark; real Basecamp 0.3.2 flows.
- **R, on the official testnet:**
  - `cargo xtask fingerprint` shows `headDecodesAsPinned: true`;
  - a fresh catalog install sends public and private LGO;
  - the explorer opens the transaction.
- **X:** screenshots of `TxDetail` in five states, both themes; `logos-kit tx show` prints the same URL the GUI opens; the LEZ Explorer app launches from the wallet in Basecamp.
- **F line:** every spike recorded VERIFIED; `e2e` on the #935 standalone sequencer green in CI on `next/lez-0.4`.
- **The user's hands-on checks** after R+X, F3, N2, N4, M and L.

## 4. Risks
| Risk | Answer |
|---|---|
| #935 changes before 0.4 (rebase onto `dev` drops token/ATA/AMM) | Pin a SHA; token program ID per zone; F1(g) tracks `lez-programs`; re-pin on every #935 move |
| 0.4 slips | Nothing on the prize path waits on a date; the R line keeps the live product working |
| No LGO on the 0.3.1 chain | The team request (with the user's OK) + the L1 → bridge spike |
| Claim proofs make receiving slow | Queue + clear ETA + batch UI; public sends to ATAs stay instant |
| Gate PoC fails | Design B (`env::verify` membership guest), or a forked token program with an `AssertHolding` message |
