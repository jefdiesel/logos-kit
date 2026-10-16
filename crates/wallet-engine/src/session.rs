//! A wallet session: our data dir, zones, and one LEZ `WalletCore` whose
//! storage lives in an encrypted vault.
//!
//! Layout (never `~/.lee/wallet`; the official CLI keeps its own):
//!
//! ```text
//! <data>/.session.lock              held while a process has the wallet open
//! <data>/keys/vault.json            vault.v1 (password): phrase, zone keys, wallet metadata
//! <data>/zones.json                 zone list hint for the lock screen (not trusted)
//! <data>/zones/<zone>/vault.json    vault.v1 (keyed): LEZ Storage for that zone
//! <data>/zones/<zone>/wallet_config.json
//! ```
//!
//! **Key hierarchy.** The password runs Argon2id once and opens the keys
//! vault. That vault holds a random key per zone; each zone vault is sealed by
//! its own key and bound to its zone id, so one zone's file can't stand in for
//! another's. A password change rewrites only the keys vault (one atomic write).
//!
//! **Zones.** LEZ's `Storage` carries sync position and private-account state
//! for one chain, so it lives per zone. Adding a zone (the LP-0022 hook)
//! restores the same phrase into a fresh zone vault and re-derives the same
//! accounts (the persisted account counts), so accounts match everywhere
//! while each zone syncs on its own.
//!
//! **First sync of a zone.** Blocks before the wallet's birthday (creation
//! time, or the restore date the user gave) are skipped: private notes can't
//! be older than the wallet. A restored wallet also discovers which accounts
//! were used, the way LEZ's own `restore-keys` does: derive a tree of
//! accounts, sync, then drop the unused ones.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, bail, ensure};
use chacha20poly1305::aead::{OsRng, rand_core::RngCore as _};
use sequencer_service_rpc::RpcClient as _;
use serde::{Deserialize, Serialize};
use wallet::{
    WalletCore,
    account::{AccountIdWithPrivacy, Label},
    config::{SequencerConnectionData, WalletConfig},
    storage::{Storage, StorageBackend},
    sync_observer::SyncObserver,
};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::vault::{
    EncryptedBackend, KdfCost, SessionLock, Vault, atomic_write, new_key, private_dir,
};

/// How often LEZ's save-per-block may hit the disk (the rest is buffered).
const SAVE_INTERVAL: Duration = Duration::from_secs(5);
/// Account discovery on restore: accounts whose layered depth is below this
/// (31 public + 31 private). Same meaning as LEZ `restore-keys --depth`.
const DISCOVERY_DEPTH: u32 = 6;
/// Start scanning this long before the birthday (clock skew, a restore date
/// given as a calendar day).
pub const BIRTHDAY_MARGIN_MS: u64 = 24 * 60 * 60 * 1000;
const AUTO_LOCK_DEFAULT_SECS: u32 = 15 * 60;
const AUTO_LOCK_BOUNDS_SECS: std::ops::RangeInclusive<u32> = 60..=24 * 60 * 60;
const BACKOFF_BASE: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_secs(60);
const LABEL_MAX_CHARS: usize = 32;

/// The preview network's sequencer and its drip faucet (`logos-kit-drip`).
pub const PREVIEW_SEQUENCER: &str = "https://lez.84.46.247.92.sslip.io";
pub const PREVIEW_FAUCET: &str = "https://lez-drip.84.46.247.92.sslip.io";
/// Logos Kit's drip faucet for the official testnet (`deploy/testnet`).
pub const TESTNET_FAUCET: &str = "https://lez-testnet-drip.84.46.247.92.sslip.io";

/// A LEZ zone the wallet talks to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Zone {
    /// Directory-safe id, e.g. `lez-testnet`.
    pub id: String,
    /// CAIP-2 chain id, e.g. `lez:testnet`.
    pub chain: String,
    pub sequencer: String,
}

impl Zone {
    pub fn testnet() -> Self {
        Self {
            id: "lez-testnet".into(),
            chain: "lez:testnet".into(),
            sequencer: "https://testnet.lez.logos.co".into(),
        }
    }

    /// Logos Kit's own public LEZ 0.3 network, from before the official
    /// testnet moved to 0.3. Kept for wallets that saved it. The URL is
    /// permanent: a saved zone must keep its URL, so a real domain later is an
    /// extra name, never a replacement.
    pub fn preview() -> Self {
        Self {
            id: "lez-preview".into(),
            chain: "lez:preview".into(),
            sequencer: PREVIEW_SEQUENCER.into(),
        }
    }

    /// Zones every wallet knows, default first.
    pub fn builtin() -> [Self; 3] {
        [Self::testnet(), Self::preview(), Self::local()]
    }

    pub fn local() -> Self {
        Self {
            id: "lez-local".into(),
            chain: "lez:local".into(),
            sequencer: "http://127.0.0.1:3040".into(),
        }
    }

    /// The drip faucet Logos Kit runs for this zone, if any.
    pub fn builtin_faucet(&self) -> Option<&'static str> {
        if *self == Self::testnet() {
            Some(TESTNET_FAUCET)
        } else if *self == Self::preview() {
            Some(PREVIEW_FAUCET)
        } else {
            None
        }
    }

    fn check(&self) -> Result<()> {
        ensure!(
            !self.id.is_empty()
                && self.id.bytes().all(|b| b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || b == b'-'
                    || b == b'_'),
            // Lowercase only: on case-insensitive disks `A` and `a` are one directory.
            "zone id must be lowercase letters, digits, '-' or '_'"
        );
        ensure!(
            self.chain.starts_with("lez:"),
            "zone chain must be a CAIP-2 lez:* id"
        );
        ensure!(
            self.sequencer.starts_with("https://") || self.sequencer.starts_with("http://"),
            "sequencer must be an http(s) URL"
        );
        // Parse exactly as LEZ will, so a bad URL fails before anything is written.
        serde_json::from_value::<SequencerConnectionData>(
            serde_json::json!({ "sequencer_addr": self.sequencer }),
        )
        .context("sequencer URL")?;
        Ok(())
    }

    fn vault_context(&self) -> String {
        format!("zone:{}", self.id)
    }
}

/// The wallet's data directory.
#[derive(Clone, Debug)]
pub struct DataDir(PathBuf);

impl DataDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self(root.into())
    }

    pub fn root(&self) -> &Path {
        &self.0
    }

    fn keys(&self) -> PathBuf {
        self.0.join("keys")
    }

    fn zone(&self, id: &str) -> PathBuf {
        self.0.join("zones").join(id)
    }

    /// Undo a create/restore that failed after writing the keys vault, so no
    /// wallet exists whose recovery phrase the user never saw.
    fn forget_partial(&self) {
        let _ = std::fs::remove_dir_all(self.keys());
    }

    pub fn is_initialized(&self) -> bool {
        Vault::exists(&self.keys())
    }

    /// Zones last used, for the lock screen. Unauthenticated: after unlock
    /// the keys vault's zone list is the one that counts.
    pub fn zones(&self) -> Result<Vec<Zone>> {
        match std::fs::read(self.0.join("zones.json")) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).context("zones.json")?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    fn write_zones_hint(&self, zones: &[Zone]) -> Result<()> {
        private_dir(&self.0)?;
        atomic_write(&self.0, "zones.json", &serde_json::to_vec_pretty(zones)?)
    }

    fn write_zone_config(&self, zone: &Zone) -> Result<PathBuf> {
        let dir = self.zone(&zone.id);
        private_dir(&dir)?;
        let config = WalletConfig {
            sequencer: SequencerConnectionData {
                sequencer_addr: zone.sequencer.parse()?,
                basic_auth: None,
            },
            // Upstream gives up after 5 polls 12 s apart. The official testnet
            // can take longer than a minute to include a transaction (seen
            // 2026-10-06: a program segment landed minutes after the wallet
            // declared it lost), so wait up to ~6 min before calling it failed.
            seq_tx_poll_max_blocks: 30,
            ..WalletConfig::default()
        };
        atomic_write(
            &dir,
            "wallet_config.json",
            &serde_json::to_vec_pretty(&config)?,
        )?;
        Ok(dir.join("wallet_config.json"))
    }
}

// ---------------------------------------------------------------------------
// What the keys vault holds

/// Secret half of the keys vault; wiped from memory on drop.
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
struct Secrets {
    phrase: String,
    zone_keys: Vec<ZoneKey>,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct ZoneKey {
    zone: String,
    /// Base64 of the 32-byte zone vault key.
    key: String,
}

impl Secrets {
    fn zone_key(&self, zone: &str) -> Result<Option<Zeroizing<[u8; 32]>>> {
        use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
        let Some(entry) = self.zone_keys.iter().find(|k| k.zone == zone) else {
            return Ok(None);
        };
        let bytes = Zeroizing::new(B64.decode(&entry.key).context("zone key")?);
        let mut key = Zeroizing::new([0_u8; 32]);
        ensure!(bytes.len() == 32, "zone key must be 32 bytes");
        key.copy_from_slice(&bytes);
        Ok(Some(key))
    }

    fn set_zone_key(&mut self, zone: &str, key: &[u8; 32]) {
        use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
        self.zone_keys.retain(|k| k.zone != zone);
        self.zone_keys.push(ZoneKey {
            zone: zone.to_owned(),
            key: B64.encode(key),
        });
    }
}

/// Non-secret wallet metadata. Lives in the keys vault so it is authenticated.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    created_at_ms: u64,
    /// Scan for private notes from here (unix ms). `None`: from genesis.
    birthday_ms: Option<u64>,
    /// Restored from a phrase: run account discovery on each zone's first sync.
    restored: bool,
    /// Zones whose discovery is done (or wasn't needed).
    discovered: BTreeSet<String>,
    /// Layered accounts derived so far, replayed on every new zone.
    accounts: Counts,
    /// account id → label, applied on every new zone.
    labels: BTreeMap<String, String>,
    /// The authoritative zone list.
    zones: Vec<Zone>,
    auto_lock_secs: u32,
    /// What connected apps may do (see `policy`).
    #[serde(default)]
    grants: Vec<crate::policy::Grant>,
    /// zone id → token definitions to look for (ATAs, token list).
    #[serde(default)]
    tokens: BTreeMap<String, Vec<String>>,
    /// Base64 key for per-app private account handles (made on first use).
    #[serde(default)]
    handle_key: Option<String>,
    /// Accounts the wallet made for its own use (token IDs, program
    /// headers and segments): kept out of the account switcher.
    #[serde(default)]
    system: BTreeSet<String>,
    /// Programs the user named, every zone (approvals show the name).
    #[serde(default)]
    programs: Vec<crate::verify::UserProgram>,
    /// zone id → token definitions the user hid / pinned.
    #[serde(default)]
    hidden_tokens: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pinned_tokens: BTreeMap<String, Vec<String>>,
    /// zone id → definition → display decimals the user set.
    #[serde(default)]
    token_decimals: BTreeMap<String, BTreeMap<String, u8>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Counts {
    public: u32,
    private: u32,
}

#[derive(Serialize, Deserialize)]
struct KeysRecord {
    secrets: Secrets,
    meta: Meta,
}

impl KeysRecord {
    fn parse(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).context("keys vault contents")
    }

    /// Serialized into a buffer sized up front, so no reallocation leaves a
    /// stray copy of the phrase in freed memory.
    fn to_bytes(&self) -> Result<Zeroizing<Vec<u8>>> {
        struct Count(usize);
        impl std::io::Write for Count {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0 += buf.len();
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut count = Count(0);
        serde_json::to_writer(&mut count, self)?;
        let mut out = Zeroizing::new(Vec::with_capacity(count.0));
        serde_json::to_writer(&mut *out, self)?;
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Public types

/// Hands LEZ a shared handle to our backend, so the session can still flush it.
struct Shared(Arc<EncryptedBackend>);

impl StorageBackend for Shared {
    fn load(&self) -> Result<Option<Vec<u8>>> {
        self.0.load()
    }

    fn save(&self, bytes: &[u8]) -> Result<()> {
        self.0.save(bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub account_id: String,
    pub kind: AccountKind,
    /// Key-tree path, e.g. `/0`.
    pub path: Option<String>,
    pub label: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    Public,
    Private,
}

/// How a restore should pick its starting point for private-note scanning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Birthday {
    /// The wallet was first used around this time (unix ms). Private notes
    /// older than this (minus a day) are not scanned, so a date that is too
    /// late hides those notes and discovery drops their accounts.
    At(u64),
    /// Scan the whole chain (slow on a long chain; always complete).
    Genesis,
}

/// Returned (inside `anyhow::Error`) while the network is in backoff.
#[derive(Debug)]
pub struct Offline {
    pub retry_in: Duration,
    pub reason: String,
}

impl fmt::Display for Offline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "offline ({}); retrying in {}s",
            self.reason,
            self.retry_in.as_secs().max(1)
        )
    }
}

impl std::error::Error for Offline {}

/// Returned (inside `anyhow::Error`) while phrase reveal is throttled.
#[derive(Debug)]
pub struct Throttled {
    pub retry_in: Duration,
}

impl fmt::Display for Throttled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "too many wrong passwords; try again in {}s",
            self.retry_in.as_secs().max(1)
        )
    }
}

impl std::error::Error for Throttled {}

/// Network state of the session's zone, for the offline banner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum NetStatus {
    /// Not tried yet in this session.
    Idle,
    #[serde(rename_all = "camelCase")]
    Online { tip: u64 },
    #[serde(rename_all = "camelCase")]
    Offline {
        attempts: u32,
        retry_in_ms: u64,
        error: String,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoneStatus {
    pub zone: Zone,
    pub network: NetStatus,
    pub synced_block: u64,
    /// Restored wallet still finding its used accounts on this zone.
    pub discovering: bool,
    pub auto_lock_secs: u32,
}

pub(crate) type PendingNote = (common::HashType, crate::tx::NoteSecrets);

enum Failure {
    Network(anyhow::Error),
    Local(anyhow::Error),
}

enum Net {
    Idle,
    Online {
        tip: u64,
    },
    Offline {
        attempts: u32,
        retry_at: Instant,
        error: String,
    },
}

/// Exponential backoff with jitter: base·2^(n-1), capped, then a random point
/// in its upper half so many wallets don't retry in lockstep.
fn backoff(attempts: u32) -> Duration {
    let exp = BACKOFF_BASE.saturating_mul(1 << attempts.saturating_sub(1).min(16));
    let full = exp.min(BACKOFF_CAP);
    let half = full / 2;
    let jitter_ms = OsRng.next_u64() % (u64::try_from(half.as_millis()).unwrap_or(0) + 1);
    half + Duration::from_millis(jitter_ms)
}

/// Failed re-auth attempts (phrase reveal, password change): after the 3rd
/// wrong password, doubling waits (1 s … 5 min) before the next try.
#[derive(Default)]
struct RevealThrottle {
    failures: u32,
    until: Option<Instant>,
}

impl RevealThrottle {
    fn check(&self) -> Result<()> {
        if let Some(until) = self.until
            && let Some(left) = until.checked_duration_since(Instant::now())
        {
            return Err(Throttled { retry_in: left }.into());
        }
        Ok(())
    }

    fn fail(&mut self) {
        self.failures += 1;
        if self.failures >= 3 {
            let secs = 1_u64 << (self.failures - 3).min(8);
            self.until = Some(Instant::now() + Duration::from_secs(secs.min(300)));
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn check_label(label: &str) -> Result<String> {
    let label = label.trim();
    ensure!(!label.is_empty(), "label is empty");
    ensure!(
        label.chars().count() <= LABEL_MAX_CHARS,
        "label is longer than {LABEL_MAX_CHARS} characters"
    );
    ensure!(
        !label.chars().any(|c| c.is_control() || c == '/'),
        "label can't contain '/' or control characters"
    );
    Ok(label.to_owned())
}

fn with_privacy(kind: AccountKind, account_id: &str) -> Result<AccountIdWithPrivacy> {
    let prefix = match kind {
        AccountKind::Public => "Public",
        AccountKind::Private => "Private",
    };
    format!("{prefix}/{account_id}")
        .parse()
        .context("account id")
}

/// Accounts derived from the layered tree (everything but the root `/`).
fn layered_counts(storage: &Storage) -> Counts {
    // Distinct tree paths: one private node can carry several account ids,
    // and imported accounts have no path.
    let keys = storage.key_chain();
    let count = |paths: BTreeSet<String>| u32::try_from(paths.len()).unwrap_or(u32::MAX);
    let tree_path = |path: Option<String>| path.filter(|p| p != "/");
    Counts {
        public: count(
            keys.public_account_ids()
                .filter_map(|(_, p)| tree_path(p.map(ToString::to_string)))
                .collect(),
        ),
        private: count(
            keys.private_account_ids()
                .filter_map(|(_, p)| tree_path(p.map(ToString::to_string)))
                .collect(),
        ),
    }
}

/// The chain link. Keys and accounts live in `Storage`, so create, restore,
/// unlock and account derivation work offline; `WalletCore` (which needs a
/// reachable sequencer) is built on [`Session::connect`] and takes the storage.
enum Link {
    Offline(Box<Storage>),
    Online(Box<WalletCore>),
    /// Only observable if `connect` panicked mid-swap.
    Poisoned,
}

pub struct Session {
    data: DataDir,
    zone: Zone,
    dir: PathBuf,
    config_path: PathBuf,
    link: Link,
    backend: Arc<EncryptedBackend>,
    keys: Vault,
    meta: Meta,
    /// Finished activity for this zone, sealed with the zone key (`None`:
    /// the file couldn't be opened; activity stays in memory this session).
    history: Option<Vault>,
    net: Net,
    reveal: RevealThrottle,
    /// Secrets of a private tx sent from here, to record its note on inclusion.
    pub(crate) pending_note: Option<PendingNote>,
    /// Token names read from definitions (they never change).
    pub(crate) token_names: std::collections::HashMap<String, Option<String>>,
    // Declared last so it is released after everything above is flushed.
    _lock: SessionLock,
}

impl Session {
    /// New wallet: fresh recovery phrase, keys vault, first zone. Returns the
    /// phrase once so the UI can show it for backup. Works offline.
    pub fn create(
        data: DataDir,
        password: &str,
        zone: Zone,
        cost: KdfCost,
    ) -> Result<(Self, Zeroizing<String>)> {
        zone.check()?;
        let lock = SessionLock::acquire(data.root())?;
        ensure!(
            !data.is_initialized(),
            "a wallet already exists in {}",
            data.root().display()
        );
        let (storage, mnemonic) = Storage::new("")?;
        let phrase = Zeroizing::new(mnemonic.to_string());
        let created = now_ms();
        let meta = Meta {
            created_at_ms: created,
            birthday_ms: Some(created),
            auto_lock_secs: AUTO_LOCK_DEFAULT_SECS,
            ..Meta::default()
        };
        let session = Self::create_keys(&data, password, &phrase, meta, cost)
            .and_then(|(keys, record)| {
                Self::open_zone(data.clone(), zone, keys, record, Some(storage), lock)
            })
            .inspect_err(|_| data.forget_partial())?;
        Ok((session, phrase))
    }

    /// Restore from a recovery phrase (e.g. one made by the official LEZ
    /// wallet). Works offline; accounts are discovered on the first sync.
    pub fn restore(
        data: DataDir,
        password: &str,
        phrase: &str,
        birthday: Birthday,
        zone: Zone,
        cost: KdfCost,
    ) -> Result<Self> {
        zone.check()?;
        let lock = SessionLock::acquire(data.root())?;
        ensure!(
            !data.is_initialized(),
            "a wallet already exists in {}",
            data.root().display()
        );
        let mnemonic = bip39::Mnemonic::parse(phrase).context("invalid recovery phrase")?;
        let phrase = Zeroizing::new(mnemonic.to_string());
        let meta = Meta {
            created_at_ms: now_ms(),
            birthday_ms: match birthday {
                Birthday::At(ms) => Some(ms),
                Birthday::Genesis => None,
            },
            restored: true,
            auto_lock_secs: AUTO_LOCK_DEFAULT_SECS,
            ..Meta::default()
        };
        Self::create_keys(&data, password, &phrase, meta, cost)
            .and_then(|(keys, record)| {
                Self::open_zone(data.clone(), zone, keys, record, None, lock)
            })
            .inspect_err(|_| data.forget_partial())
    }

    /// Unlock an existing wallet on one of its zones (added if new). Works
    /// offline. The only Argon2 run: everything else is opened by key.
    pub fn unlock(data: DataDir, password: &str, zone: Zone) -> Result<Self> {
        zone.check()?;
        let lock = SessionLock::acquire(data.root())?;
        let (keys, bytes) = Vault::unlock(data.keys(), password)?;
        let record = KeysRecord::parse(&bytes)?;
        if let Some(known) = record.meta.zones.iter().find(|z| z.id == zone.id) {
            ensure!(
                *known == zone,
                "zone {} is configured as {} at {}; remove and re-add it to change it",
                zone.id,
                known.chain,
                known.sequencer
            );
        }
        Self::open_zone(data, zone, keys, record, None, lock)
    }

    fn create_keys(
        data: &DataDir,
        password: &str,
        phrase: &str,
        meta: Meta,
        cost: KdfCost,
    ) -> Result<(Vault, KeysRecord)> {
        let record = KeysRecord {
            secrets: Secrets {
                phrase: phrase.to_owned(),
                zone_keys: Vec::new(),
            },
            meta,
        };
        let keys = Vault::create(data.keys(), password, &record.to_bytes()?, cost)?;
        Ok((keys, record))
    }

    fn open_zone(
        data: DataDir,
        zone: Zone,
        keys: Vault,
        mut record: KeysRecord,
        fresh: Option<Storage>,
        lock: SessionLock,
    ) -> Result<Self> {
        let dir = data.zone(&zone.id);
        let config_path = data.write_zone_config(&zone)?;
        let context = zone.vault_context();

        let key = record.secrets.zone_key(&zone.id)?;
        let (vault, storage) = match key {
            Some(key) if Vault::exists(&dir) => {
                let (vault, bytes) = Vault::unlock_keyed(&dir, key, &context)?;
                (vault, Storage::from_bytes(&bytes)?)
            }
            None if Vault::exists(&dir) => {
                // Zone state (imported keys, notes) is never deleted on a guess:
                // this happens only if the keys vault was rolled back.
                bail!(
                    "{} holds a zone vault this wallet has no key for (keys vault restored from an older copy?); move it away to start this zone over",
                    dir.display()
                );
            }
            key => {
                let storage = match fresh {
                    Some(storage) => storage,
                    None => Self::derive_zone_storage(&record)?,
                };
                // Record the key before writing the vault: a crash in between
                // leaves a key without a vault, which the next open recreates.
                let key = match key {
                    Some(key) => key,
                    None => {
                        let key = new_key();
                        record.secrets.set_zone_key(&zone.id, &key);
                        if !record.meta.restored {
                            record.meta.discovered.insert(zone.id.clone());
                        }
                        keys.save(&record.to_bytes()?)?;
                        key
                    }
                };
                let vault =
                    Vault::create_keyed(&dir, key, &context, &Zeroizing::new(storage.to_bytes()?))?;
                (vault, storage)
            }
        };

        if !record.meta.zones.contains(&zone) {
            record.meta.zones.push(zone.clone());
            keys.save(&record.to_bytes()?)?;
        }
        data.write_zones_hint(&record.meta.zones)?;

        let history = open_history(&dir, &context, record.secrets.zone_key(&zone.id)?);
        let backend = Arc::new(EncryptedBackend::new(vault, SAVE_INTERVAL)?);
        let meta = record.meta.clone();
        Ok(Self {
            data,
            zone,
            dir,
            config_path,
            link: Link::Offline(Box::new(storage)),
            backend,
            keys,
            meta,
            history,
            net: Net::Idle,
            reveal: RevealThrottle::default(),
            pending_note: None,
            token_names: std::collections::HashMap::new(),
            _lock: lock,
        })
    }

    /// Storage for a zone we haven't seen: the phrase's key tree, shaped like
    /// the wallet's other zones (or a discovery tree for a restored wallet),
    /// with the wallet's labels.
    fn derive_zone_storage(record: &KeysRecord) -> Result<Storage> {
        let mnemonic =
            bip39::Mnemonic::parse(&record.secrets.phrase).context("keys vault phrase")?;
        let (mut storage, _) = Storage::new("")?;
        storage.restore(&mnemonic, "")?;
        let keys = storage.key_chain_mut();
        if record.meta.restored {
            // LEZ requires a fresh tree here, which this is.
            keys.generate_trees_for_depth(DISCOVERY_DEPTH);
        } else {
            // Layered derivation is deterministic, so replaying the counts
            // yields the same accounts at the same paths as the other zones.
            for _ in 0..record.meta.accounts.public {
                keys.generate_new_public_transaction_private_key(None);
            }
            for _ in 0..record.meta.accounts.private {
                keys.generate_new_privacy_preserving_transaction_key_chain(None);
            }
        }
        let known: BTreeMap<String, AccountKind> = public_and_private(&storage)
            .into_iter()
            .map(|(id, kind, _)| (id, kind))
            .collect();
        for (account_id, label) in &record.meta.labels {
            if let Some(kind) = known.get(account_id) {
                let _ = storage.add_label(Label::new(label), with_privacy(*kind, account_id)?);
            }
        }
        Ok(storage)
    }

    /// Reach the zone's sequencer. On failure the session stays offline and
    /// usable (accounts, receive codes); the error says why.
    pub async fn connect(&mut self) -> Result<()> {
        let storage = match std::mem::replace(&mut self.link, Link::Poisoned) {
            Link::Offline(storage) => *storage,
            online => {
                self.link = online;
                return Ok(());
            }
        };
        // Keep the bytes: WalletCore::new consumes the storage even when it fails.
        let fallback = Zeroizing::new(storage.to_bytes()?);
        // Since LEZ v0.3.1 building the WalletCore no longer talks to the
        // sequencer (the multi-sequencer client that probed it is gone), so
        // ask it for its health to keep "connected" meaning "reachable".
        let core = match WalletCore::new_with_storage_backend(
            self.config_path.clone(),
            // LEZ installs a plaintext FileBackend at this path before swapping
            // in ours; point it somewhere nothing ever reads, never the vault.
            self.dir.join(".lez-storage-unused"),
            None,
            storage,
            Box::new(Shared(Arc::clone(&self.backend))),
        ) {
            Ok(core) => match core.client().check_health().await {
                Ok(()) => Ok(core),
                Err(e) => Err(anyhow::Error::from(e)),
            },
            Err(e) => Err(e),
        };
        match core {
            Ok(core) => {
                self.link = Link::Online(Box::new(core));
                Ok(())
            }
            Err(e) => {
                self.link = Link::Offline(Box::new(Storage::from_bytes(&fallback)?));
                Err(e.context(format!("can't reach {}", self.zone.sequencer)))
            }
        }
    }

    pub const fn is_online(&self) -> bool {
        matches!(self.link, Link::Online(_))
    }

    pub const fn zone(&self) -> &Zone {
        &self.zone
    }

    pub const fn data_dir(&self) -> &DataDir {
        &self.data
    }

    /// The LEZ wallet, once connected.
    pub fn core(&self) -> Option<&WalletCore> {
        match &self.link {
            Link::Online(core) => Some(core),
            _ => None,
        }
    }

    pub fn core_mut(&mut self) -> Option<&mut WalletCore> {
        match &mut self.link {
            Link::Online(core) => Some(core),
            _ => None,
        }
    }

    fn storage(&self) -> Result<&Storage> {
        match &self.link {
            Link::Offline(storage) => Ok(storage),
            Link::Online(core) => Ok(core.storage()),
            Link::Poisoned => {
                bail!("session is unusable after a failed connect; unlock again")
            }
        }
    }

    fn storage_mut(&mut self) -> Result<&mut Storage> {
        match &mut self.link {
            Link::Offline(storage) => Ok(storage),
            Link::Online(core) => Ok(core.storage_mut()),
            Link::Poisoned => {
                bail!("session is unusable after a failed connect; unlock again")
            }
        }
    }

    fn discovering(&self) -> bool {
        self.meta.restored && !self.meta.discovered.contains(&self.zone.id)
    }

    // -- accounts ------------------------------------------------------------

    pub fn accounts(&self) -> Result<Vec<AccountInfo>> {
        let storage = self.storage()?;
        public_and_private(storage)
            .into_iter()
            .map(|(account_id, kind, path)| {
                let label = storage
                    .labels_for_account(with_privacy(kind, &account_id)?)
                    .next()
                    .map(ToString::to_string);
                Ok(AccountInfo {
                    account_id,
                    kind,
                    path,
                    label,
                })
            })
            .collect()
    }

    /// Derive the next account of `kind` (the same key-chain calls LEZ's
    /// `WalletCore::create_new_account_*` make) and persist it right away.
    pub fn new_account(&mut self, kind: AccountKind) -> Result<AccountInfo> {
        // The discovery tree fills every slot above its depth, so an account
        // made now would land outside it and not match other zones.
        ensure!(
            !self.discovering(),
            "still finding this wallet's accounts; sync once, then add accounts"
        );
        let keys = self.storage_mut()?.key_chain_mut();
        let (id, path) = match kind {
            AccountKind::Public => keys.generate_new_public_transaction_private_key(None),
            AccountKind::Private => {
                keys.generate_new_privacy_preserving_transaction_key_chain(None)
            }
        };
        self.persist_now()?;
        let counts = layered_counts(self.storage()?);
        self.update_meta(|meta| {
            meta.accounts.public = meta.accounts.public.max(counts.public);
            meta.accounts.private = meta.accounts.private.max(counts.private);
        })?;
        Ok(AccountInfo {
            account_id: id.to_string(),
            kind,
            path: Some(path.to_string()),
            label: None,
        })
    }

    /// A new account for the user, named "Public account N" / "Private
    /// account N" (the lowest N not taken).
    pub fn new_named_account(&mut self, kind: AccountKind) -> Result<AccountInfo> {
        let word = match kind {
            AccountKind::Public => "Public",
            AccountKind::Private => "Private",
        };
        // Free across the whole wallet (labels apply to every zone), chosen
        // before the account exists so a clash can't leave it unnamed.
        let taken = self.taken_labels()?;
        let label = (1..)
            .map(|n| format!("{word} account {n}"))
            .find(|l| !taken.contains(l))
            .expect("some number is free");
        let mut info = self.new_account(kind)?;
        self.set_label(&info.account_id, Some(&label))?;
        info.label = Some(label);
        Ok(info)
    }

    /// A public account the wallet uses itself (a token's definition, a
    /// program's header or segment), labelled and kept out of the switcher.
    pub fn new_system_account(&mut self, label: &str) -> Result<AccountInfo> {
        let taken = self.taken_labels()?;
        // Labels are unique and at most 32 characters: a repeat gets a number.
        let base: String = label.chars().take(28).collect();
        let label = std::iter::once(base.clone())
            .chain((2..).map(|n| format!("{base} {n}")))
            .find(|l| !taken.contains(l))
            .expect("some number is free");
        let mut info = self.new_account(AccountKind::Public)?;
        // Hidden first: if labelling fails, the account still stays out of
        // the switcher.
        let id = info.account_id.clone();
        self.update_meta(|m| {
            m.system.insert(id);
        })?;
        self.set_label(&info.account_id, Some(&label))?;
        info.label = Some(label);
        Ok(info)
    }

    fn taken_labels(&self) -> Result<BTreeSet<String>> {
        let mut taken: BTreeSet<String> = self.meta.labels.values().cloned().collect();
        taken.extend(self.accounts()?.into_iter().filter_map(|a| a.label));
        Ok(taken)
    }

    /// Programs the user named on this zone.
    pub fn named_programs(&self) -> Vec<crate::verify::UserProgram> {
        self.meta
            .programs
            .iter()
            .filter(|p| p.zone == self.zone().id)
            .cloned()
            .collect()
    }

    /// Name `account` (a program) on this zone; a new name replaces the old.
    pub fn name_program(&mut self, account: &str, name: &str) -> Result<()> {
        let name = name.trim();
        ensure!(
            !name.is_empty() && name.chars().count() <= 40,
            "a program name is 1–40 characters"
        );
        ensure!(
            !name.chars().any(char::is_control),
            "a program name can't contain control characters"
        );
        crate::decode::account_id(account)?;
        let zone = self.zone().id.clone();
        let entry = crate::verify::UserProgram {
            zone: zone.clone(),
            account: account.to_owned(),
            name: name.to_owned(),
            added_ms: now_ms(),
        };
        self.update_meta(|m| {
            m.programs
                .retain(|p| !(p.zone == zone && p.account == entry.account));
            m.programs.push(entry);
        })
    }

    pub fn forget_program(&mut self, account: &str) -> Result<()> {
        let zone = self.zone().id.clone();
        self.update_meta(|m| {
            m.programs
                .retain(|p| !(p.zone == zone && p.account == account))
        })
    }

    /// Made by the wallet for its own use (see `new_system_account`).
    pub fn is_system_account(&self, account_id: &str) -> bool {
        self.meta.system.contains(account_id)
            || self.tracked_tokens().iter().any(|t| t == account_id)
    }

    /// Import a public account by its private key (hex), e.g. a key from
    /// the official wallet or a test genesis account. Imported keys live in
    /// this zone only; they are not derived from the phrase.
    pub fn import_public_key(&mut self, private_key_hex: &str) -> Result<AccountInfo> {
        let key: lee::PrivateKey = private_key_hex
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("not a hex private key"))?;
        let id = lee::AccountId::from(&lee::PublicKey::new_from_private_key(&key));
        self.storage_mut()?
            .key_chain_mut()
            .add_imported_public_account(key);
        self.persist_now()?;
        Ok(AccountInfo {
            account_id: id.to_string(),
            kind: AccountKind::Public,
            path: None,
            label: None,
        })
    }

    /// What someone needs to pay this private account privately: its
    /// nullifier and viewing public keys, hex (LEZ `show-keys` format). Not
    /// secret; the sender picks a fresh identifier per payment.
    pub fn receive_keys(&self, account_id: &str) -> Result<(String, String)> {
        let id: lee::AccountId = crate::decode::account_id(account_id)?;
        let entry = self
            .storage()?
            .key_chain()
            .private_account(id)
            .context("not a private account of this wallet")?;
        Ok((
            hex::encode(entry.key_chain.nullifier_public_key.0),
            hex::encode(entry.key_chain.viewing_public_key.to_bytes()),
        ))
    }

    /// x-only public key of one of this wallet's public accounts.
    pub(crate) fn public_key(&self, account_id: &str) -> Result<[u8; 32]> {
        let id: lee::AccountId = crate::decode::account_id(account_id)?;
        let key = self
            .storage()?
            .key_chain()
            .pub_account_signing_key(id)
            .context("not a public account of this wallet")?;
        Ok(*lee::PublicKey::new_from_private_key(key).value())
    }

    /// BIP-340 signature by one of this wallet's public accounts over a
    /// 32-byte prehash (message signing), and its x-only public key. The key
    /// never leaves the key chain.
    pub(crate) fn sign_prehash(
        &self,
        account_id: &str,
        prehash: &[u8; 32],
    ) -> Result<([u8; 64], [u8; 32])> {
        let id: lee::AccountId = crate::decode::account_id(account_id)?;
        let key = self
            .storage()?
            .key_chain()
            .pub_account_signing_key(id)
            .context("not a public account of this wallet")?;
        let signature = lee::Signature::new(key, prehash);
        Ok((
            signature.value,
            *lee::PublicKey::new_from_private_key(key).value(),
        ))
    }

    /// Name an account (`None` clears it). One label per account; names are
    /// unique within the wallet. Stored in LEZ's own label map, so the
    /// official CLI sees it too.
    pub fn set_label(&mut self, account_id: &str, label: Option<&str>) -> Result<()> {
        let label = label.map(check_label).transpose()?;
        let account = self
            .accounts()?
            .into_iter()
            .find(|a| a.account_id == account_id)
            .context("no such account in this wallet")?;
        let id = with_privacy(account.kind, account_id)?;
        if let Some(label) = &label
            && self
                .meta
                .labels
                .iter()
                .any(|(other, l)| l == label && other != account_id)
        {
            bail!("another account is already called {label:?}");
        }
        let storage = self.storage_mut()?;
        if let Some(label) = &label
            && let Some(other) = storage.resolve_label(&Label::new(label))
            && other != id
        {
            bail!("another account is already called {label:?}");
        }
        let old: Vec<Label> = storage.labels_for_account(id).cloned().collect();
        for l in &old {
            storage.remove_label(l);
        }
        if let Some(label) = &label {
            storage.add_label(Label::new(label), id)?;
        }
        self.persist_now()?;
        self.update_meta(|meta| match label {
            Some(label) => {
                meta.labels.insert(account_id.to_owned(), label);
            }
            None => {
                meta.labels.remove(account_id);
            }
        })
    }

    // -- sync and network ----------------------------------------------------

    /// Sync this zone to the sequencer's latest block (connects if needed).
    ///
    /// On a network failure the session goes offline with exponential
    /// backoff; calls during the backoff fail fast with [`Offline`] until
    /// [`Session::retry_now`] or the wait is over.
    pub async fn sync(&mut self, observer: &mut dyn SyncObserver) -> Result<u64> {
        if let Net::Offline {
            retry_at, error, ..
        } = &self.net
            && let Some(left) = retry_at.checked_duration_since(Instant::now())
        {
            return Err(Offline {
                retry_in: left,
                reason: error.clone(),
            }
            .into());
        }
        match self.sync_inner(observer).await {
            Ok(tip) => {
                self.net = Net::Online { tip };
                Ok(tip)
            }
            // Local failures (disk, bookkeeping) are not an outage.
            Err(Failure::Local(e)) => Err(e),
            Err(Failure::Network(e)) => {
                let attempts = match &self.net {
                    Net::Offline { attempts, .. } => attempts + 1,
                    _ => 1,
                };
                self.net = Net::Offline {
                    attempts,
                    retry_at: Instant::now() + backoff(attempts),
                    error: format!("{e:#}"),
                };
                Err(e)
            }
        }
    }

    /// The user asked to retry: end the backoff wait (attempts are kept).
    pub fn retry_now(&mut self) {
        if let Net::Offline { retry_at, .. } = &mut self.net {
            *retry_at = Instant::now();
        }
    }

    async fn sync_inner(&mut self, observer: &mut dyn SyncObserver) -> Result<u64, Failure> {
        use Failure::{Local, Network};
        self.connect().await.map_err(Network)?;
        let birthday = self.meta.birthday_ms;
        let core = self.core_mut().context("not connected").map_err(Local)?;
        if core.storage().last_synced_block() == 0
            && let Some(birthday) = birthday
        {
            let first = first_block_at_or_after(core, birthday.saturating_sub(BIRTHDAY_MARGIN_MS))
                .await
                .map_err(Network)?;
            core.storage_mut()
                .set_last_synced_block(first.saturating_sub(1));
        }
        let result = core.sync_to_latest_block_with_observer(observer).await;
        // Persist whatever was synced, even if sync stopped part-way.
        self.backend.flush().map_err(Local)?;
        let tip = result.map_err(Network)?;
        if self.discovering() {
            self.finish_discovery().await.map_err(Network)?;
        }
        Ok(tip)
    }

    /// Drop the discovery tree's unused accounts (public: never touched on
    /// chain; private: no note found), then remember the counts.
    async fn finish_discovery(&mut self) -> Result<()> {
        let core = self.core_mut().context("not connected")?;
        let client = core.client_owned();
        core.storage_mut()
            .key_chain_mut()
            .cleanup_trees_remove_uninit_layered(DISCOVERY_DEPTH, |id| {
                let client = &client;
                async move { client.get_account(id).await.map_err(anyhow::Error::from) }
            })
            .await?;
        // Cleanup keeps a prefix of the layered order, so topping up to the
        // wallet's known counts rebuilds exactly the other zones' accounts.
        let want = self.meta.accounts;
        let have = layered_counts(self.storage()?);
        let keys = self.storage_mut()?.key_chain_mut();
        for _ in have.public..want.public {
            keys.generate_new_public_transaction_private_key(None);
        }
        for _ in have.private..want.private {
            keys.generate_new_privacy_preserving_transaction_key_chain(None);
        }
        self.persist_now()?;
        let counts = layered_counts(self.storage()?);
        let zone = self.zone.id.clone();
        self.update_meta(|meta| {
            meta.accounts.public = meta.accounts.public.max(counts.public);
            meta.accounts.private = meta.accounts.private.max(counts.private);
            meta.discovered.insert(zone);
        })
    }

    pub fn status(&self) -> Result<ZoneStatus> {
        let network = match &self.net {
            Net::Idle => NetStatus::Idle,
            Net::Online { tip } => NetStatus::Online { tip: *tip },
            Net::Offline {
                attempts,
                retry_at,
                error,
            } => NetStatus::Offline {
                attempts: *attempts,
                retry_in_ms: retry_at
                    .checked_duration_since(Instant::now())
                    .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
                error: error.clone(),
            },
        };
        Ok(ZoneStatus {
            zone: self.zone.clone(),
            network,
            synced_block: self.storage()?.last_synced_block(),
            discovering: self.discovering(),
            auto_lock_secs: self.meta.auto_lock_secs,
        })
    }

    // -- security settings ---------------------------------------------------

    pub const fn auto_lock(&self) -> Duration {
        Duration::from_secs(self.meta.auto_lock_secs as u64)
    }

    /// Lock after this long without use (1 minute to 24 hours).
    pub fn set_auto_lock(&mut self, secs: u32) -> Result<()> {
        ensure!(
            AUTO_LOCK_BOUNDS_SECS.contains(&secs),
            "auto-lock must be between 1 minute and 24 hours"
        );
        self.update_meta(|meta| meta.auto_lock_secs = secs)
    }

    /// The recovery phrase, after re-entering the password. Wrong passwords
    /// are throttled (see `RevealThrottle`).
    pub fn reveal_phrase(&mut self, password: &str) -> Result<Zeroizing<String>> {
        self.verify_current(password)?;
        let record = KeysRecord::parse(&self.keys.read()?)?;
        Ok(Zeroizing::new(record.secrets.phrase.clone()))
    }

    /// New password. Only the keys vault is rewritten (zone vaults are keyed).
    pub fn change_password(&mut self, current: &str, new: &str) -> Result<()> {
        ensure!(!new.is_empty(), "the new password is empty");
        self.verify_current(current)?;
        let bytes = self.keys.read()?;
        self.keys.change_password(current, new, &bytes)
    }

    /// Re-auth for an approval (first connect, private spend, large amount).
    /// Shares the throttle with phrase reveal and password change.
    pub fn reauth(&mut self, password: &str) -> Result<()> {
        self.verify_current(password)
    }

    pub fn grants(&self) -> &[crate::policy::Grant] {
        &self.meta.grants
    }

    /// Replace the grant list (stored encrypted in the keys vault).
    pub fn set_grants(&mut self, grants: Vec<crate::policy::Grant>) -> Result<()> {
        self.update_meta(|meta| meta.grants = grants)
    }

    /// Token definitions this wallet tracks in the current zone.
    pub fn tracked_tokens(&self) -> Vec<String> {
        self.meta
            .tokens
            .get(&self.zone().id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn track_token(&mut self, definition: &str) -> Result<()> {
        let zone = self.zone().id.clone();
        if self.tracked_tokens().iter().any(|t| t == definition) {
            return Ok(());
        }
        let definition = definition.to_owned();
        self.update_meta(|meta| meta.tokens.entry(zone).or_default().push(definition))
    }

    /// Stop treating `definition` as added by the user (it falls back to
    /// Unknown, or Verified if it's on the list).
    pub fn untrack_token(&mut self, definition: &str) -> Result<()> {
        let zone = self.zone().id.clone();
        self.update_meta(|m| {
            if let Some(list) = m.tokens.get_mut(&zone) {
                list.retain(|t| t != definition);
            }
        })
    }

    /// Token definitions the user hid on this zone.
    pub fn hidden_tokens(&self) -> Vec<String> {
        self.meta
            .hidden_tokens
            .get(&self.zone().id)
            .cloned()
            .unwrap_or_default()
    }

    /// Token definitions the user pinned on this zone.
    pub fn pinned_tokens(&self) -> Vec<String> {
        self.meta
            .pinned_tokens
            .get(&self.zone().id)
            .cloned()
            .unwrap_or_default()
    }

    /// Hide (or show again) a token on this zone. Hiding never deletes
    /// anything; showing never makes a token trusted.
    pub fn set_token_hidden(&mut self, definition: &str, hidden: bool) -> Result<()> {
        crate::decode::account_id(definition)?;
        let zone = self.zone().id.clone();
        let definition = definition.to_owned();
        self.update_meta(|m| {
            let list = m.hidden_tokens.entry(zone).or_default();
            list.retain(|t| *t != definition);
            if hidden {
                list.push(definition);
            }
        })
    }

    pub fn set_token_pinned(&mut self, definition: &str, pinned: bool) -> Result<()> {
        crate::decode::account_id(definition)?;
        let zone = self.zone().id.clone();
        let definition = definition.to_owned();
        self.update_meta(|m| {
            let list = m.pinned_tokens.entry(zone).or_default();
            list.retain(|t| *t != definition);
            if pinned {
                list.push(definition);
            }
        })
    }

    /// Display decimals the user set for a token (LEZ stores none).
    pub fn token_decimals(&self, definition: &str) -> Option<u8> {
        self.meta
            .token_decimals
            .get(&self.zone().id)
            .and_then(|m| m.get(definition))
            .copied()
    }

    pub fn set_token_decimals(&mut self, definition: &str, decimals: Option<u8>) -> Result<()> {
        crate::decode::account_id(definition)?;
        ensure!(decimals.is_none_or(|d| d <= 36), "decimals are 0–36");
        let zone = self.zone().id.clone();
        let definition = definition.to_owned();
        self.update_meta(|m| {
            let map = m.token_decimals.entry(zone).or_default();
            match decimals {
                Some(d) => {
                    map.insert(definition, d);
                }
                None => {
                    map.remove(&definition);
                }
            }
        })
    }

    /// Re-auth for sensitive actions, throttled so an unlocked wallet can't be
    /// used to guess its own password.
    /// Checks the password without this session (see `vault::PasswordCheck`).
    pub fn password_checker(&self) -> Result<crate::vault::PasswordCheck> {
        self.keys.checker()
    }

    /// The wallet's secret for per-app private account handles: random,
    /// stored encrypted with the keys, so handles can't be computed outside
    /// the wallet.
    pub fn handle_key(&mut self) -> Result<[u8; 32]> {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD;
        if let Some(k) = &self.meta.handle_key
            && let Ok(bytes) = b64.decode(k)
            && let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice())
        {
            return Ok(key);
        }
        let key = *crate::vault::new_key();
        let encoded = b64.encode(key);
        self.update_meta(|m| m.handle_key = Some(encoded))?;
        Ok(key)
    }

    fn verify_current(&mut self, password: &str) -> Result<()> {
        self.reveal.check()?;
        if !self.keys.verify_password(password)? {
            self.reveal.fail();
            bail!("wrong password");
        }
        self.reveal = RevealThrottle::default();
        Ok(())
    }

    fn update_meta(&mut self, f: impl FnOnce(&mut Meta)) -> Result<()> {
        let mut record = KeysRecord::parse(&self.keys.read()?)?;
        f(&mut record.meta);
        self.keys.save(&record.to_bytes()?)?;
        self.meta = record.meta.clone();
        Ok(())
    }

    /// This zone's activity record, as `save_history` wrote it (`Null` if
    /// unreadable).
    /// Fails when there's no readable record; the caller must then not
    /// overwrite it.
    pub fn load_history(&self) -> Result<serde_json::Value> {
        let vault = self
            .history
            .as_ref()
            .context("activity history unavailable")?;
        let bytes = vault.read()?;
        serde_json::from_slice(&bytes).context("activity history doesn't parse")
    }

    pub fn save_history(&self, record: &serde_json::Value) -> Result<()> {
        match &self.history {
            Some(vault) => vault.save(&serde_json::to_vec(record)?),
            None => Ok(()),
        }
    }

    /// Restored from a phrase: past activity is on chain to be found.
    pub const fn is_restored(&self) -> bool {
        self.meta.restored
    }

    /// From when this wallet's activity can matter (unix ms): its creation,
    /// or a restored wallet's "first used" date; `None` = from genesis.
    pub const fn activity_since_ms(&self) -> Option<u64> {
        if self.meta.restored {
            self.meta.birthday_ms
        } else {
            Some(self.meta.created_at_ms)
        }
    }

    // -- persistence ---------------------------------------------------------

    /// Write the current storage to the vault now (not debounced).
    pub fn persist_now(&self) -> Result<()> {
        self.storage()?
            .save_to(&Shared(Arc::clone(&self.backend)))?;
        self.backend.flush()
    }

    /// Flush and drop the session (keys leave memory with it).
    pub fn lock(self) -> Result<()> {
        self.backend.flush()
    }
}

/// `(account id, kind, path)` for every account in the key chain.
fn public_and_private(storage: &Storage) -> Vec<(String, AccountKind, Option<String>)> {
    let keys = storage.key_chain();
    let public = keys.public_account_ids().map(|(id, path)| {
        (
            id.to_string(),
            AccountKind::Public,
            path.map(ToString::to_string),
        )
    });
    let private = keys.private_account_ids().map(|(id, path)| {
        (
            id.to_string(),
            AccountKind::Private,
            path.map(ToString::to_string),
        )
    });
    public.chain(private).collect()
}

/// The lowest block whose timestamp is at or after `ms` (unix ms), by binary
/// search over block headers. Returns the tip when every block is older, and
/// 0 (scan everything) if a block in the range is missing.
async fn first_block_at_or_after(core: &WalletCore, ms: u64) -> Result<u64> {
    let tip = core.get_last_block_id().await?;
    let timestamp = |id: u64| async move {
        Ok::<_, anyhow::Error>(core.get_block(id).await?.map(|b| b.header.timestamp))
    };
    match timestamp(tip).await? {
        Some(ts) if ts < ms => return Ok(tip),
        None => return Ok(0),
        _ => {}
    }
    let (mut lo, mut hi) = (1, tip);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        match timestamp(mid).await? {
            Some(ts) if ts < ms => lo = mid + 1,
            Some(_) => hi = mid,
            None => return Ok(0),
        }
    }
    Ok(lo)
}

/// The zone's activity vault (`<zone>/history`), sealed with the zone key
/// under its own context. History is a convenience: a vault that won't open
/// is moved aside (kept, never deleted) and a new one starts.
fn open_history(dir: &Path, context: &str, key: Option<Zeroizing<[u8; 32]>>) -> Option<Vault> {
    let hdir = dir.join("history");
    let context = format!("{context}:history");
    // The zone vault was just opened or created with this key.
    let key = key?;
    if Vault::exists(&hdir) {
        match Vault::unlock_keyed(&hdir, key.clone(), &context) {
            Ok((vault, _)) => return Some(vault),
            // A file that's there but won't open (damaged, or another
            // key): kept aside, never deleted, and a new history starts. If
            // it can't even be moved, this session keeps history in memory.
            Err(_) => {
                std::fs::rename(&hdir, dir.join(format!("history.unreadable-{}", now_ms()))).ok()?
            }
        }
    }
    Vault::create_keyed(&hdir, key, &context, br#"{"scanned":0,"items":[]}"#).ok()
}
