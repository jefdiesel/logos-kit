# LEZ `wallet` crate patches (Logos Kit)

Small, upstreamable patch series against the LEZ `wallet` crate
(`logos-blockchain/logos-execution-zone` at tag `v0.3.1`,
`1a003bfbfb89fcf987f62975cd08a401169e1b8c`), needed so the Logos Kit wallet engine can embed
`wallet::WalletCore` in a Basecamp module and a CLI. Every patch keeps the existing CLI
behaviour and public API. The old entry points are now built from the new ones.

Apply:

```sh
git clone https://github.com/logos-blockchain/logos-execution-zone lez
cd lez && git checkout 1a003bfbfb89fcf987f62975cd08a401169e1b8c
git am /path/to/logos-kit/vendor/lez-patches/*.patch
```

| # | File | Touches | +/- |
|---|------|---------|-----|
| 1 | `0001-storage-backend.patch` | `storage.rs`, `lib.rs` | +107 / -34 |
| 2 | `0002-sync-observer.patch` | new `sync_observer.rs`, `lib.rs` | +106 / -20 |
| 3 | `0003-public-tx-prepare-sign.patch` | `account_manager.rs`, `lib.rs` | +163 / -62 |
| 4 | `0004-private-tx-prepare-prove-submit.patch` | `lib.rs` | +105 / -23 |
| 5 | `0005-keycard-feature.patch` | `Cargo.toml`, `account_manager.rs`, `cli/mod.rs` | +20 / -4 |
| 6 | `0006-storage-remove-label.patch` | `storage.rs` | +5 / -0 |
| 7 | `0007-private-note-no-stdout.patch` | `lib.rs` | +13 / -9 |
| 8 | `0008-low-memory-proving.patch` | `lee/state_machine/src/privacy_preserving_transaction/circuit/mod.rs` | +20 / -0 |

Most of the lines in patches 3 and 4 are existing function bodies moved into the new
functions. The series adds no new dependencies.

## 1. `StorageBackend`: pluggable storage persistence

**Why:** `Storage` always writes plaintext JSON, and `WalletCore` writes it directly, including
once per synced block. With this patch we can encrypt the wallet at rest and debounce those
writes in our own backend.

```rust
// wallet::storage
pub trait StorageBackend: Send + Sync {
    fn load(&self) -> Result<Option<Vec<u8>>>;   // None = nothing stored yet
    fn save(&self, bytes: &[u8]) -> Result<()>;
}
pub struct FileBackend { /* path */ }            // default: the old plaintext file
impl FileBackend { pub const fn new(path: PathBuf) -> Self; pub fn path(&self) -> &Path; }
impl StorageBackend for FileBackend { .. }

impl Storage {
    pub fn to_bytes(&self) -> Result<Vec<u8>>;
    pub fn from_bytes(bytes: &[u8]) -> Result<Self>;
    pub fn load_from(backend: &dyn StorageBackend) -> Result<Self>;
    pub fn save_to(&self, backend: &dyn StorageBackend) -> Result<()>;
    // from_path / save_to_path are unchanged; they now go through FileBackend
}

// wallet::WalletCore
pub fn new_with_storage_backend(
    config_path: PathBuf,
    storage_path: PathBuf,              // only used by storage_path() and in messages
    config_overrides: Option<WalletConfigOverrides>,
    storage: Storage,                   // Storage::load_from(&backend) or Storage::new(pw)
    storage_backend: Box<dyn StorageBackend>,
) -> Result<Self>;
pub fn set_storage_backend(&mut self, storage_backend: Box<dyn StorageBackend>);
```

All saves go through the configured backend, including `store_persistent_data` and the
per-block save during sync. The save cadence is unchanged. The on-disk format and the error
messages are also unchanged.

Since the v0.3.1 re-pin, `new_with_storage_backend` is a plain `fn` without
`statistics_path`, matching upstream's constructors: LEZ v0.3.1 removed the multi-sequencer
client and its statistics file, and `WalletCore::{new_update_chain, new_init_storage}` are no
longer `async`.

## 2. `SyncObserver`: sync progress without stdout

**Why:** sync used `println!` and an `indicatif` bar, so a UI had no way to show progress.

```rust
// wallet::sync_observer
pub trait SyncObserver: Send {                       // all methods default to no-op
    fn on_start(&mut self, blocks: RangeInclusive<BlockId>) {}
    fn on_block(&mut self, block_id: BlockId) {}      // block processed + storage persisted
    fn on_finish(&mut self, block_id: BlockId, elapsed: Duration) {}
    fn on_message(&mut self, message: &str) {}
}
#[derive(Default)] pub struct CliSyncObserver { .. }  // exact old output, keeps indicatif

// wallet::WalletCore
pub async fn sync_to_block_with_observer(&mut self, block_id: u64,
    observer: &mut dyn SyncObserver) -> Result<()>;
pub async fn sync_to_latest_block_with_observer(&mut self,
    observer: &mut dyn SyncObserver) -> Result<u64>;
// sync_to_block / sync_to_latest_block now call these with CliSyncObserver
```

The observer receives the old "Latest block is N" and per-block "Stored persistent accounts
at ..." lines as `on_message`.

## 3. Public tx: prepare, then sign and submit

**Why:** the user must approve the exact message before it is signed, and must be able to
drop it instead.

```rust
pub enum TxSigner {                                    // re-exported at crate root
    Local(AccountId),                                  // key looked up in storage at sign time
    Keycard { account_id: AccountId, key_path: String },
}
pub struct PreparedPublicTx { .. }                     // holds no private keys
impl PreparedPublicTx {
    pub const fn message(&self) -> &lee::public_transaction::Message; // incl. `.fee`, `.hash()`
    pub fn signers(&self) -> &[TxSigner];              // in signature order
}

// wallet::WalletCore
pub async fn prepare_public(&self, accounts: Vec<AccountMention>,
    instruction_data: InstructionData, program_account_id: AccountId,
    payer: Option<AccountId>,
    tx_pre_check: impl FnOnce(&[SelectedShard]) -> Result<(), ExecutionFailureKind>,
) -> Result<PreparedPublicTx, ExecutionFailureKind>;
pub async fn sign_and_submit_public(&self, prepared: PreparedPublicTx)
    -> Result<HashType, ExecutionFailureKind>;
// send_pub_tx_with_pre_check = prepare_public + sign_and_submit_public
```

The signature order is unchanged: local accounts, then keycard accounts, then an external fee
co-signer. For keycard signers, `PreparedPublicTx` holds the PIN that was read during
preparation (in a private field). `AccountManager` already did this. Signing moves from
`AccountManager::sign_message` to a crate-internal free `account_manager::sign_message`.

## 4. Private tx: prepare, prove, then sign and submit

**Why:** we need to run proving on our own worker thread, cancel it before anything is
submitted, and compare the proved public effects with what the user approved.

```rust
pub struct PreparedPrivateTx { .. }   // ProvingInput (holds private witnesses), program, signers
impl PreparedPrivateTx {
    /// CPU-heavy and blocking: run it on a worker thread.
    pub fn prove(self) -> Result<ProvedPrivateTx, ExecutionFailureKind>;
}
pub struct ProvedPrivateTx { .. }     // dropping it cancels the tx
impl ProvedPrivateTx {
    pub const fn message(&self) -> &privacy_preserving_transaction::message::Message;
    pub fn signers(&self) -> &[TxSigner];
}

// wallet::WalletCore
pub async fn prepare_private(&self, accounts: Vec<AccountMention>,
    instruction_data: InstructionData, program: &ProgramWithDependencies,
    tx_pre_check: impl FnOnce(&[SelectedShard]) -> Result<(), ExecutionFailureKind>,
) -> Result<PreparedPrivateTx, ExecutionFailureKind>;
pub async fn sign_and_submit_private(&self, proved: ProvedPrivateTx)
    -> Result<(HashType, Vec<SharedSecretKey>), ExecutionFailureKind>;
// send_privacy_preserving_tx_with_pre_check =
//   prepare_private + spawn_blocking(prove) + sign_and_submit_private
```

To cancel, drop the `ProvedPrivateTx`, or stop waiting for `prove`. A proof that is already
running still finishes on its thread, but nothing gets signed or submitted.

## 5. `keycard` Cargo feature (default on)

**Why:** `keycard_wallet` links PC/SC (`pcsc`), which we don't want in every module build.

```toml
[features]
default = ["keycard"]
keycard = ["dep:keycard_wallet"]
keycard-debug = ["keycard"]
```

Without the feature:

- The `wallet keycard ...` subcommand is not compiled.
- `m/...` key-path mentions and `TxSigner::Keycard` fail with "Keycard support is not enabled
  in this build".
- `pcsc` and `keycard-rs` drop out of the dependency tree.

`AccountIdentity::PublicKeycard` still exists, as plain data. Remaining gap: preparing a
`PublicKeycard` identity without the feature still prompts for a PIN before it fails at signing
time. Nothing in the CLI reaches that path, because key-path mentions fail earlier.

## 8. Low-memory proving: a settable segment size

`lee::privacy_preserving_transaction::circuit::set_segment_limit_po2(Some(n))` makes every
local proof started afterwards run with segments of 2^n cycles instead of risc0's default
2^20. Both proving paths (`execute_and_prove` for the privacy circuit and `prove_session` for
inner programs) apply it. Smaller segments need less memory and take longer, so a wallet on
a machine with little free memory can still prove instead of being killed. `None` restores
the default; the value is process-wide and read when a proof starts.

## Verification (at the tip of the series on `v0.3.1`, macOS arm64, toolchain 1.98.1)

Re-run 2026-10-15 for the `v0.3.1` re-pin. In a LEZ checkout with the series applied, with
`RISC0_SKIP_BUILD_KERNELS=1 RISC0_SKIP_BUILD=1`:

| Command | Result |
|---------|--------|
| `cargo check -p wallet` | Finished, exit 0 |
| `cargo check -p wallet --no-default-features` | Finished, exit 0 |
| `cargo clippy -p wallet --all-targets` | Finished, no warnings, exit 0 |
| `cargo clippy -p wallet --all-targets --no-default-features` | Finished, no warnings, exit 0 |
| `cargo check -p wallet-ffi` (downstream consumer, in `vendor/lez`) | Finished, exit 0 (without the `RISC0_SKIP_*` variables it fails in `risc0-sys`, which needs Xcode's Metal compiler) |

In `vendor/lez` as produced by `scripts/lez-vendor.sh` (no `RISC0_SKIP_*` variables):

| Command | Result |
|---------|--------|
| `cargo build --release -p wallet` | Finished, exit 0 |
| `RISC0_DEV_MODE=1 cargo test -p wallet --lib` | `test result: ok. 49 passed; 0 failed` |
| `RISC0_DEV_MODE=1 cargo test -p wallet --lib --no-default-features` | `test result: ok. 49 passed; 0 failed` |
| `cargo clippy -p wallet --all-targets --features keycard-debug` | Finished, no warnings, exit 0 |

v0.3.1 has 49 wallet unit tests (v0.3.0 had 69); upstream deleted the tests along with the
multi-sequencer client and the token/AMM/ATA facades. The series adds no tests of its own.

Clippy was checked at the tip only, not after each patch. `scripts/lez-vendor.sh` (`git am`
of the eight files onto `1a003bf`) reproduces the branch's tree exactly, and re-exporting
from that `vendor/lez` gives back the same eight files. Formatting uses stable `rustfmt`,
because the repo's nightly-only rustfmt options were unavailable.

## Upstream PR summary

> **wallet: make `WalletCore` embeddable (storage backend, sync observer, staged tx sending,
> optional keycard)**
>
> `WalletCore` is currently hard to embed outside the CLI:
>
> - It always writes plaintext JSON to disk.
> - Sync prints to stdout.
> - Building, proving, signing and submitting a transaction happen in one call.
> - It always links PC/SC.
>
> This series adds five small extension points:
>
> 1. A `StorageBackend` trait (`FileBackend` is the default), plus
>    `Storage::{to_bytes, from_bytes}`.
> 2. A `SyncObserver` for sync progress (`CliSyncObserver` is the default).
> 3. `prepare_public` / `sign_and_submit_public`, where `PreparedPublicTx` exposes the exact
>    `Message` and `TxSigner`s and holds no keys.
> 4. `prepare_private` / `PreparedPrivateTx::prove` / `sign_and_submit_private`, so proving can
>    run on a caller-chosen thread and can be cancelled before submission.
> 5. A default-on `keycard` feature.
>
> Each existing entry point is now a composition of the new ones. The CLI's output, on-disk
> format, signature ordering and errors are unchanged. There are no new dependencies. Clippy
> passes with the workspace lints, and all 49 wallet unit tests pass with and without default
> features.
