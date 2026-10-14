# Logos Kit: the build to testnet 0.4 (wallet + tokens + NFTs + private proofs + Market + explorer)

**v2, 2026-10-14.** v1 was written after three verified research passes: the LEZ 0.3.1 port scope, the LEZ 0.4 actor model in PR #935, and a map of our code, plus Discord, `lambda-prize`, the live testnet and explorers, and Refero. v2 folds in two independent Codex passes: `docs/reviews/plan-0.4/codex-research.md` and `codex-review.md`.

It supersedes the stage order in `docs/dev/PLAN-LP0001.md`. Product design §4–§5 there still holds except where this plan changes it. Facts: `docs/dev/status-2026-10-14.md`.

## 1. Context

**Verified:**
- **The prizes moved.** LP-0021 and LP-0001 target testnet 0.4 (`lambda-prize` `ad608bf`, merged 2026-10-13). LP-0021 still requires fungible tokens, public and private, from "the existing/reference implementation" of the token program, plus the faucet mini-app while lambda-prize#181 is open.
- **The live testnet is LEZ 0.3.1, reset ~2026-10-13.**
  - Our wallet can't decode its blocks (the header lost `producer`/`signature`).
  - **0.3.1 removed the token, ATA and AMM programs**, so today's chain carries only native LGO.
  - Its v0.3.1 genesis accounts `6iAr…2VQV` (10k LGO) and `7wHg…3YQo` (20k LGO) are funded, with keys published in the LEZ source.
  - The maintainers' intended funding route is EmPoWer on the Logos blockchain, then bridge in.
- **The 0.4 model is draft PR #935** (open, head `93e6cbf`, 388 files).
  - Actors handle messages. There's no `Plan::inspect` and there are no private identifiers.
  - Incoming private assets arrive as pending messages claimed with a proof.
  - Its genesis has actor token/ATA/AMM, but its author says ATA is removed on `dev`, so the 0.4 token program and its IDs are **unknown**.
- **Draft PR #949 (private fees) is a risk for every private flow:** the proof must land within 20 blocks (~200 s) of its fee quote, and our real proofs take 267–757 s.
- **Our code carries over at the top.** UI, approvals, policy, history, shim, LWS-0 names, QML kit, theme, apps and docs. The chain-specific parts are 15 engine files, `chain-index`, the TS codec, client node actions, facade `transfer()` and the testimonial program.

**Decision (mine, per the user; Codex agreed, `codex-review.md` "Strategy alternatives"):**
- **Line 1, `main`:** a *bounded* repair to 0.3.1 with native LGO, so the released wallet works on the official network. The work includes reset detection and per-network capabilities, which 0.4 needs too.
- **Line 2, `next/lez-0.4`:** everything token-based (tokens, NFTs, gate, Pass, Wall/Studio, Market, testimonial on actors), proven on a local standalone sequencer from PR #935, then cut over when 0.4 launches.
- **No hosted token programs on 0.3.1.**
- **Protocol risks first.** The F1 spikes (#949 fees, gate freshness, actor approvals, private destinations, Market atomicity) and the chain-neutral SDK contract (N3a) **run in parallel with R and X**, not after them.

**Standing rules:**
- one combined launch, with promotion only after the user's hands-on test;
- no deadlines;
- CLI parity;
- UX states are first-class;
- network-missing features degrade gracefully and switch on by themselves;
- 21st.dev components, real logos, one Refero-grounded design (Ledger), light and dark;
- tests only for complex integrations (the adversarial protocol tests below count);
- merge each sub-stage when green;
- **every claim verified before it's written**;
- Discord questions may be asked from the user's account in a plain style; other public posts need the user's OK.

## 2. Stages

| # | Stage | Branch | Exit proof |
|---|---|---|---|
| **0** | Clean slate | `main` | PR #10 merged; Daily green; pins correct |
| **R** | Back on the live testnet (LEZ 0.3.1, native LGO) | `main` | Fresh catalog install on Basecamp 0.3.2 (macOS + Linux): fund → public and private send → testimonial → explorer opens; a pre-reset install detects the reset and recovers |
| **X** | Activity, transaction states, explorer | `main` | Harness walks light/dark for all states in §X; CLI `history` and `tx show`; "Open LEZ Explorer" in real Basecamp |
| **N3a** | SDK contract, chain-neutral (sessions, grants, proof-only session, capability discovery, NFT/proof/pending schemas) | `main` | Conformance fake scenarios; `authz.rs` proof-only tests; published examples outside developers can build on |
| **F0** | 0.4 line bootstrap (PR #935 pin) | `next/lez-0.4` | Local standalone: public and private native sends; a pending private receipt claimed; restore finds vaults |
| **F1** | Protocol spikes on actors (ordered, §F1) | `next/lez-0.4` | `docs/dev/lez04-spikes.md`, each result VERIFIED with a test; decisions recorded for fees, gate issuance, destinations, approvals and the Market |
| **F2** | Testimonial on actors | `next/lez-0.4` | `e2e-testimonial` on standalone incl. forged nested/async negatives; guest-repro |
| **F3** | Tokens on actors (stage T re-done) | `next/lez-0.4` | `e2e/tokens.sh` on actors + capability switch-on/off while installed; gated on the final token program's source and ID |
| **N2** | NFTs | `next/lez-0.4` | `e2e/nft.sh` + a real private→private NFT send; restore finds NFTs; wrong-descriptor and occupied-destination cases |
| **N3b** | Actor approvals + NFT/proof SDK | `next/lez-0.4` | Approval acceptance tests (§N3b); capability matrix |
| **N4** | Gate, proofs, verifiers | `next/lez-0.4` | The F1 freshness matrix passing on real proofs; off-chain = on-chain verdicts; security review before deploy |
| **P** | Pass | `next/lez-0.4` | A fresh install claims, makes private, proves |
| **N6** | Members' Wall + Collection Studio + template | `next/lez-0.4` | `e2e/wall-app.sh`, `e2e/studio-app.sh`, real Basecamp |
| **M** | Market v0 | `next/lez-0.4` | The F1(e) adversarial suite + `e2e/market.sh`; security review |
| **N8** | Docs, demo, benchmarks, evidence exporters | both | `demo.sh --local/--nft` from a clean clone, macOS + Linux |
| **C** | Cutover to official testnet 0.4 | `next` → `main` | `demo.sh --testnet --nft` on 0.4 with real proofs; programs immutable + registry; CI green on the **default branch** |
| **L** | Combined launch | `main` | User sign-off → the user runs promotion |
| **Q** | Qualification (post-launch adoption) | — | Criterion ledger complete with evidence for every adoption row |

**How the two branches work:**
- Every merge to `main` is merged into `next/lez-0.4` straight away.
- Sub-stages land on `next` as their own PRs, merged when green, never stacked.
- `next` re-pins LEZ only after a compatibility test whenever #935 moves or merges.
- At C there is one cutover PR, merged with a merge commit (the `cutover/0.3` precedent).

**Criterion ledger** (new, `docs/dev/criteria-ledger.md`, from N3a on): one row per LP-0021 and LP-0001 success criterion. Each row has the stage, its exit proof, an evidence link, and for adoption rows the count, month and independence. It replaces the stale `criteria-audit.md`. The coverage table in `codex-review.md` is its seed.

### Stage 0: clean slate
- Daily without the preview-network check, the Basecamp tag corrections (0.3.0 = `bbe5da0`, 0.3.2 = `f2fae63`), the status file, this plan and the Codex reports: all in PR #10. Merge when green.

### Stage R: native LGO on 0.3.1 (bounded)
1. **Re-pin LEZ** to `v0.3.1` = `1a003bfb`: `crates/xtask/src/main.rs:22`, `crates/wallet-engine/src/lib.rs:32`, `flake.nix:22` + `flake.lock`. Circuits and rapidsnark are unchanged.
2. **Rebase the patches.**
   - 0001: `new_with_storage_backend` becomes sync, with no `statistics_path`.
   - 0002: context refresh (`self.poller()`).
   - 0004: submit via `sequencer_client.send_transaction`.
   - 0005: refresh the Cargo.toml and `cli/mod.rs` hunks (apply after 0003).
   - 0003 and 0006–0008: unchanged.
   - Update the patches README.
3. **Engine compile fixes.**
   - `session.rs:212`: `sequencers` → `sequencer`.
   - `session.rs:833-843`: sync constructor, no statistics.
   - `helm_owned` → `client_owned` (`session.rs:1246`, `activity.rs:58`, `tx.rs:652`, `engine.rs:637`).
   - `engine.rs:1721-1724`: `poll_tx`.
4. **Connect probe.** `Session::connect` calls `get_last_block_id`, because the v0.3.1 constructor never fails on network.
5. **Chain-reset detection** (needed for 0.4 too).
   - LEZ storage: `last_synced_block > tip`, or a changed block-1 hash, triggers a private-state rebuild for that network.
   - `chain-index`: the cursor carries the genesis hash (`crates/chain-index/src/lib.rs:95-100`).
   - History rows are reconciled by hash.
   - Copy: "This network was reset. Your keys are safe; balances on the old chain are gone."
6. **Per-network capabilities** (needed for 0.4 too).
   - The engine probes program headers for token, ATA and NFT support and reports `zone.capabilities`. It re-probes on each connect, so features switch on or off **without a reinstall**.
   - Without a token program: token and NFT screens show "This network doesn't have a token program yet"; the CLI `token …` commands say the same; token decoders and builtins entries are gated.
   - The token crates are cfg-gated out of the 0.3.1 build on `main` only.
7. **TS client and codec.**
   - `packages/client/src/decode.ts:56,76-77`: drop `producer` and the 64-byte signature; re-record `getBlock.json` and `getTransaction.json`; breaking changeset.
   - QML bundle rebuild; `cargo xtask vectors` (only `lezRev` should change); token/ATA helpers gated per network.
8. **`xtask fingerprint`:** label 0.3.1.
9. **`just verify-builtins`:** 10 builtins, checked against the live headers (fee `0c9bad5d…`, clock `c2e82612…`, bridge `f56c08c7…`, sequencer_stake `e7ff6b67…`).
10. **Funding, in this order.**
    - (a) **Genesis accounts.** Move a working amount from the published genesis accounts `6iAr…2VQV` / `7wHg…3YQo` into **our own** deployer and drip-treasury keys (`~/.config/logos/testnet/`), as on 2026-10-06. The shared keys are never used as the treasury.
    - (b) **Bridge spike, in parallel.** L1 faucet or EmPoWer (`pow_start_mining`/`pow_claim` in the blockchain module, the Mining view in the blockchain app) → Channel Deposit to LEZ with metadata = Borsh `AccountId` (`lez/sequencer/core/src/lib.rs:173-176`), a tiny amount to a key we control. Record each step. It becomes the Faucet app's "Earn test LGO" path (fits lambda-prize#181 and LP-0022).
    - (c) Ask the Logos team only if (a) and (b) both fail.
11. **Redeploy the testimonial program** (same address `5YoH…` if its header key still works; keep `lee_core` at `db66590` so the image stays `61b22436…`). Update `registry/programs.json`, `packages/codec/src/programs.ts:94` and the docs.
12. **Drip v2** redeploy once funded (LGO only; v3's LKT path waits for F3). The **faucet mini-app stays a required deliverable** while lambda-prize#181 is open.
13. **Basecamp 0.3.2.** Re-test catalog installs (`e2e/catalog-install.sh`, `--docker`, `catalog-install-gui.sh`); re-measure the first open; check that `basecamp://intent/lez.wallet.open` reaches the wallet.
14. **Rehearse:** `e2e/standalone.sh`, `demo.sh --local`, one `--real-proofs` run, then `demo.sh --testnet` (native-only steps); commit the logs.
15. **Quiet release** of the catalog build, no posts; confirm with the user right before it, as in stage A.
16. **Docs:** `pins.md`, `networks.mdx`, the `deploy/testnet` header, the cutover runbook, README network notes.

### Stage X: activity, transaction states, explorer (`main`)
**References (Refero, viewed):** Phantom "Sent" `e250c084`, Family "Received" `680c3a6e`, Glow failed `ac2866f9`, Family date-grouped history `bf1a4ad7`. Support pages Codex cited: Phantom history, detail/explorer and pending/retry; Rainbow activity; Family activity and spam options.

**Transaction state model** (engine `TxStatus`, `engine.rs:89-139`). Each state has its own copy and actions:

| State | Meaning | Copy / actions |
|---|---|---|
| **Proof failed before submission** | out of memory, cancelled, expired fee quote, invalid | "Nothing was sent. No fee." Retry |
| **Submitted, outcome unknown** | sent; no inclusion seen yet | "Check before retrying"; the explorer link stays available |
| **Included, reverted** | in a block, failed | Fee as actually charged, from inclusion |
| **Included, succeeded** | in a block, done | Normal detail |
| **Private receipt pending claim** | 0.4 only | Claim actions (see F0) |

**Engine.**
- **New fields:** `kind` (send, receive, shield, unshield, private-send, token-send/receive, faucet, testimonial, connect/approval, later nft and market), `submittedAtMs`, `blockTimeMs` (block header via `chain-index`), and `feePaid` from inclusion (the fee payer's LGO change, not an estimate).
- **Reconciliation:** history reconciles by hash across restarts and resets.
- **Explorer per network:** `Prefs.explorers` (like `Prefs.faucets`, `service.rs:82-83`), `ui_setExplorer`, `explorer_base` reading prefs; loopback `http://` allowed for local explorers.
- **For apps:** `lez_getCapabilities.explorer: bool`; the faucet and testimonial apps drop their hardcoded `lez:testnet` checks.
- **Basecamp explorer app:** detect `lez_explorer_ui` via the plugins-dir lookup in `app_info` (`service.rs:2733-2755`).

**UI.**
- **`TxDetail.qml`** from every activity row (Home, token detail at `TokenDetail.qml:168`, the new Activity screen):
  - hero: logo + direction badge, signed amount, status pill with time;
  - rows: Date, From, To, Network, Privacy/route, Fee, Block, App, full hash with copy;
  - primary **"View on explorer"** (the web deep link, the reliable transaction-specific action);
  - secondary "Open LEZ Explorer" (by name only, when installed).
  - `ProofView` keeps live proving and hands over to `TxDetail` once settled.
- **`ActivityView.qml`:** date-grouped, filter by account and token, connect/approval events, hide spam rows (Family).
- **Toasts:** "View" on settled transactions; incoming payments toast too.
- **Settings:** explorer per network (show / edit / reset).
- **Wallet UI `metadata.json`:** `"uses": [{"intent":"basecamp.apps.launch"}]`, plus a `Store.qml` `launchApp()`.
- **Gas:** show estimated and used gas where the network exposes it (LP-0021 Usability).
- **CLI parity:** `logos-kit history [--account] [--token] [--limit] [--json]` and `logos-kit tx show <hash|handle> [--json]`, both printing the explorer URL.
- **Upstream (only with the user's OK):** an issue on `lez-explorer-ui` proposing `provides: lez.explorer.open {tx|account|block}`.

### Stage N3a: SDK contract, chain-neutral (`main`, starts with R)
- **Protocol 0.2.0 schemas, additive:**
  - capabilities, including `nfts`, `membershipProof`, `pendingIncoming` and `explorer`;
  - the proof-only session (`getSession` → `{accounts: [], grants: {request_proof}}`);
  - per-capability grants with `granted_at`/`last_used_at`, revoke one or all (affects live sessions);
  - NFT and proof method schemas: `getNfts`, `getCollections`, `transferNft`, `requestMembershipProof`, `getProofStatus/Bundle`, `verifyMembershipProof`;
  - a pending-incoming `claim` intent.
- **Conformance fake** (`modules/logos_kit_wallet_fake`) implements them first, so outside developers can build against stable shapes while the actor backend lands.
- **Negative authorization tests** for every read and proposal path (`authz.rs`): proof-only apps get `accounts: []`; balance and propose return 4100; a declined non-member looks identical to a declined member.
- **Published template and examples**, so independent builders (10 for LP-0021, 3 for LP-0001) can start real commit histories early. Promotion stays at L.

### Stage F0: the 0.4 line (`next/lez-0.4`)
- **Vendor LEZ** at #935 `93e6cbf` via `scripts/lez-vendor.sh`. Port the patches: 0001, 0002, 0005, 0007, 0008; 0003/0004 become a prepare / prove / submit split around `send_tx` / `send_proven`.
- **Engine.**
  - drop `Identifier`, `AccDecodeData` and `PreparedPrivateTx`;
  - build transactions as root message + declared actors + admission evidence;
  - fetch the public actor states the simulation needs.
- **Decoders and chain-index:** the root message + predicted boundary; index `public_context.actors` and publications.
- **Vaults:** one HD key node per private holding (no identifiers).
  - Measure the restore-depth limit.
  - Test that keys stay discoverable after many empty or rejected destinations.
- **Incoming → Claim.**
  - Durable states: detected, proving, submitted, claimed, recoverable failure.
  - Idempotent retry, queue position, time and memory estimate.
  - A recovery scan for old installs and for restarts or out-of-memory mid-proof.
  - Show the sender **only when authenticated data exists** (sealed publications carry none).
  - Receipts unauthorized by default; show the body before claiming.
  - Test a receipt that can't be claimed because its destination is occupied.
  - CLI `pending list|claim`.
- **Recovery bindings:** the first send to a new private address is proven and creates the binding; the review screen explains it.
- **`e2e/standalone.sh`** builds the #935 standalone sequencer (`--features standalone`).

### Stage F1: protocol spikes (ordered; each recorded VERIFIED with a test in `docs/dev/lez04-spikes.md`)
- **(a) Private fees and expiry (#949), first.**
  - Real proof: quote → prove → submit → expire → requote, measuring fee, retry, cancellation, low-memory mode and the 4-per-block cap.
  - **No claim or proof UX is promised until this passes.**
  - If the window can't fit laptop proving, record it, ask upstream (we asked on Discord 2026-10-14), and design around it: pre-quote at the end of proving if the protocol allows, a proving queue with requote, or a paired faster prover.
- **(b) Gate issuance and freshness** (LP-0001: "verify without a transaction", and "a new proof invalidates the old for that token").
  - Define exactly when a proof is *issued* and when vault state changes.
  - Prove that an off-chain verifier reads the authoritative latest state.
  - **Test matrix:**
    - proof A valid;
    - proof B issued for the same token without consuming A on-chain makes A stale;
    - B valid;
    - transfer and expiry;
    - master and printed NFTs;
    - two copies from one collection;
    - two contexts;
    - replay across contexts.
  - Keep the proof-only session and the shared on/off-chain verdict table (PLAN-LP0001 §4.4).
  - **Candidate design:** a private self-transfer of the `NftPrintedCopy` with `notify → gate`; the gate checks `from.program == token`, `descriptor == {C, NftPrintedCopy}`, `amount == 1`. Alternatives: design B (`env::verify`) or a token `AssertHolding` message, if upstream adds one.
- **(c) Off-chain verify:** `Receipt::verify` + the boundary-step parse, and the verifier's root source.
- **(d) Per-asset private destinations and bindings**, replacing the single reusable receive code and the assumed ATA default. Test wrong asset, occupied cell, duplicate delivery, recovery and refund.
- **(e) Market:**
  - listing custody of the unique item;
  - mutual exclusion of buy and cancel;
  - asset/payment atomicity, or explicit deferred-settlement states;
  - failed-buy fee disclosure, refund and recovery;
  - proof invalidation on list and sale;
  - racing buyers;
  - squatted-cell reset.
  - Adversarial tests written **before** stage M. The AMM pattern (`935:lez/programs/amm/src/swap.rs`) is the starting point, not the proof.
- **(f) Actor approvals:** turn declared actors and nested Sync/Async messages into a trusted effects summary; program source-verification status from our registry (LP-0023-style); estimated and used gas. Signing is **blocked** when effects can't be rendered, with an "Unknown effects" state.
- **(g) Pass:** print to a public holding → make private.
- **(h) Media:** images in QML sandboxes on Basecamp 0.3.2; `storage_module` upload and download at realistic sizes; network-off behaviour.
- **(i) Where proofs run:** the `logos_host` subprocess vs the shell, and what an out-of-memory kills.
- **(j) Token program for 0.4:** the #935 builtin vs a `lez-programs` actor port. Token program ID is per-network config. **F3 waits for the final reference program's source, ID and deployment path.**
- **(k) Address formats:** what a public receive code must carry (the public key, for admission evidence).

### F2–M on the actor model
- **F2 testimonial:**
  - `actor_entrypoint!`, root actor `(author, testimonial)`, checks `from.is_none()` + `is_authorized`;
  - the required fields (custom text, optional name, submission ID) are tested;
  - stats actor append, pages kept, `timestamp_window`;
  - **negative tests** for forged nested and async calls.
- **F3 tokens:**
  - `Transfer{to, descriptor, amount, notify}` with the F1(d) destination model;
  - trust tiers, spam rules, the token list and logos unchanged;
  - LKT and drip v3 return;
  - capability switch-on/off test.
- **N2 NFTs:** the PLAN-LP0001 §4.3 UX. A per-item identifier is defined by us, because #935's `NftPrintedCopy { owned }` has no serial. Counts cover public, private and pending separately. Chain-observer unlinkability test.
- **N3b actor approvals + SDK:**
  - the F1(f) summary in `ApprovalView`;
  - acceptance tests with a malicious program, an unknown program, an unlimited approval, a nested asset movement, an ambiguous simulation, and a proof-only grant;
  - single-item vs collection-wide NFT approval;
  - the proof prompt is separate from the account prompt (approve the proof, decline accounts in one request).
- **N4 gate:** the F1(b) design and matrix, transport by handle → status → parts, no membership oracle, a separate approval per proof, and a security review before the immutable deploy.
- **M Market:** the F1(e) suite first. v0 order: native-LGO buy, then private deferred buy, then same-transaction private buy. Its own security review.

### Stage C: cutover to official testnet 0.4
- Re-pin to the 0.4 release tag after a compatibility test.
- Deploy testimonial, gate and Market (staging, then immutable) with registry entries; create the Pass collection and LKT; fund the drip.
- `demo.sh --testnet --nft` with real proofs; logs committed.
- PR `next/lez-0.4` → `main` (merge commit); CI green on `main`; catalog release; docs; ledger refreshed.

### Stage L: combined launch
Codex full review against both prizes, the user's hands-on test and fix round, the narrated video, `solutions/` drafts, and launch-post drafts. The user runs promotion.

### Stage Q: qualification (post-launch)
- Tracked in the criterion ledger with evidence.
- **LP-0021:** 10 independent Basecamp apps with real commit histories; 150 distinct-account testimonials, ≥ 30 a month for 2 months; 30 Discord + 30 X vouches.
- **LP-0001:** 5 independent collections; 3 independent NFT/proof apps; 100 distinct-account gated actions, ≥ 20 a month; 20 + 20 vouches. The 50-testimonial baseline is waived only if we win LP-0021.
- Exporters and daily snapshots from N8. No manufactured activity.

## 3. Verification
- **Each stage:** its exit proof; CI green on both branches (rust, ts + capability matrix, e2e, qml-gate, guest-repro, nightly real proof); harness walks offscreen in light and dark; real Basecamp 0.3.2 flows.
- **R, on the official testnet:** `fingerprint` shows `headDecodesAsPinned: true`; a fresh catalog install sends public and private LGO; the explorer opens the transaction; a pre-reset install recovers.
- **X:** screenshots of every row of the state table in both themes; `logos-kit tx show` prints the URL the GUI opens; the LEZ Explorer app launches from the wallet in Basecamp.
- **F line:** every F1 item VERIFIED with a test before the stage that depends on it; e2e on the #935 standalone sequencer green in CI on `next/lez-0.4`.
- **The user's hands-on checks** after R+X, F3, N2, N4, M and L.

## 4. Risks
| Risk | Answer |
|---|---|
| #949 fee window shorter than laptop proving | F1(a) first; upstream question asked; designs: requote at proof end, a queue, a paired prover |
| #935 changes before 0.4 (rebase drops token/ATA/AMM) | Pin a SHA; per-network token program ID; F3 gated on the final program; re-pin only after compatibility tests |
| Gate freshness can't be verified off-chain | F1(b) decides between the self-transfer/notify design, design B, or an upstream `AssertHolding` |
| Assets stranded in private destinations | F1(d) per-asset destinations, refund and recovery tests |
| 0.4 slips | Nothing on the prize path waits on a date; R keeps the live product working |
| The shared genesis LGO gets drained | Take our working amount early in R(a); the bridge path R(b) as the durable route |
| Too few independent builders over time | N3a ships stable SDK shapes and the template early |
