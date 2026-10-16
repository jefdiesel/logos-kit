//! The engine a host embeds: the unlocked wallet (behind auto-lock), the one
//! pending request, and transaction statuses. Every entry point takes the
//! [`Caller`] the host observed and applies the policy in `policy.rs`.
//!
//! Approval hardening (all `ui_qml` code shares one process, so a module name
//! alone could be impersonated):
//! - only one request is pending at a time; a second one is refused;
//! - approving echoes the request hash, and a mismatch cancels the request;
//! - the pending request is taken out atomically, so it is used at most once;
//! - private spends, large amounts and connects need the password again;
//! - it expires at its deadline, on a zone switch, lock, or restart (memory only).
//!
//! Proving runs on a blocking worker *without* the wallet lock, so status
//! reads (and the auto-lock timer) keep working for the minutes it takes.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::aead::{OsRng, rand_core::RngCore as _};
use serde::{Deserialize, Serialize};

use crate::{
    auto_lock::AutoLock,
    faucet::{FaucetBackend, FundOutcome},
    policy::{self, Caller, Capability, Code, Denied, Grant},
    session::AccountKind,
    session::Session,
    tx::{self, Intent, Prepared, Review},
};

/// How long a request waits for the user.
pub const REQUEST_TTL: Duration = Duration::from_secs(5 * 60);
/// After an app's request is declined or expires, it waits this long before
/// it can ask again (so it can't keep the one pending slot busy).
pub const APP_COOLDOWN: Duration = Duration::from_secs(30);
/// Statuses kept for reads; the oldest finished ones go first.
const MAX_STATUSES: usize = 640;
/// Finished statuses kept on disk per zone (the activity history).
const MAX_HISTORY: usize = 500;
/// How long after landing our transfer into a private account may still
/// explain a rise in its balance (private syncs lag inclusion).
const OWN_CREDIT_WINDOW_MS: u64 = 30 * 60 * 1000;

/// A finished status as the history vault stores it: the API form plus who
/// asked (never sent to apps, so `TxStatus` skips it).
#[derive(Serialize, Deserialize)]
struct StoredStatus {
    #[serde(flatten)]
    status: TxStatus,
    #[serde(default)]
    requester: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    AwaitingApproval,
    Building,
    Proving,
    Signing,
    Submitted,
    Included,
    Rejected,
    Dropped,
    Expired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Success,
    Failure,
    Unknown,
}

/// Where `outcome` comes from. Only an event or a check on our own account
/// counts; otherwise the outcome stays unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutcomeSource {
    OwnAccountInvariant,
    None,
}

/// `lez_getTransactionStatus` result (plus `error` for the wallet's own UI).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TxStatus {
    pub handle: String,
    pub chain: String,
    pub lifecycle: Lifecycle,
    pub outcome: Outcome,
    pub outcome_source: OutcomeSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<u64>,
    /// When the current phase began (unix ms), for elapsed timers.
    pub phase_started_ms: u64,
    /// Why it stopped (the wallet's own UI; apps get only `errorCode`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<i64>,
    /// One line for activity rows ("Send 12.5 LEZ to …"); owner views only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// How a transfer travels (private routes prove for minutes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<tx::Route>,
    /// The wallet account it spends from or signs with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// For a transfer: its amount (base units), recipient and token (none =
    /// native). Lets the UI show real activity amounts and private buckets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The native fee a public send paid (lepta), read from the sender's
    /// balance: the 0.3 RPC reports no gas used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_paid: Option<String>,
    /// A payment someone else made to us, found in a block (owner only).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub incoming: bool,
    /// The wallet zone it belongs to (several zones can share a chain id);
    /// its history is stored with that zone.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub zone: String,
    #[serde(skip)]
    requester: Option<String>,
}

impl TxStatus {
    fn new(handle: &str, chain: &str, requester: Option<String>) -> Self {
        Self {
            handle: handle.to_owned(),
            chain: chain.to_owned(),
            lifecycle: Lifecycle::AwaitingApproval,
            outcome: Outcome::Unknown,
            outcome_source: OutcomeSource::None,
            tx_hash: None,
            block: None,
            phase_started_ms: now_ms(),
            error: None,
            error_code: None,
            title: None,
            route: None,
            from: None,
            amount: None,
            to: None,
            token: None,
            fee_paid: None,
            incoming: false,
            zone: String::new(),
            requester,
        }
    }

    /// The app that asked (None: the owner).
    pub fn requester(&self) -> Option<&str> {
        self.requester.as_deref()
    }

    pub const fn is_final(&self) -> bool {
        !matches!(
            self.lifecycle,
            Lifecycle::AwaitingApproval
                | Lifecycle::Building
                | Lifecycle::Proving
                | Lifecycle::Signing
                | Lifecycle::Submitted
        )
    }
}

/// `request_funds` result.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Funds {
    #[serde(flatten)]
    pub outcome: FundOutcome,
    /// Which backend paid (shown in the UI).
    pub faucet: String,
    /// The public account the faucet paid.
    pub funded_account: String,
    /// For a private target: the shield waiting for approval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shield: Option<Ticket>,
    /// Funded, but the shield couldn't be queued (why).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shield_error: Option<String>,
}

/// What `request_*` hands the approving UI.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ticket {
    pub handle: String,
    pub request: RequestView,
    /// The approval must carry the password (private route, large amount,
    /// token authority, connect).
    pub needs_password: bool,
    /// The wallet can't decode what this does: approving must acknowledge that.
    pub needs_acknowledgement: bool,
    pub expires_in_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RequestView {
    Transaction(Box<Review>),
    #[serde(rename_all = "camelCase")]
    Connect {
        requester: String,
        chain: String,
        accounts: Vec<String>,
        capabilities: Vec<Capability>,
        request_hash: String,
    },
}

enum Kind {
    Tx(Box<Prepared>),
    Connect {
        requester: String,
        accounts: Vec<String>,
        capabilities: Vec<Capability>,
    },
}

struct Pending {
    handle: String,
    requester: Option<String>,
    kind: Kind,
    hash: [u8; 32],
    needs_password: bool,
    needs_ack: bool,
    deadline: Instant,
    /// The wallet/zone generation it was built in, and its zone.
    epoch: u64,
    zone: String,
    /// What the approving UI was shown (re-read after a restart of the view).
    ticket: Ticket,
}

#[derive(Default)]
struct State {
    pending: Option<Pending>,
    statuses: HashMap<String, TxStatus>,
    /// Handle of the transaction being proved (one prover slot).
    proving: Option<String>,
    cancelled: Vec<String>,
    /// Bumped on lock, zone switch and unlock: requests from an older
    /// generation are never approved or stored.
    epoch: u64,
    /// App → when it may ask again.
    cooldown: HashMap<String, Instant>,
    /// A status changed since the history was last written.
    history_dirty: bool,
    /// How far the incoming scan has read (block 0: never).
    cursor: chain_index::Cursor,
    /// Tokens that landed in our accounts or their token accounts, by
    /// definition (what the token list calls "Unknown" until added).
    seen_tokens: std::collections::BTreeMap<String, SeenToken>,
    /// Blocks the scan has yet to read after its last step.
    scan_behind: u64,
    /// The current session's chain and zone (set when its history loads).
    chain: Option<String>,
    zone_id: String,
    /// The current zone's history couldn't be read: don't overwrite it.
    history_unreadable: bool,
    /// Transactions the faucet made for us, so the incoming scan labels them.
    faucet_hashes: std::collections::HashSet<String>,
    /// Our transfers into private accounts that a balance rise has already
    /// accounted for (`record_private_receipt`).
    credited: std::collections::HashSet<String>,
}

/// A token found in one of our accounts by the incoming scan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeenToken {
    pub first_block: u64,
    /// Our account it arrived for.
    pub account: String,
    /// Its token account (ATA), when it landed there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    /// Who sent it first (`None`: from a private account).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// A transaction hash compared across sources (drip answer, block).
fn hash_key(h: &str) -> String {
    h.trim().trim_start_matches("0x").to_ascii_lowercase()
}

impl State {
    fn insert_status(&mut self, status: TxStatus) {
        if self.statuses.len() >= MAX_STATUSES {
            let mut done: Vec<(u64, String)> = self
                .statuses
                .values()
                .filter(|s| s.is_final())
                .map(|s| (s.phase_started_ms, s.handle.clone()))
                .collect();
            done.sort();
            for (_, h) in done.iter().take(self.statuses.len() + 1 - MAX_STATUSES) {
                self.statuses.remove(h);
            }
        }
        let mut status = status;
        if status.zone.is_empty() {
            status.zone.clone_from(&self.zone_id);
        }
        self.statuses.insert(status.handle.clone(), status);
        self.history_dirty = true;
    }

    /// The history record for `chain`: the scan position and the finished
    /// statuses, newest first.
    fn history(&self, zone: &str) -> serde_json::Value {
        serde_json::json!({
            "cursor": self.cursor,
            "tokens": self.seen_tokens,
            "items": self.history_items(zone),
        })
    }

    fn history_items(&self, zone: &str) -> Vec<serde_json::Value> {
        // Finished rows, plus ones in flight past approval, so a restart can
        // say what happened to them (a request awaiting approval never
        // outlives the session).
        let mut done: Vec<&TxStatus> = self
            .statuses
            .values()
            .filter(|s| s.lifecycle != Lifecycle::AwaitingApproval && s.zone == zone)
            .collect();
        done.sort_by_key(|s| std::cmp::Reverse(s.phase_started_ms));
        done.into_iter()
            .take(MAX_HISTORY)
            .filter_map(|s| {
                serde_json::to_value(StoredStatus {
                    status: s.clone(),
                    requester: s.requester.clone(),
                })
                .ok()
            })
            .collect()
    }

    /// Bring back `session`'s history. Finished statuses of other zones leave
    /// memory (they're on disk); anything still running stays.
    fn load_history(&mut self, session: &Session) {
        let chain = session.zone().chain.clone();
        let zone = session.zone().id.clone();
        self.chain = Some(chain);
        self.zone_id.clone_from(&zone);
        // Rows still in flight for another zone stay (they're written to
        // their own zone when it is current again).
        self.statuses.retain(|_, s| !s.is_final() || s.zone == zone);
        let record = if let Ok(r) = session.load_history() {
            self.history_unreadable = false;
            r
        } else {
            // Keep what's on disk; this session's activity stays in memory.
            self.history_unreadable = true;
            serde_json::Value::Null
        };
        self.cursor = record
            .get("cursor")
            .and_then(|c| serde_json::from_value(c.clone()).ok())
            // Records from before chain-index kept only the block number.
            .or_else(|| {
                record
                    .get("scanned")
                    .and_then(serde_json::Value::as_u64)
                    .map(|b| chain_index::Cursor::at(b, None))
            })
            .unwrap_or_default();
        self.seen_tokens = record
            .get("tokens")
            .and_then(|t| serde_json::from_value(t.clone()).ok())
            .unwrap_or_default();
        let items = record
            .get("items")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for v in items {
            if let Ok(stored) = serde_json::from_value::<StoredStatus>(v) {
                let mut status = stored.status;
                status.requester = stored.requester;
                if status.zone.is_empty() {
                    status.zone.clone_from(&zone);
                }
                // In flight when the wallet closed (a quit, a crash, or the
                // OS stopping it for memory while it proved).
                match status.lifecycle {
                    Lifecycle::Building | Lifecycle::Proving => {
                        status.lifecycle = Lifecycle::Dropped;
                        status.error_code = Some(Code::ProofFailed as i64);
                        status.error = Some(
                            "The proof stopped when the wallet closed. Nothing was sent."
                                .to_owned(),
                        );
                    }
                    Lifecycle::Signing | Lifecycle::Submitted => {
                        status.lifecycle = Lifecycle::Expired;
                        status.error = Some(
                            "The wallet closed before it saw the result. It may still have landed: check the balance or the explorer."
                                .to_owned(),
                        );
                    }
                    _ => {}
                }
                self.statuses.entry(status.handle.clone()).or_insert(status);
            }
        }
        self.history_dirty = false;
    }
}

/// Holds the one prover slot; released on every exit path, including the
/// approve future being dropped mid-proof.
struct ProvingSlot<'a> {
    engine: &'a Engine,
    handle: String,
}

impl Drop for ProvingSlot<'_> {
    fn drop(&mut self) {
        let mut state = self.engine.state();
        if state.proving.as_deref() == Some(self.handle.as_str()) {
            state.proving = None;
        }
        state.cancelled.retain(|h| *h != self.handle);
    }
}

pub struct Config {
    /// Amounts at or above this need the password again (base units).
    pub reauth_at: u128,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            reauth_at: 1_000_000,
        }
    }
}

pub struct Engine {
    wallet: tokio::sync::Mutex<AutoLock>,
    /// The user acted while something held the wallet: applied at the next tick.
    touched: std::sync::atomic::AtomicBool,
    state: Mutex<State>,
    config: Config,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn new_handle() -> String {
    let mut bytes = [0_u8; 18];
    OsRng.fill_bytes(&mut bytes);
    format!("lk_{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// Constant-time equality for the echoed request hash.
fn same_hash(echoed: &str, expected: &[u8; 32]) -> bool {
    let Ok(bytes) = hex::decode(echoed.trim_start_matches("0x")) else {
        return false;
    };
    bytes.len() == 32
        && bytes
            .iter()
            .zip(expected)
            .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

fn tx_hash_hex(hash: &str) -> String {
    let hex = hash.trim_start_matches("0x").to_ascii_lowercase();
    format!("0x{hex}")
}

impl Engine {
    pub fn new(session: Session, config: Config) -> Self {
        let mut state = State::default();
        state.load_history(&session);
        Self {
            wallet: tokio::sync::Mutex::new(AutoLock::new(session)),
            touched: std::sync::atomic::AtomicBool::new(false),
            state: Mutex::new(state),
            config,
        }
    }

    /// Write the zone's finished activity if anything changed. Called by the
    /// host's tick, on lock and on zone switch; the CLI calls it before exit.
    pub async fn flush_history(&self) -> Result<()> {
        let mut wallet = self.wallet.lock().await;
        let Some(session) = wallet.peek() else {
            return Ok(());
        };
        Self::flush_into(&self.state, session)
    }

    fn flush_into(state: &Mutex<State>, session: &Session) -> Result<()> {
        let record = {
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !state.history_dirty || state.history_unreadable {
                return Ok(());
            }
            state.history_dirty = false;
            state.history(&session.zone().id)
        };
        session.save_history(&record)
    }

    /// A private account's LGO balance rose by `delta` since the last snapshot.
    /// Whatever our own sends and shields into it explain is ours; the rest
    /// arrived from someone else, privately (no sender, no transaction we
    /// can name). Our transfers still in flight count as ours, so a shield
    /// that lands just before its status updates isn't mistaken for a gift.
    pub fn record_private_receipt(
        &self,
        account: &str,
        delta: u128,
        block: Option<u64>,
        zone: &str,
    ) {
        let mut state = self.state();
        // Snapshots straddling a zone switch say nothing.
        if state.zone_id != zone {
            return;
        }
        // Ours: sent and maybe landed (submitted), or landed recently and not
        // yet matched to a rise. Private balances can lag their transfer's
        // inclusion by a sync or two, so landing time alone can't tell.
        let now = now_ms();
        let state = &mut *state;
        state.credited.retain(|h| state.statuses.contains_key(h));
        let mut ours: Vec<(bool, u64, String, u128)> = state
            .statuses
            .values()
            .filter(|s| {
                !s.incoming
                    && s.zone == zone
                    && s.to.as_deref() == Some(account)
                    && s.token.is_none()
                    && !state.credited.contains(&s.handle)
            })
            .filter(|s| {
                s.lifecycle == Lifecycle::Submitted
                    || (s.lifecycle == Lifecycle::Included
                        && now.saturating_sub(s.phase_started_ms) < OWN_CREDIT_WINDOW_MS)
            })
            .filter_map(|s| {
                let amount = s.amount.as_deref()?.parse::<u128>().ok()?;
                Some((
                    s.lifecycle != Lifecycle::Included,
                    s.phase_started_ms,
                    s.handle.clone(),
                    amount,
                ))
            })
            .collect();
        // Landed first, oldest first. A landed one explains the rise even
        // when the account also paid something out meanwhile (the rise is
        // net); one still in flight only when it fits whole.
        ours.sort();
        let mut rest = delta;
        for (in_flight, _, handle, amount) in ours {
            if rest == 0 {
                break;
            }
            if !in_flight || amount <= rest {
                rest = rest.saturating_sub(amount);
                state.credited.insert(handle);
            }
        }
        if rest == 0 {
            return;
        }
        let Some(chain) = state.chain.clone() else {
            return;
        };
        let mut status = TxStatus::new(&format!("in:private:{account}:{now}"), &chain, None);
        status.lifecycle = Lifecycle::Included;
        status.outcome = Outcome::Success;
        status.block = block;
        status.phase_started_ms = now;
        status.title = Some("Received privately".to_owned());
        status.to = Some(account.to_owned());
        status.amount = Some(rest.to_string());
        status.route = Some(tx::Route::Private);
        status.incoming = true;
        state.insert_status(status);
    }

    /// Read the blocks since the last scan for payments into our public
    /// accounts and add them to the activity (see `incoming`). Network reads
    /// happen without the wallet lock. A new wallet starts at today's tip; a
    /// restored one reads from block 1, `incoming::STEP` blocks per call.
    pub async fn scan_incoming(&self) -> Result<usize> {
        let (client, mine, decoders, chain, since_ms, epoch) = {
            let mut wallet = self.wallet.lock().await;
            let s = wallet.session()?;
            let Some(core) = s.core() else {
                return Ok(0);
            };
            let mine: std::collections::HashSet<lee::AccountId> = s
                .accounts()?
                .iter()
                .filter(|a| a.kind == AccountKind::Public)
                .filter_map(|a| crate::decode::account_id(&a.account_id).ok())
                .collect();
            (
                core.client_owned(),
                mine,
                s.decoders(),
                s.zone().chain.clone(),
                s.activity_since_ms(),
                self.state().epoch,
            )
        };
        // Where to read from; a wallet that has never read starts at the
        // first block after it was made (a minute's slack for clocks).
        let (current, fresh) = {
            let state = self.state();
            if state.epoch != epoch {
                return Ok(0);
            }
            (state.cursor.clone(), state.cursor.block == 0)
        };
        let cursor = match (fresh, since_ms) {
            (true, Some(ms)) => {
                let start = chain_index::Scanner::new(&client)
                    .cursor_before_time(ms.saturating_sub(60_000))
                    .await?;
                let mut state = self.state();
                if state.epoch != epoch || state.cursor.block != 0 {
                    return Ok(0);
                }
                state.cursor = start.clone();
                state.history_dirty = true;
                start
            }
            _ => current,
        };
        if mine.is_empty() {
            return Ok(0);
        }
        let (found, next, step) = crate::incoming::scan(&client, &cursor, &mine, &decoders).await?;
        let mut state = self.state();
        if state.epoch != epoch || state.cursor != cursor {
            return Ok(0);
        }
        if matches!(step, chain_index::Step::Reset { .. }) {
            // A reset chain: what we read from the old one is gone (its
            // tokens and payments to us); read the new one from its start.
            let zone_id = state.zone_id.clone();
            state.seen_tokens.clear();
            state
                .statuses
                .retain(|_, s| !(s.incoming && s.zone == zone_id));
            state.cursor = chain_index::Cursor::default();
            state.history_dirty = true;
            return Ok(0);
        }
        state.scan_behind = step.behind();
        let mut changed = next != state.cursor;
        let mut added = 0;
        for (i, inc) in found.into_iter().enumerate() {
            if let Some(def) = &inc.token
                && !state.seen_tokens.contains_key(def)
            {
                state.seen_tokens.insert(
                    def.clone(),
                    SeenToken {
                        first_block: inc.block,
                        account: inc.account.clone(),
                        holder: inc.holder.clone(),
                        from: inc.from.clone(),
                    },
                );
                changed = true;
            }
            // Our own send, or this payment found before (a transaction
            // paying two of our accounts makes two rows).
            let key = hash_key(&inc.tx_hash);
            let known = state.statuses.values().any(|s| {
                s.tx_hash.as_deref().map(hash_key).as_deref() == Some(key.as_str())
                    && (!s.incoming
                        || (s.to.as_deref() == Some(inc.account.as_str()) && s.token == inc.token))
            });
            if known {
                continue;
            }
            let from_faucet = state.faucet_hashes.contains(&key);
            let handle = format!("in:{}:{i}", inc.tx_hash);
            let mut status = TxStatus::new(&handle, &chain, None);
            status.lifecycle = Lifecycle::Included;
            status.outcome = Outcome::Success;
            status.tx_hash = Some(inc.tx_hash);
            status.block = Some(inc.block);
            status.phase_started_ms = inc.timestamp_ms;
            status.title = Some(
                if from_faucet {
                    "Test LGO from the faucet"
                } else {
                    "Received"
                }
                .to_owned(),
            );
            status.from = inc.from;
            status.to = Some(inc.account);
            status.amount = Some(inc.amount);
            status.token = inc.token;
            status.incoming = true;
            state.insert_status(status);
            added += 1;
        }
        state.cursor = next;
        // Nothing new: don't rewrite the history vault on every sync.
        if changed || added > 0 {
            state.history_dirty = true;
        }
        Ok(added)
    }

    /// Tokens that landed in this zone's accounts (or their token accounts)
    /// since the wallet started reading, by definition.
    pub fn seen_tokens(&self) -> std::collections::BTreeMap<String, SeenToken> {
        self.state().seen_tokens.clone()
    }

    /// Blocks the incoming scan still has to read (0: caught up).
    pub fn scan_behind(&self) -> u64 {
        self.state().scan_behind
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // A panic while holding it leaves plain data; keep serving.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Run `f` with the unlocked session (fails with `Locked` otherwise).
    pub async fn with_session<T>(
        &self,
        f: impl AsyncFnOnce(&mut Session) -> Result<T>,
    ) -> Result<T> {
        let mut wallet = self.wallet.lock().await;
        f(wallet.session()?).await
    }

    /// Lock now; the pending request expires with it.
    pub async fn lock(&self) -> Result<()> {
        self.expire_pending("wallet locked");
        let mut wallet = self.wallet.lock().await;
        if let Some(session) = wallet.peek() {
            let _ = Self::flush_into(&self.state, session);
        }
        wallet.lock()
    }

    /// Host timer: auto-lock when idle (also expires the pending request).
    /// Not while a proof runs: the user is waiting on it, and locking would
    /// throw the proof away.
    pub async fn tick(&self) -> Result<bool> {
        let mut wallet = self.wallet.lock().await;
        // Write the history first, also while a proof runs: if the OS stops
        // the host mid-proof, the next start can say what happened to it.
        if let Some(session) = wallet.peek() {
            let _ = Self::flush_into(&self.state, session);
        }
        {
            // Not while a proof runs or a transaction is being sent: the
            // user is waiting on it, and locking would throw it away.
            let state = self.state();
            let busy = state.proving.is_some()
                || state
                    .statuses
                    .values()
                    .any(|s| matches!(s.lifecycle, Lifecycle::Signing | Lifecycle::Submitted));
            if busy {
                return Ok(false);
            }
        }
        if self
            .touched
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            wallet.touch();
        }
        let locked = wallet.tick()?;
        drop(wallet);
        if locked {
            self.expire_pending("wallet locked");
        }
        Ok(locked)
    }

    /// Switch to another unlocked session (zone switch, unlock after lock).
    pub async fn set_session(&self, session: Session) -> Result<()> {
        self.expire_pending("wallet or zone changed");
        let mut wallet = self.wallet.lock().await;
        if let Some(old) = wallet.peek() {
            let _ = Self::flush_into(&self.state, old);
        }
        self.state().load_history(&session);
        wallet.set(session)
    }

    fn expire_pending(&self, why: &str) {
        let mut state = self.state();
        state.epoch += 1;
        if let Some(p) = state.pending.take() {
            Self::finish(&mut state, &p, Lifecycle::Expired, Some(why));
        }
    }

    /// End a request that never ran; an app's declined or expired request
    /// starts its cooldown.
    fn finish(state: &mut State, p: &Pending, lifecycle: Lifecycle, error: Option<&str>) {
        if let Some(s) = state.statuses.get_mut(&p.handle) {
            s.lifecycle = lifecycle;
            s.phase_started_ms = now_ms();
            s.error = error.map(str::to_owned);
        }
        state.history_dirty = true;
        if let Some(app) = &p.requester {
            state
                .cooldown
                .insert(app.clone(), Instant::now() + APP_COOLDOWN);
        }
    }

    /// Resolve who the request is for, or refuse the caller.
    fn requester_for(caller: &Caller, relayed: Option<&str>) -> Result<Option<String>> {
        match caller {
            // Our UI relays the shell-attested requester (None: the user themself).
            Caller::WalletUi => Ok(relayed.map(str::to_owned)),
            Caller::LocalOwner => Ok(None),
            Caller::Module(name) => Ok(Some(name.clone())),
            Caller::Host | Caller::Bridge | Caller::Unknown => Err(Denied::err(
                Code::Unauthorized,
                "this caller can't make wallet requests",
            )),
        }
    }

    /// Room for a new request from `requester` (None: the owner). An expired
    /// request is cleared; the owner's own request replaces an app's; an app
    /// in its cooldown waits.
    fn check_free(state: &mut State, requester: Option<&str>) -> Result<()> {
        if let Some(app) = requester
            && state.cooldown.get(app).is_some_and(|t| Instant::now() < *t)
        {
            return Err(Denied::err(
                Code::RequestPending,
                "your last request was just declined; try again shortly",
            ));
        }
        if let Some(p) = state.pending.take() {
            let (lifecycle, why) = if Instant::now() >= p.deadline {
                (Lifecycle::Expired, "not approved in time")
            } else if requester.is_none() && p.requester.is_some() {
                (Lifecycle::Expired, "replaced by the wallet owner's request")
            } else {
                state.pending = Some(p);
                return Err(Denied::err(
                    Code::RequestPending,
                    "a request is already open in your wallet",
                ));
            };
            Self::finish(state, &p, lifecycle, Some(why));
        }
        Ok(())
    }

    /// Refuse to store a request built under an older wallet generation.
    fn check_epoch(state: &State, epoch: u64) -> Result<()> {
        if state.epoch == epoch {
            Ok(())
        } else {
            Err(Denied::err(
                Code::StaleApproval,
                "the wallet was locked or switched zones while this was prepared",
            ))
        }
    }

    // -- requests --------------------------------------------------------------

    /// Ask to send a transaction. The wallet UI (or CLI) must then approve it.
    pub async fn request_tx(
        &self,
        caller: &Caller,
        relayed: Option<&str>,
        intent: Intent,
    ) -> Result<Ticket> {
        let requester = Self::requester_for(caller, relayed)?;
        let mut wallet = self.wallet.lock().await;
        let session = wallet.session()?;
        let zone = session.zone().id.clone();
        if let (Caller::Module(name), false) = (
            caller,
            policy::allows(
                session.grants(),
                &zone,
                requester.as_deref().unwrap_or(""),
                intent.from_account(),
                Capability::ProposeTx,
            ),
        ) {
            return Err(Denied::err(
                Code::Unauthorized,
                format!("{name} isn't connected to {}", intent.from_account()),
            ));
        }
        let epoch = {
            let mut state = self.state();
            Self::check_free(&mut state, requester.as_deref())?;
            state.epoch
        };
        let prepared = session.prepare(requester.as_deref(), intent).await?;
        // Every account that signs authorizes the program, not only `from`:
        // an app needs a grant on each (a Call can name any of ours).
        if let Caller::Module(name) = caller
            && let Some(ungranted) = prepared.review.summary.signers.iter().find(|a| {
                !policy::allows(
                    session.grants(),
                    &zone,
                    requester.as_deref().unwrap_or(""),
                    a,
                    Capability::ProposeTx,
                )
            })
        {
            return Err(Denied::err(
                Code::Unauthorized,
                format!("{name} isn't connected to {ungranted}, which this would sign with"),
            ));
        }
        drop(wallet);
        let review = &prepared.review;
        let token_out = review
            .summary
            .outflows
            .iter()
            .any(|f| f.asset != crate::decode::Asset::Native);
        let needs_password = review.route.is_some_and(tx::Route::is_private)
            || review.native_outflow() >= self.config.reauth_at
            || token_out
            || review.summary.unknown
            || !review.summary.authorities.is_empty();
        let needs_ack = review.summary.unknown;

        let handle = new_handle();
        let review = prepared.review.clone();
        let ticket = Ticket {
            handle: handle.clone(),
            request: RequestView::Transaction(Box::new(review.clone())),
            needs_password,
            needs_acknowledgement: needs_ack,
            expires_in_ms: u64::try_from(REQUEST_TTL.as_millis()).unwrap_or(u64::MAX),
        };
        let mut state = self.state();
        // Checked again: the wallet or the slot may have changed while we built.
        Self::check_epoch(&state, epoch)?;
        Self::check_free(&mut state, requester.as_deref())?;
        let mut status = TxStatus::new(&handle, &review.chain, requester.clone());
        status.title = Some(review.summary.title.clone());
        status.route = review.route;
        status.from = Some(review.intent.from_account().to_owned());
        if let tx::Intent::Transfer {
            to, amount, token, ..
        } = &review.intent
        {
            status.amount = Some(amount.to_string());
            status.to = to.clone();
            status.token = token.clone();
        } else {
            // A call (e.g. an app's transfer): the one asset leaving the
            // signer, and where it goes, from the decoded flows.
            let from = review.intent.from_account();
            let outs: Vec<_> = review
                .summary
                .outflows
                .iter()
                .filter(|f| f.account == from)
                .collect();
            if let [out] = outs.as_slice() {
                status.amount = Some(out.amount.to_string());
                status.token = match &out.asset {
                    crate::decode::Asset::Native => None,
                    crate::decode::Asset::Token { definition, .. } => Some(definition.clone()),
                };
                status.to = review
                    .summary
                    .inflows
                    .iter()
                    .find(|f| f.account != from && f.asset == out.asset)
                    .map(|f| f.account.clone());
            }
        }
        state.insert_status(status);
        state.pending = Some(Pending {
            handle,
            requester,
            hash: *prepared.hash(),
            kind: Kind::Tx(Box::new(prepared)),
            needs_password,
            needs_ack,
            deadline: Instant::now() + REQUEST_TTL,
            epoch,
            zone,
            ticket: ticket.clone(),
        });
        Ok(ticket)
    }

    /// `lez_requestFunds`: testnet funds for one of this wallet's accounts.
    /// A private target is funded through a public account of ours (`via`,
    /// else the first), and the shield into it is queued for approval (its
    /// ticket is returned). Takes as long as the faucet does; holds no lock.
    pub async fn request_funds<F: FaucetBackend>(
        &self,
        caller: &Caller,
        relayed: Option<&str>,
        account: &str,
        via: Option<&str>,
        faucet: &F,
        request_key: &str,
    ) -> Result<Funds> {
        let requester = Self::requester_for(caller, relayed)?;
        let (target_private, via) = {
            let mut wallet = self.wallet.lock().await;
            let session = wallet.session()?;
            let zone = session.zone().id.clone();
            if let (Caller::Module(name), false) = (
                caller,
                policy::allows(
                    session.grants(),
                    &zone,
                    requester.as_deref().unwrap_or(""),
                    account,
                    Capability::Accounts,
                ),
            ) {
                return Err(Denied::err(
                    Code::Unauthorized,
                    format!("{name} isn't connected to {account}"),
                ));
            }
            let accounts = session.accounts()?;
            let kind = accounts
                .iter()
                .find(|a| a.account_id == account)
                .map(|a| a.kind)
                .ok_or_else(|| {
                    Denied::err(
                        Code::InvalidParams,
                        format!("{account} is not in this wallet"),
                    )
                })?;
            let private = kind == AccountKind::Private;
            let via = if private {
                let v = match via {
                    Some(v) => accounts
                        .iter()
                        .find(|a| a.account_id == v && a.kind == AccountKind::Public),
                    None => accounts.iter().find(|a| a.kind == AccountKind::Public),
                };
                Some(
                    v.ok_or_else(|| {
                        Denied::err(
                            Code::InvalidParams,
                            "funding a private account needs a public account to shield from",
                        )
                    })?
                    .account_id
                    .clone(),
                )
            } else {
                None
            };
            // An app may only have its own accounts funded or shielded from:
            // `via` is revealed to it and linked to the target on chain.
            if let (Caller::Module(name), Some(v)) = (caller, via.as_deref()) {
                let app = requester.as_deref().unwrap_or("");
                for cap in [Capability::Accounts, Capability::ProposeTx] {
                    if !policy::allows(session.grants(), &zone, app, v, cap) {
                        return Err(Denied::err(
                            Code::Unauthorized,
                            format!("{name} isn't connected to {v} to shield from"),
                        ));
                    }
                }
            }
            (private, via)
        };
        if target_private {
            // Don't take funds for a shield that couldn't be queued.
            Self::check_free(&mut self.state(), requester.as_deref())?;
        }
        let public = via.as_deref().unwrap_or(account);
        let id = crate::decode::account_id(public)?;
        let outcome = faucet.fund(id, request_key).await?;
        // (A drip that settled after a restart knows no hash.)
        if let FundOutcome::Funded { tx_hash, .. } = &outcome
            && !tx_hash.is_empty()
        {
            let key = hash_key(tx_hash);
            let mut state = self.state();
            // The scan may have found the payment while the drip waited for
            // its block: label that row now.
            let mut relabelled = false;
            for s in state.statuses.values_mut() {
                if s.incoming && s.tx_hash.as_deref().map(hash_key).as_deref() == Some(key.as_str())
                {
                    s.title = Some("Test LGO from the faucet".to_owned());
                    relabelled = true;
                }
            }
            state.history_dirty |= relabelled;
            state.faucet_hashes.insert(key);
        }
        let (mut shield, mut shield_error) = (None, None);
        if let (FundOutcome::Funded { amount, .. }, true) = (&outcome, target_private) {
            // The funds arrived either way: report them even if the shield
            // can't be queued now (the user can shield later).
            match self
                .request_tx(
                    caller,
                    relayed,
                    Intent::Transfer {
                        from: public.to_owned(),
                        to: Some(account.to_owned()),
                        amount: *amount,
                        token: None,
                        to_keys: None,
                    },
                )
                .await
            {
                Ok(ticket) => shield = Some(ticket),
                Err(e) => shield_error = Some(format!("{e:#}")),
            }
        }
        Ok(Funds {
            outcome,
            faucet: faucet.name().to_owned(),
            funded_account: public.to_owned(),
            shield,
            shield_error,
        })
    }

    /// Ask to connect: `capabilities` on `accounts` for the requester.
    pub async fn request_connect(
        &self,
        caller: &Caller,
        relayed: Option<&str>,
        accounts: Vec<String>,
        capabilities: Vec<Capability>,
    ) -> Result<Ticket> {
        let requester = Self::requester_for(caller, relayed)?
            .context("a connect request needs a requesting app")
            .map_err(|e| Denied::err(Code::InvalidParams, e.to_string()))?;
        let epoch = self.state().epoch;
        let (chain, zone) = self
            .with_session(async |s| {
                let mine: Vec<String> = s.accounts()?.into_iter().map(|a| a.account_id).collect();
                for a in &accounts {
                    if !mine.contains(a) {
                        return Err(Denied::err(
                            Code::InvalidParams,
                            format!("{a} is not an account of this wallet"),
                        ));
                    }
                }
                Ok((s.zone().chain.clone(), s.zone().id.clone()))
            })
            .await?;
        let hash = connect_hash(&chain, &zone, &requester, &accounts, &capabilities);
        let handle = new_handle();
        let ticket = Ticket {
            handle: handle.clone(),
            request: RequestView::Connect {
                requester: requester.clone(),
                chain: chain.clone(),
                accounts: accounts.clone(),
                capabilities: capabilities.clone(),
                request_hash: hex::encode(hash),
            },
            needs_password: true,
            needs_acknowledgement: false,
            expires_in_ms: u64::try_from(REQUEST_TTL.as_millis()).unwrap_or(u64::MAX),
        };
        let mut state = self.state();
        Self::check_epoch(&state, epoch)?;
        Self::check_free(&mut state, Some(&requester))?;
        let mut status = TxStatus::new(&handle, &chain, Some(requester.clone()));
        status.title = Some(format!("Connect {requester}"));
        state.insert_status(status);
        state.pending = Some(Pending {
            handle,
            requester: Some(requester.clone()),
            kind: Kind::Connect {
                requester,
                accounts,
                capabilities,
            },
            hash,
            needs_password: true,
            needs_ack: false,
            deadline: Instant::now() + REQUEST_TTL,
            epoch,
            zone,
            ticket: ticket.clone(),
        });
        Ok(ticket)
    }

    /// The open request as the approving UI saw it, with the time left
    /// (None when nothing is open or it just ran out). Owner only.
    pub fn pending(&self, caller: &Caller) -> Option<Ticket> {
        if !caller.is_owner() {
            return None;
        }
        let state = self.state();
        let p = state.pending.as_ref()?;
        let left = p.deadline.checked_duration_since(Instant::now())?;
        let mut ticket = p.ticket.clone();
        ticket.expires_in_ms = u64::try_from(left.as_millis()).unwrap_or(u64::MAX);
        Some(ticket)
    }

    /// Every status this caller may read, newest first.
    pub fn statuses(&self, caller: &Caller) -> Vec<TxStatus> {
        let state = self.state();
        let mut out: Vec<TxStatus> = state
            .statuses
            .values()
            .filter(|s| Self::may_read(caller, s))
            .cloned()
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.phase_started_ms));
        out
    }

    /// Unlocked, time to auto-lock and zone status, without waiting: None
    /// while something holds the wallet (a long sync), so a status poll never
    /// queues behind it.
    pub fn peek(&self) -> Option<(bool, Option<Duration>, Option<crate::session::ZoneStatus>)> {
        let mut wallet = self.wallet.try_lock().ok()?;
        let remaining = wallet.remaining();
        let unlocked = wallet.is_unlocked();
        let status = if unlocked {
            wallet.session_quiet().ok().and_then(|s| s.status().ok())
        } else {
            None
        };
        Some((unlocked, remaining, status))
    }

    /// Whether a session is open (not locked).
    pub async fn is_unlocked(&self) -> bool {
        self.wallet.lock().await.is_unlocked()
    }

    /// Time left before auto-lock (None: locked).
    pub async fn remaining(&self) -> Option<Duration> {
        self.wallet.lock().await.remaining()
    }

    /// Like [`Engine::with_session`], without counting as use: background
    /// sync and the UI's own reads must not keep the wallet unlocked.
    pub async fn with_session_quiet<T>(
        &self,
        f: impl AsyncFnOnce(&mut Session) -> Result<T>,
    ) -> Result<T> {
        let mut wallet = self.wallet.lock().await;
        f(wallet.session_quiet()?).await
    }

    /// Count now as use (the user acted in the wallet UI). Never waits: while
    /// something holds the wallet, the touch is applied at the next tick.
    pub fn touch(&self) {
        match self.wallet.try_lock() {
            Ok(mut wallet) => wallet.touch(),
            Err(_) => self
                .touched
                .store(true, std::sync::atomic::Ordering::SeqCst),
        }
    }

    /// Drop every grant of `requester` (optionally only on `account`) in the
    /// current zone. Owner, or the app itself (disconnect).
    pub async fn revoke(
        &self,
        caller: &Caller,
        requester: &str,
        account: Option<&str>,
    ) -> Result<usize> {
        let own = matches!(caller, Caller::Module(n) if n == requester);
        if !caller.is_owner() && !own {
            return Err(Denied::err(Code::Unauthorized, "not your grants"));
        }
        self.with_session_quiet(async |s| {
            let zone = s.zone().id.clone();
            let before = s.grants().len();
            let kept: Vec<Grant> = s
                .grants()
                .iter()
                .filter(|g| {
                    !(g.zone == zone
                        && g.requester == requester
                        && account.is_none_or(|a| g.account == a))
                })
                .cloned()
                .collect();
            let removed = before - kept.len();
            if removed > 0 {
                s.set_grants(kept)?;
            }
            Ok(removed)
        })
        .await
    }

    /// Decline a pending request: the owner, or the app that asked (cancel).
    pub fn reject(&self, caller: &Caller, handle: &str) -> Result<()> {
        let mut state = self.state();
        let Some(p) = state.pending.as_ref().filter(|p| p.handle == handle) else {
            return Err(Denied::err(Code::UnknownHandle, "no such pending request"));
        };
        let own = matches!(caller, Caller::Module(n) if p.requester.as_deref() == Some(n));
        if !caller.is_owner() && !own {
            return Err(Denied::err(Code::UnknownHandle, "no such pending request"));
        }
        if let Some(p) = state.pending.take() {
            Self::finish(&mut state, &p, Lifecycle::Rejected, Some("declined"));
        }
        Ok(())
    }

    /// Stop a transaction that is being proved: nothing will be signed.
    pub fn cancel(&self, caller: &Caller, handle: &str) -> Result<()> {
        let mut state = self.state();
        let visible = state
            .statuses
            .get(handle)
            .is_some_and(|s| Self::may_read(caller, s));
        if !visible || state.proving.as_deref() != Some(handle) {
            return Err(Denied::err(Code::UnknownHandle, "nothing to cancel"));
        }
        state.cancelled.push(handle.to_owned());
        Ok(())
    }

    fn may_read(caller: &Caller, status: &TxStatus) -> bool {
        match caller {
            c if c.is_owner() => true,
            Caller::Module(name) => status.requester.as_deref() == Some(name.as_str()),
            _ => false,
        }
    }

    /// Only the requester and the owner can read a status; to anyone else an
    /// existing handle looks exactly like an unknown one.
    pub fn status(&self, caller: &Caller, handle: &str) -> Result<TxStatus> {
        let state = self.state();
        let mut status = state
            .statuses
            .get(handle)
            .filter(|s| Self::may_read(caller, s))
            .cloned()
            .ok_or_else(|| Denied::err(Code::UnknownHandle, "unknown transaction handle"))?;
        if !caller.is_owner() {
            // Internal detail (nonces, RPC errors) stays in the wallet.
            status.error = None;
        }
        Ok(status)
    }

    // -- approval ------------------------------------------------------------

    /// Approve the pending request `handle`. `echoed_hash` must be the hash
    /// the UI was given; `password` is required when the ticket said so, and
    /// `acknowledged_unknown` when it said the wallet can't decode the request.
    /// `progress` sees every status change (proving can take minutes).
    pub async fn approve(
        &self,
        caller: &Caller,
        handle: &str,
        echoed_hash: &str,
        password: Option<&str>,
        acknowledged_unknown: bool,
        progress: &mut (dyn FnMut(&TxStatus) + Send),
    ) -> Result<TxStatus> {
        if !caller.is_owner() {
            return Err(Denied::err(
                Code::Unauthorized,
                "only the wallet can approve requests",
            ));
        }
        // Atomic take: whatever happens next, this approval is used once.
        let pending = {
            let mut state = self.state();
            match state.pending.take() {
                Some(p) if p.handle == handle => p,
                other => {
                    state.pending = other;
                    return Err(Denied::err(Code::UnknownHandle, "no such pending request"));
                }
            }
        };
        if Instant::now() >= pending.deadline {
            Self::finish(
                &mut self.state(),
                &pending,
                Lifecycle::Expired,
                Some("not approved in time"),
            );
            return Err(Denied::err(Code::Timeout, "the request expired"));
        }
        if !same_hash(echoed_hash, &pending.hash) {
            Self::finish(
                &mut self.state(),
                &pending,
                Lifecycle::Rejected,
                Some("approval did not match the request"),
            );
            return Err(Denied::err(
                Code::Unauthorized,
                "the approval does not match the request; it was cancelled",
            ));
        }
        if pending.needs_ack && !acknowledged_unknown {
            // Not a rejection: the UI must show the warning and ask again
            // (unless the wallet moved on meanwhile).
            let mut state = self.state();
            if state.pending.is_none() && state.epoch == pending.epoch {
                state.pending = Some(pending);
            } else {
                Self::finish(
                    &mut state,
                    &pending,
                    Lifecycle::Expired,
                    Some("wallet changed"),
                );
            }
            return Err(Denied::err(
                Code::Unauthorized,
                "the wallet can't read what this does; acknowledge that to approve",
            ));
        }
        if pending.needs_password {
            let checked = self
                .with_session(async |s| match password {
                    Some(pw) => s.reauth(pw),
                    None => Err(Denied::err(
                        Code::Unauthorized,
                        "enter your password to approve",
                    )),
                })
                .await;
            if let Err(e) = checked {
                // A typo doesn't cost the user the request, unless the wallet
                // moved on meanwhile (lock, zone switch, a newer request).
                let mut state = self.state();
                if state.pending.is_none() && state.epoch == pending.epoch {
                    state.pending = Some(pending);
                } else {
                    Self::finish(
                        &mut state,
                        &pending,
                        Lifecycle::Expired,
                        Some("wallet changed"),
                    );
                }
                return Err(e);
            }
        }
        let (epoch, zone) = (pending.epoch, pending.zone.clone());

        match pending.kind {
            Kind::Connect {
                requester,
                accounts,
                capabilities,
            } => {
                self.with_session(async |s| {
                    self.same_wallet(epoch, &zone, s)?;
                    let mut grants = s.grants().to_vec();
                    for account in &accounts {
                        for cap in &capabilities {
                            let g = Grant {
                                zone: zone.clone(),
                                requester: requester.clone(),
                                account: account.clone(),
                                capability: *cap,
                            };
                            if !grants.contains(&g) {
                                grants.push(g);
                            }
                        }
                    }
                    s.set_grants(grants)
                })
                .await?;
                Ok(self.update(handle, progress, |s| {
                    s.lifecycle = Lifecycle::Included;
                    s.outcome = Outcome::Success;
                    s.phase_started_ms = now_ms();
                }))
            }
            Kind::Tx(prepared) => self.run_tx(handle, *prepared, epoch, &zone, progress).await,
        }
    }

    /// Whether another unfinished transaction spends from the same account.
    fn others_in_flight(&self, handle: &str, watch: Option<&PublicWatch>) -> bool {
        let Some(PublicWatch { from, .. }) = watch else {
            return false;
        };
        self.state().statuses.values().any(|s| {
            s.handle != handle
                && !s.is_final()
                && s.lifecycle != Lifecycle::AwaitingApproval
                && s.from.as_deref() == Some(from.as_str())
        })
    }

    /// The session is still the one the request was built for.
    fn same_wallet(&self, epoch: u64, zone: &str, s: &Session) -> Result<()> {
        if self.state().epoch != epoch || s.zone().id != zone {
            return Err(Denied::err(
                Code::StaleApproval,
                "the wallet was locked or switched zones since you approved",
            ));
        }
        Ok(())
    }

    fn update(
        &self,
        handle: &str,
        progress: &mut (dyn FnMut(&TxStatus) + Send),
        f: impl FnOnce(&mut TxStatus),
    ) -> TxStatus {
        let mut state = self.state();
        let status = state
            .statuses
            .get_mut(handle)
            .expect("status exists for every request");
        f(status);
        let snapshot = status.clone();
        state.history_dirty = true;
        drop(state);
        progress(&snapshot);
        snapshot
    }

    fn phase(
        &self,
        handle: &str,
        lifecycle: Lifecycle,
        progress: &mut (dyn FnMut(&TxStatus) + Send),
    ) {
        self.update(handle, progress, |s| {
            s.lifecycle = lifecycle;
            s.phase_started_ms = now_ms();
        });
    }

    fn fail(
        &self,
        handle: &str,
        lifecycle: Lifecycle,
        e: anyhow::Error,
        progress: &mut (dyn FnMut(&TxStatus) + Send),
    ) -> anyhow::Error {
        let code = policy::code_of(&e) as i64;
        self.update(handle, progress, |s| {
            s.lifecycle = lifecycle;
            s.phase_started_ms = now_ms();
            s.error = Some(crate::decode::shorten_ids(&format!("{e:#}")));
            s.error_code = Some(code);
        });
        e
    }

    async fn run_tx(
        &self,
        handle: &str,
        prepared: Prepared,
        epoch: u64,
        zone: &str,
        progress: &mut (dyn FnMut(&TxStatus) + Send),
    ) -> Result<TxStatus> {
        // Own-account invariant: which of our private accounts must move, by how much.
        let watch = own_invariant(&prepared.review);
        // Public: our sender's native outflow and the fee cap.
        let public_watch = public_invariant(&prepared.review);
        // A testimonial post proves itself by the record it writes.
        let post =
            crate::testimonial::watch(prepared.review.program.as_ref(), prepared.public_message());
        let tx_hash = if prepared.needs_proof() {
            let (job, pins) = prepared.into_proving()?;
            let slot = {
                let mut state = self.state();
                if state.proving.is_some() {
                    drop(state);
                    let e = Denied::err(
                        Code::RequestPending,
                        "Another private transaction is being proved. Try again when it finishes.",
                    );
                    return Err(self.fail(handle, Lifecycle::Dropped, e, progress));
                }
                state.proving = Some(handle.to_owned());
                ProvingSlot {
                    engine: self,
                    handle: handle.to_owned(),
                }
            };
            let before = match &watch {
                Some((account, token, _)) => self
                    .with_session(async |s| s.balance_of(account, token.as_deref()).await)
                    .await
                    .ok(),
                None => None,
            };
            // Enough free memory, and the segment size for it (`proving`).
            if let Err(e) = crate::proving::prepare() {
                drop(slot);
                return Err(self.fail(handle, Lifecycle::Dropped, e, progress));
            }
            self.phase(handle, Lifecycle::Proving, progress);
            let proved = tokio::task::spawn_blocking(move || job.run()).await;
            let cancelled = self.state().cancelled.iter().any(|h| h == handle);
            // The user sat through the proof: that counts as use. Before the
            // slot goes, so an auto-lock can't land between proof and signing.
            self.wallet.lock().await.touch();
            drop(slot);
            if cancelled {
                let e = Denied::err(Code::UserRejected, "cancelled before signing");
                return Err(self.fail(handle, Lifecycle::Dropped, e, progress));
            }
            let proved = match proved.context("prover thread").and_then(|r| r) {
                Ok(p) => p,
                Err(e) => {
                    let e = Denied::err(Code::ProofFailed, format!("{e:#}"));
                    return Err(self.fail(handle, Lifecycle::Dropped, e, progress));
                }
            };
            self.phase(handle, Lifecycle::Signing, progress);
            let sent = self
                .with_session(async |s| {
                    self.same_wallet(epoch, zone, s)?;
                    s.submit_proved(&pins, proved).await
                })
                .await;
            match sent {
                Ok(h) => (h, before),
                Err(e) => return Err(self.fail(handle, Lifecycle::Dropped, e, progress)),
            }
        } else {
            self.phase(handle, Lifecycle::Signing, progress);
            match self
                .with_session(async |s| {
                    self.same_wallet(epoch, zone, s)?;
                    // Public outflow: the sender's balance is the evidence.
                    let before = match &public_watch {
                        Some(w) => s.balance_of(&w.from, w.token.as_deref()).await.ok(),
                        None => None,
                    };
                    Ok((s.submit_public(prepared).await?, before))
                })
                .await
            {
                Ok((h, before)) => (h, before),
                Err(e) => return Err(self.fail(handle, Lifecycle::Dropped, e, progress)),
            }
        };
        let (tx_hash, before) = tx_hash;

        self.update(handle, progress, |s| {
            s.lifecycle = Lifecycle::Submitted;
            s.phase_started_ms = now_ms();
            s.tx_hash = Some(tx_hash_hex(&tx_hash));
        });
        // A poll that gives up leaves the outcome unknown, not failed: the
        // transaction may still land (retrying could send it twice).
        // Wait for the block without the wallet lock (it can take minutes);
        // then record the result under it (the poll answers at once).
        let poller = self
            .with_session_quiet(async |s| Ok(s.core().context("not connected")?.poller()))
            .await;
        let seen = match (poller, tx_hash.parse()) {
            (Ok(poller), Ok(hash)) => poller.poll_tx(hash).await.map(|_| ()),
            (Err(e), _) => Err(e),
            (_, Err(_)) => Err(anyhow::anyhow!("bad transaction hash")),
        };
        // Not seen: stop here rather than wait again under the lock.
        if let Err(e) = seen {
            let e = Denied::err(
                Code::Timeout,
                format!("sent, not seen in a block yet: {e:#}"),
            );
            return Err(self.fail(handle, Lifecycle::Submitted, e, progress));
        }
        let included = self
            .with_session(async |s| {
                self.same_wallet(epoch, zone, s)?;
                s.wait_included(&tx_hash).await
            })
            .await
            .map_err(|e| {
                Denied::err(
                    Code::Timeout,
                    format!("sent, not seen in a block yet: {e:#}"),
                )
            });
        let (block, warning) = match included {
            Ok(done) => done,
            Err(e) => return Err(self.fail(handle, Lifecycle::Submitted, e, progress)),
        };
        // Own-account invariant: our private account moved by exactly the amount.
        let mut fee_paid = None;
        let outcome = match (watch, before) {
            (Some((account, token, delta)), Some(before)) => {
                let after = self
                    .with_session(async |s| {
                        self.same_wallet(epoch, zone, s)?;
                        s.balance_of(&account, token.as_deref()).await
                    })
                    .await
                    .ok();
                let expected = if delta >= 0 {
                    before.checked_add(delta.unsigned_abs())
                } else {
                    before.checked_sub(delta.unsigned_abs())
                };
                match after {
                    // The note wasn't recorded locally: the balance proves nothing yet.
                    _ if warning.is_some() => (Outcome::Unknown, OutcomeSource::None),
                    Some(after) if Some(after) == expected => {
                        (Outcome::Success, OutcomeSource::OwnAccountInvariant)
                    }
                    // Included, note recorded, and our account didn't move.
                    Some(after) if after == before => {
                        (Outcome::Failure, OutcomeSource::OwnAccountInvariant)
                    }
                    // Moved by another amount (e.g. another note arrived).
                    _ => (Outcome::Unknown, OutcomeSource::None),
                }
            }
            // Public native outflow: the sender dropped by the amount plus a
            // fee within the approved cap. Anything else (an incoming payment
            // in between, only the fee charged) proves nothing either way.
            // Another send from this account in flight could move it too.
            (None, Some(_)) if self.others_in_flight(handle, public_watch.as_ref()) => {
                (Outcome::Unknown, OutcomeSource::None)
            }
            (None, Some(before)) if public_watch.is_some() && post.is_none() => {
                let w = public_watch.clone().unwrap_or_default();
                let after = self
                    .with_session(async |s| {
                        self.same_wallet(epoch, zone, s)?;
                        s.balance_of(&w.from, w.token.as_deref()).await
                    })
                    .await
                    .ok();
                // Native pays the fee too (within the cap); a token moves by
                // exactly the amount (fees are native).
                let fee = if w.token.is_some() { 0 } else { w.max_fee };
                let moved = if w.incoming {
                    // Arrives exactly (fees are native, the slot is ours).
                    after
                        .and_then(|a| a.checked_sub(before))
                        .filter(|d| *d == w.out)
                } else {
                    after.and_then(|a| before.checked_sub(a))
                };
                match moved {
                    Some(d) if d >= w.out && d <= w.out.saturating_add(fee) => {
                        if w.token.is_none() && !w.incoming {
                            fee_paid = Some((d - w.out).to_string());
                        }
                        (Outcome::Success, OutcomeSource::OwnAccountInvariant)
                    }
                    _ => (Outcome::Unknown, OutcomeSource::None),
                }
            }
            _ => match post {
                Some(post) => match self
                    .with_session(async |s| {
                        let core = s.core().context("not connected")?;
                        crate::testimonial::observe(core, &post).await
                    })
                    .await
                {
                    // The record is keyed by our author account.
                    Ok(Some(true)) => (Outcome::Success, OutcomeSource::OwnAccountInvariant),
                    Ok(Some(false)) => (Outcome::Failure, OutcomeSource::OwnAccountInvariant),
                    _ => (Outcome::Unknown, OutcomeSource::None),
                },
                None => (Outcome::Unknown, OutcomeSource::None),
            },
        };
        Ok(self.update(handle, progress, |s| {
            s.lifecycle = Lifecycle::Included;
            s.phase_started_ms = now_ms();
            s.block = Some(block);
            (s.outcome, s.outcome_source) = outcome;
            s.fee_paid = fee_paid;
            s.error = warning;
        }))
    }
}

/// A public transaction's evidence: our sending account, the asset it sends
/// (`token: None` = native), how much, and the fee cap it approved.
#[derive(Clone, Debug, Default)]
struct PublicWatch {
    from: String,
    token: Option<String>,
    out: u128,
    max_fee: u128,
    /// The amount arrives instead of leaving (a token create's supply).
    incoming: bool,
}

/// For public transactions: the native outflow of our sender, else its one
/// fungible-token outflow (held in its own token slot).
fn public_invariant(review: &Review) -> Option<PublicWatch> {
    use crate::decode::Asset;
    if review.route.is_some_and(tx::Route::is_private) {
        return None;
    }
    let from = review.intent.from_account().to_owned();
    let max_fee: u128 = review.fee.max_fee.as_deref()?.parse().ok()?;
    let ours = || review.summary.outflows.iter().filter(|f| f.account == from);
    let native = ours()
        .filter(|f| f.asset == Asset::Native)
        .fold(0u128, |a, f| a.saturating_add(f.amount));
    if native > 0 {
        return Some(PublicWatch {
            from,
            token: None,
            out: native,
            max_fee,
            incoming: false,
        });
    }
    let mut tokens = ours().filter_map(|f| match &f.asset {
        Asset::Token {
            definition,
            nft: None,
        } => Some((definition.clone(), f.amount)),
        _ => None,
    });
    if let Some((definition, out)) = tokens.next() {
        return (tokens.next().is_none() && out > 0).then_some(PublicWatch {
            from,
            token: Some(definition),
            out,
            max_fee,
            incoming: false,
        });
    }
    // Nothing leaves: one fungible-token inflow to our sender (token create).
    let mut ins = review.summary.inflows.iter().filter(|f| f.account == from);
    let f = ins.next()?;
    let Asset::Token {
        definition,
        nft: None,
    } = &f.asset
    else {
        return None;
    };
    (ins.next().is_none() && f.amount > 0).then(|| PublicWatch {
        from,
        token: Some(definition.clone()),
        out: f.amount,
        max_fee,
        incoming: true,
    })
}

/// For private routes: the private account of ours whose balance must change,
/// the asset, and the signed change (a shield credits, a spend debits).
fn own_invariant(review: &Review) -> Option<(String, Option<String>, i128)> {
    let Intent::Transfer {
        from,
        amount,
        token,
        ..
    } = &review.intent
    else {
        return None;
    };
    let amount = i128::try_from(*amount).ok()?;
    match review.route? {
        tx::Route::Shield => Some((review.recipient.clone()?, token.clone(), amount)),
        tx::Route::Unshield | tx::Route::Private => Some((from.clone(), token.clone(), -amount)),
        tx::Route::Public => None,
    }
}

fn connect_hash(
    chain: &str,
    zone: &str,
    requester: &str,
    accounts: &[String],
    caps: &[Capability],
) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(b"logos-kit/connect/v1\0");
    let body = serde_json::json!({
        "chain": chain, "zone": zone, "requester": requester, "accounts": accounts, "capabilities": caps,
    });
    h.update(body.to_string().as_bytes());
    h.finalize().into()
}
