//! Transactions: build, bind to an approval, (prove), re-check, sign, submit.
//!
//! The approval binds to a **request hash** (SHA-256 over a domain tag, the
//! chain and zone, the requester, the intent and the exact message, or for a
//! private transaction its expected public effects, recipient and signer
//! nonces). The approving UI must echo it back, so a UI shown one request
//! can't approve another.
//!
//! At sign time the mutable inputs are fetched again: a signer's nonce, and
//! the header of an upgradeable program. If either moved, the approval is
//! stale ([`Stale`]). A private transaction is proved after approval; its
//! proved message must have exactly the approved public effects (decoded by
//! `decode.rs`), nonces and program images before it is signed.

use anyhow::{Context as _, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use lee::{
    AccountId, privacy_preserving_transaction::circuit::ProgramWithDependencies, program::Program,
};
use lee_core::{
    Identifier, NullifierPublicKey, PrivateAccountKind, ProgramImageClaim, SharedSecretKey,
    encryption::ViewingPublicKey,
    native_token::{self, NATIVE_TOKEN_PROGRAM_ID, decode_balance},
};
use sequencer_service_rpc::RpcClient as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use token_core::{TokenDescriptor, TokenHolding, TokenKind};
use wallet::{
    AccDecodeData, AccountIdentity, ExecutionFailureKind, PreparedPrivateTx, PreparedPublicTx,
    ProvedPrivateTx, SelectedShard, TxSigner, WalletCore,
};

use crate::{
    decode::{self, Asset, Decoders, Flow, PublicEffect, Summary},
    session::Session,
    verify::{self, ProgramCheck},
};

const HASH_DOMAIN: &[u8] = b"logos-kit/approval/v2\0";
/// Largest instruction payload an app may propose (the sequencer's own cap is higher).
const MAX_CALL_DATA: usize = 64 * 1024;
const MAX_CALL_ACCOUNTS: usize = 32;

/// What a caller asks the wallet to do. Amounts are base units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Intent {
    /// Move native tokens or a token. The route follows from the accounts:
    /// public → public, public → own private (shield), own private → public
    /// (unshield), own private → private (own, or anyone's via `toKeys`).
    #[serde(rename_all = "camelCase")]
    Transfer {
        from: String,
        /// Recipient account. For someone else's private account give
        /// `toKeys` instead (and optionally `to`, which must then match).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
        #[serde(with = "amount")]
        amount: u128,
        /// Token definition account; omit for the native token.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_keys: Option<RecipientKeys>,
    },
    /// A public call to any program (dApps). Decoded for the approval sheet;
    /// a program the wallet can't decode must be acknowledged explicitly.
    #[serde(rename_all = "camelCase")]
    Call {
        /// Pays the fee; one of this wallet's public accounts.
        from: String,
        program: String,
        accounts: Vec<CallAccount>,
        /// Borsh instruction bytes, base64.
        data: String,
    },
    /// Post to the testimonial program. The wallet adds the time and the
    /// program's accounts. `from` signs, pays and is shown on-chain as the
    /// author, so it must be public.
    #[serde(rename_all = "camelCase")]
    Testimonial {
        from: String,
        /// Program account; defaults to the registry's for this chain. It
        /// must run the Logos Kit testimonial image.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program: Option<String>,
        #[serde(default = "default_submission")]
        submission: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        text: String,
    },
}

fn default_submission() -> String {
    crate::testimonial::SUBMISSION.to_owned()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallAccount {
    pub account: String,
    /// Which program's shard of the account; defaults to the called program.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shard: Option<String>,
    /// This wallet signs for it (must hold its key).
    #[serde(default)]
    pub signer: bool,
}

/// Someone else's private account: what `logos-kit account keys` (LEZ `show-keys` format) exports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipientKeys {
    /// Nullifier public key, 32 bytes hex.
    pub npk: String,
    /// Viewing public key (ML-KEM-768), hex.
    pub vpk: String,
    /// Base58; a fresh random one is chosen (and shown for approval) if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
}

/// Amounts travel as decimal strings: JSON numbers lose precision past 2^53.
pub(crate) mod amount {
    use serde::{Deserialize as _, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl Intent {
    /// The account whose grant an app needs to propose this.
    pub fn from_account(&self) -> &str {
        match self {
            Self::Transfer { from, .. }
            | Self::Call { from, .. }
            | Self::Testimonial { from, .. } => from,
        }
    }
}

/// How a transfer travels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    Public,
    Shield,
    Unshield,
    Private,
}

impl Route {
    /// Needs a zero-knowledge proof (minutes on a desktop CPU).
    pub const fn is_private(self) -> bool {
        !matches!(self, Self::Public)
    }

    /// Spends from a private account (its key authorizes inside the proof).
    pub const fn spends_private(self) -> bool {
        matches!(self, Self::Unshield | Self::Private)
    }
}

/// The approval sheet's content: everything the user is agreeing to.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub chain: String,
    /// The app asking (module name), or `None` when the owner asked (CLI, wallet UI).
    pub requester: Option<String>,
    pub intent: Intent,
    /// Transfers only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<Route>,
    /// Decoded from the message (public) or from the approved effects (private).
    pub summary: Summary,
    /// The program's header and verification status (not for native transfers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<ProgramCheck>,
    /// Private routes: the public effects the proof must have, exactly.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expected_effects: Vec<PublicEffect>,
    /// The resolved recipient (for `toKeys`, derived with the chosen identifier).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<String>,
    /// The sender's balance of the asset now, for "balance after".
    #[serde(with = "amount")]
    pub from_balance: u128,
    pub fee: Fee,
    /// Hex; the approving UI echoes it back.
    pub request_hash: String,
}

impl Review {
    /// Native value leaving this wallet's own accounts (for the re-auth threshold).
    pub fn native_outflow(&self) -> u128 {
        self.summary
            .outflows
            .iter()
            .filter(|f| f.asset == Asset::Native)
            .fold(0u128, |a, f| a.saturating_add(f.amount))
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fee {
    /// Who pays (public transactions); private ones are fee-exempt in 0.3.
    pub payer: Option<String>,
    /// The cap the transaction reserves; the real fee is at most this.
    pub max_fee: Option<String>,
    pub gas_limit: Option<u64>,
    /// The sequencer's current execution base fee per gas (`getFeeState`),
    /// when it offers one. Gas used is only known after inclusion.
    pub base_fee_exec: Option<u64>,
    /// The fee at today's base fees. LEZ charges `cycles·base_fee_exec +
    /// bytes·base_fee_stor + tip`, and runs the native transfer outside the
    /// zkVM at zero cycles: for it this is the fee (`exact`), unless the base
    /// fee moves before inclusion. A guest program's cycles are only known
    /// once it runs, so for those this is the most it can cost (cycles at
    /// the gas limit). The 0.3 RPC reports no gas used, so the paid fee is
    /// read from the sender's balance afterwards.
    pub estimate: Option<String>,
    pub exact: bool,
    /// Signed size in bytes, the tip, and whether execution is free.
    #[serde(skip)]
    sizing: Option<(u64, u64, bool)>,
}

/// Borsh size of a signed public transaction around its message: the
/// `LeeTransaction` variant tag and the witness `Vec` length, then a 64-byte
/// signature and a 32-byte public key per signer. LEZ bills storage gas on
/// exactly this size.
const SIGNED_OVERHEAD_BYTES: u64 = 1 + 4;
const WITNESS_BYTES_PER_SIGNER: u64 = 64 + 32;

/// Returned (inside `anyhow::Error`) when something changed since approval.
#[derive(Debug)]
pub struct Stale(pub String);

impl std::fmt::Display for Stale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "changed since you approved: {}", self.0)
    }
}

impl std::error::Error for Stale {}

/// What must still hold at sign time.
#[derive(Clone, Debug)]
pub struct Pins {
    /// Public signers and the nonces they were approved at.
    nonces: Vec<(AccountId, u128)>,
    /// An upgradeable program's image id when approved.
    program: Option<(AccountId, [u32; 8])>,
}

/// A built transaction waiting for approval. A private body carries the
/// spender's proving witness in memory until it is proved or dropped.
pub struct Prepared {
    pub review: Review,
    hash: [u8; 32],
    pins: Pins,
    body: Body,
}

impl Prepared {
    /// The public message that gets signed (public transactions only).
    pub fn public_message(&self) -> Option<&lee::public_transaction::Message> {
        match &self.body {
            Body::Public(tx) => Some(tx.message()),
            Body::Private { .. } => None,
        }
    }
}

enum Body {
    Public(PreparedPublicTx),
    Private {
        tx: Box<PreparedPrivateTx>,
        check: ProofCheck,
    },
}

/// What a proof must match, and which of its notes are ours to record.
pub struct ProofCheck {
    effects: Vec<PublicEffect>,
    /// Private accounts in mention order; `Some` for this wallet's own.
    notes: Vec<Option<AccountId>>,
    /// The program whose image a disclosed claim must name (token transfers).
    program: Option<(AccountId, [u32; 8])>,
}

/// Private body after proving, ready for the re-check and signature.
pub struct Proved {
    tx: ProvedPrivateTx,
    check: ProofCheck,
}

impl Prepared {
    pub const fn hash(&self) -> &[u8; 32] {
        &self.hash
    }

    pub const fn needs_proof(&self) -> bool {
        matches!(self.body, Body::Private { .. })
    }
}

fn lez(e: ExecutionFailureKind) -> anyhow::Error {
    match e {
        // LEZ's "Can not pay for operation": the balance the network shows
        // is short. On a busy testnet a payment can take a minute to show.
        ExecutionFailureKind::InsufficientFundsError => anyhow::anyhow!(
            "Not enough funds for this, as the network shows the account now. If you were just paid, the network may not show it yet: try again in a minute."
        ),
        e => anyhow::anyhow!("{e}"),
    }
}

/// Account ids may carry LEZ's `Public/` or `Private/` prefix.
fn parse_account(s: &str) -> Result<(AccountId, Option<bool>)> {
    if let Some(rest) = s.strip_prefix("Public/") {
        Ok((decode::account_id(rest)?, Some(false)))
    } else if let Some(rest) = s.strip_prefix("Private/") {
        Ok((decode::account_id(rest)?, Some(true)))
    } else {
        Ok((decode::account_id(s)?, None))
    }
}

fn request_hash(scope: &str, requester: Option<&str>, intent: &Intent, bound: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(HASH_DOMAIN);
    // Serialized with a fixed field order (serde derive), so it is canonical.
    let intent = serde_json::to_vec(intent).unwrap_or_default();
    for part in [
        scope.as_bytes(),
        requester.unwrap_or("").as_bytes(),
        &intent,
        bound,
    ] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part);
    }
    h.finalize().into()
}

async fn nonce_of(core: &WalletCore, id: AccountId) -> Result<u128> {
    let nonces = core.get_accounts_nonces(&[id]).await?;
    Ok(nonces.first().context("sequencer returned no nonce")?.0)
}

/// The shard `program` keeps in one of this wallet's accounts (public: live;
/// private: as synced).
pub(crate) async fn own_shard(
    core: &WalletCore,
    id: AccountId,
    private: bool,
    program: AccountId,
) -> Result<Vec<u8>> {
    if private {
        let account = core
            .private_account_state(id)
            .with_context(|| format!("{id} is not a private account of this wallet"))?;
        Ok(account.data.shard(program).as_ref().to_vec())
    } else {
        let account = core
            .get_account_view(lee::ProgramShardSelector::new(id, program))
            .await?;
        Ok(account.data.shard(program).as_ref().to_vec())
    }
}

fn holding(bytes: &[u8]) -> Result<Option<TokenHolding>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    borsh::from_slice(bytes)
        .map(Some)
        .map_err(|_| anyhow::anyhow!("the account's token data doesn't decode"))
}

/// Amount of `asset` a holding shard represents (fungible balance, NFT count).
pub(crate) fn holding_amount(h: &TokenHolding) -> u128 {
    match h {
        TokenHolding::Fungible { balance, .. } => *balance,
        TokenHolding::NftMaster { print_balance, .. } => *print_balance,
        TokenHolding::NftPrintedCopy { owned, .. } => u128::from(*owned),
    }
}

/// Funds check run on the exact shards LEZ selected (same as its facades),
/// plus the recipient-side check the token program would fail on.
fn preflight(
    program: AccountId,
    descriptor: Option<TokenDescriptor>,
    amount: u128,
) -> impl FnOnce(&[SelectedShard]) -> Result<(), ExecutionFailureKind> {
    move |shards: &[SelectedShard]| {
        let [from, to] = shards else {
            return Err(ExecutionFailureKind::AmountMismatchError);
        };
        let bad = |s: &SelectedShard| ExecutionFailureKind::AccountDataError(s.selector.account_id);
        let Some(descriptor) = descriptor else {
            let balance = decode_balance(from.shard_of(program)).map_err(|_| bad(from))?;
            return if balance >= amount {
                Ok(())
            } else {
                Err(ExecutionFailureKind::InsufficientFundsError)
            };
        };
        let sender = TokenHolding::try_from(from.shard_of(program)).map_err(|_| bad(from))?;
        if holding_amount(&sender) < amount {
            return Err(ExecutionFailureKind::InsufficientFundsError);
        }
        let recipient = to.shard_of(program);
        if !recipient.is_empty() {
            let r = TokenHolding::try_from(recipient).map_err(|_| bad(to))?;
            if r.definition_id() != descriptor.definition_id {
                return Err(bad(to));
            }
        }
        Ok(())
    }
}

/// Everything a transfer's route needs, resolved against this wallet.
struct Resolved {
    route: Route,
    from: AccountIdentity,
    to: AccountIdentity,
    to_id: AccountId,
    /// Private mentions in order, `Some` when the account is ours.
    notes: Vec<Option<AccountId>>,
}

fn resolve(
    core: &WalletCore,
    from: AccountId,
    to: Option<(AccountId, Option<bool>)>,
    keys: Option<&RecipientKeys>,
) -> Result<Resolved> {
    let (from_ident, from_private) = if core.get_account_public_signing_key(from).is_some() {
        (AccountIdentity::Public(from), false)
    } else if let Some(p @ AccountIdentity::PrivateOwned(_)) = core.resolve_private_account(from) {
        // Shared (GMS) accounts aren't supported yet: their notes and
        // balances live outside this wallet's key tree.
        (p, true)
    } else {
        bail!("{from} is not an account of this wallet");
    };

    let (to_ident, to_id, to_private, to_ours) = if let Some(keys) = keys {
        let npk: [u8; 32] = hex::decode(keys.npk.trim())
            .ok()
            .and_then(|b| b.try_into().ok())
            .context("toKeys.npk must be 32 bytes of hex")?;
        let npk = NullifierPublicKey(npk);
        let vpk = ViewingPublicKey::from_bytes(
            hex::decode(keys.vpk.trim()).context("toKeys.vpk must be hex")?,
        )
        .map_err(|e| anyhow::anyhow!("toKeys.vpk is not a viewing key: {e:?}"))?;
        let identifier: Identifier = keys
            .identifier
            .as_deref()
            .context("recipient identifier missing")?
            .parse()
            .map_err(|e| anyhow::anyhow!("toKeys.identifier: {e}"))?;
        let id = AccountId::from((&npk, &vpk, identifier));
        if let Some((given, _)) = to {
            ensure!(
                given == id,
                "`to` {given} doesn't match the account these keys derive ({id})"
            );
        }
        let ident = AccountIdentity::PrivateForeign {
            npk,
            vpk,
            kind: PrivateAccountKind::Regular(identifier),
        };
        (ident, id, true, false)
    } else {
        let (id, marked) = to.context("a recipient (`to` or `toKeys`) is required")?;
        if let Some(p @ AccountIdentity::PrivateOwned(_)) = core.resolve_private_account(id) {
            (p, id, true, true)
        } else {
            // Someone else's private account can't be paid by id alone.
            ensure!(
                marked != Some(true),
                "{id} is a private account of someone else: send to their keys (toKeys)"
            );
            (AccountIdentity::PublicNoSign(id), id, false, false)
        }
    };
    ensure!(from != to_id, "sender and recipient are the same account");

    let route = match (from_private, to_private) {
        (false, false) => Route::Public,
        (false, true) => Route::Shield,
        (true, false) => Route::Unshield,
        (true, true) => Route::Private,
    };
    ensure!(
        !(route == Route::Shield && !to_ours),
        "shielding goes to one of this wallet's private accounts; to pay someone privately, send from a private account"
    );
    let mut notes = Vec::new();
    if from_private {
        notes.push(Some(from));
    }
    if to_private {
        notes.push(to_ours.then_some(to_id));
    }
    Ok(Resolved {
        route,
        from: from_ident,
        to: to_ident,
        to_id,
        notes,
    })
}

fn fee_of(message: &lee::public_transaction::Message) -> Fee {
    message.fee.as_ref().map_or_else(Fee::default, |f| Fee {
        payer: Some(f.payer.to_string()),
        max_fee: Some(f.max_fee.to_string()),
        gas_limit: Some(f.gas_limit),
        base_fee_exec: None,
        estimate: None,
        exact: false,
        sizing: borsh::to_vec(message).ok().map(|b| {
            let witness = WITNESS_BYTES_PER_SIGNER * message.nonces.len() as u64;
            (
                b.len() as u64 + SIGNED_OVERHEAD_BYTES + witness,
                f.tip,
                message.program_account_id == NATIVE_TOKEN_PROGRAM_ID,
            )
        }),
    })
}

/// Lepta as LGO text: 1 LGO = 10^9 lepta, trailing zeros trimmed.
pub(crate) fn lgo(lepta: u128) -> String {
    let whole = lepta / 1_000_000_000;
    let frac = lepta % 1_000_000_000;
    if frac == 0 {
        return format!("{whole} LGO");
    }
    let frac = format!("{frac:09}");
    format!("{whole}.{} LGO", frac.trim_end_matches('0'))
}

/// The fee payer can cover `max_fee` on top of what `sender` sends natively.
async fn check_fee(
    core: &WalletCore,
    message: &lee::public_transaction::Message,
    sender: AccountId,
    sender_balance: u128,
    native_out: u128,
) -> Result<()> {
    let Some(f) = message.fee.as_ref() else {
        return Ok(());
    };
    if f.payer == sender {
        ensure!(
            sender_balance >= native_out.saturating_add(f.max_fee),
            "Not enough LGO. This needs {} plus up to {} for the fee, and the account has {}.",
            lgo(native_out),
            lgo(f.max_fee),
            lgo(sender_balance)
        );
    } else {
        let payer = core.get_account_balance(f.payer).await?;
        ensure!(
            payer >= f.max_fee,
            "The account paying the fee ({}) has {}, and the fee can be up to {}.",
            f.payer,
            lgo(payer),
            lgo(f.max_fee)
        );
    }
    Ok(())
}

impl Session {
    pub fn decoders(&self) -> Decoders {
        Decoders::default()
            .with_rebuilds(
                &self.zone().id,
                crate::verify::load_cache(self.data_dir().root()),
            )
            .with_named(self.named_programs())
    }

    /// Build `intent` for approval (connects if needed). Checks that the
    /// spender is ours, funds, the recipient's token slot, the program header.
    pub async fn prepare(&mut self, requester: Option<&str>, intent: Intent) -> Result<Prepared> {
        let chain = self.zone().chain.clone();
        self.connect().await?;
        let decoders = self.decoders();
        // Chain and zone id: two zones never share an approval.
        let scope = format!("{chain}|{}", self.zone().id);
        let core = self.core().context("not connected")?;

        let (mut intent, built) = match intent {
            Intent::Transfer { .. } => prepare_transfer(core, &decoders, intent).await?,
            Intent::Call { .. } => (
                intent.clone(),
                prepare_call(core, &decoders, &intent).await?,
            ),
            Intent::Testimonial { .. } => {
                prepare_testimonial(core, &decoders, intent, &chain).await?
            }
        };
        // A foreign recipient's random identifier is part of what is approved.
        if let (
            Intent::Transfer {
                to_keys: Some(k), ..
            },
            Some(id),
        ) = (&mut intent, built.chosen_identifier.as_ref())
        {
            k.identifier = Some(id.clone());
        }
        let Built {
            route,
            summary,
            program,
            expected_effects,
            recipient,
            from_balance,
            mut fee,
            pins,
            body,
            bound,
            ..
        } = built;
        if fee.payer.is_some() {
            // Informational only: the approval binds to max_fee, not these.
            if let Ok(q) = core.client().get_fee_state().await {
                fee.base_fee_exec = Some(q.base_fee_exec);
                if let Some((gas, (bytes, tip, exec_free))) = fee.gas_limit.zip(fee.sizing) {
                    let exec = if exec_free { 0 } else { u128::from(gas) };
                    fee.estimate = Some(
                        (exec * u128::from(q.base_fee_exec)
                            + u128::from(bytes) * u128::from(q.base_fee_stor)
                            + u128::from(tip))
                        .to_string(),
                    );
                    fee.exact = exec_free;
                }
            }
        }
        let hash = request_hash(&scope, requester, &intent, &bound);
        Ok(Prepared {
            review: Review {
                chain,
                requester: requester.map(str::to_owned),
                intent,
                route,
                summary,
                program,
                expected_effects,
                recipient,
                from_balance,
                fee,
                request_hash: hex::encode(hash),
            },
            hash,
            pins,
            body,
        })
    }

    /// Nothing the approval depends on moved (call right before signing).
    async fn recheck(&self, pins: &Pins) -> Result<()> {
        let core = self.core().context("not connected")?;
        for (signer, nonce) in &pins.nonces {
            let now = nonce_of(core, *signer).await?;
            if now != *nonce {
                return Err(Stale(format!("{signer} nonce is {now}, approved at {nonce}")).into());
            }
        }
        if let Some((program, image)) = pins.program {
            let now = verify::read_header(core, program).await?;
            if now.map(|h| h.image_id) != Some(image) {
                return Err(Stale(format!("program {program} was upgraded")).into());
            }
        }
        Ok(())
    }

    /// Sign and submit an approved public transaction. Returns the tx hash.
    pub async fn submit_public(&mut self, prepared: Prepared) -> Result<String> {
        let Prepared {
            review,
            pins,
            body: Body::Public(tx),
            ..
        } = prepared
        else {
            bail!("this transaction needs a proof first");
        };
        self.recheck(&pins).await?;
        let tx = self.repage(review, tx).await?;
        let core = self.core().context("not connected")?;
        let hash = core.sign_and_submit_public(tx).await.map_err(lez)?;
        Ok(hash.to_string())
    }

    /// A testimonial post names its stats page when it is built (by us or by
    /// the app), and a page holds 1000 authors. If the page filled while the
    /// user was approving, the program would refuse the post and still charge
    /// the fee, so post the same record to the open page instead. Only for
    /// the trusted testimonial program; author, text, claimed time and signer
    /// nonce stay the approved ones, and the fee cap may not rise.
    async fn repage(&mut self, review: Review, tx: PreparedPublicTx) -> Result<PreparedPublicTx> {
        let Some(check) = review
            .program
            .as_ref()
            .filter(|c| crate::testimonial::trusted(c))
        else {
            return Ok(tx);
        };
        let message = tx.message();
        let Ok(testimonial_core::Instruction::Post {
            submission,
            page,
            username,
            text,
            timestamp_ms,
        }) = borsh::from_slice(&message.instruction_data)
        else {
            return Ok(tx);
        };
        let program = message.program_account_id;
        let Some(author) = message.shard_selectors.first().map(|s| s.account_id) else {
            return Ok(tx);
        };
        let decoders = self.decoders();
        let core = self.core().context("not connected")?;
        let open = crate::testimonial::open_page(
            &crate::testimonial::pages(core, program, &submission).await?,
        );
        if open == page || program.to_string() != check.account {
            return Ok(tx);
        }
        let call = crate::testimonial::post_call(
            program,
            author,
            &submission,
            open,
            username.as_deref(),
            &text,
            timestamp_ms,
        )?;
        let built = prepare_call(core, &decoders, &call).await?;
        let cap = |f: &Fee| f.max_fee.as_deref().and_then(|f| f.parse::<u128>().ok());
        if built.summary.unknown
            || built.program.as_ref().map(|c| &c.image_id) != Some(&check.image_id)
            || cap(&built.fee) > cap(&review.fee)
        {
            return Err(Stale(format!(
                "testimonial page {page} filled while you approved; post again"
            ))
            .into());
        }
        match built.body {
            Body::Public(tx) => Ok(tx),
            Body::Private { .. } => bail!("a testimonial post is public"),
        }
    }

    /// Check the proof matches the approval, then sign and submit.
    pub async fn submit_proved(&mut self, pins: &Pins, proved: Proved) -> Result<String> {
        let message = proved.tx.message();
        let effects = decode::private_effects(message);
        if effects != proved.check.effects {
            return Err(Stale(format!(
                "the proof's public effects {} differ from the approved {}",
                serde_json::to_string(&effects).unwrap_or_default(),
                serde_json::to_string(&proved.check.effects).unwrap_or_default(),
            ))
            .into());
        }
        let nonces: Vec<u128> = message.nonces.iter().map(|n| n.0).collect();
        let approved: Vec<u128> = pins.nonces.iter().map(|(_, n)| *n).collect();
        if nonces != approved {
            return Err(Stale(format!(
                "the proof uses nonces {nonces:?}; approved {approved:?}"
            ))
            .into());
        }
        // A wallet-built transfer runs at most one program besides native.
        if message.program_image_claims.len() > usize::from(proved.check.program.is_some()) {
            return Err(Stale(format!(
                "the proof claims {} program images; approved {}",
                message.program_image_claims.len(),
                usize::from(proved.check.program.is_some())
            ))
            .into());
        }
        for claim in &message.program_image_claims {
            match (claim, proved.check.program) {
                (
                    ProgramImageClaim::Disclosed {
                        account_id,
                        image_id,
                    },
                    Some(pin),
                ) if (*account_id, *image_id) == pin => {}
                (ProgramImageClaim::Disclosed { account_id, .. }, _) => {
                    return Err(Stale(format!(
                        "the proof runs program {account_id}, which wasn't approved"
                    ))
                    .into());
                }
                // Some immutable program's image, not named: only acceptable
                // when the approved program is immutable (pinned by header).
                (ProgramImageClaim::Undisclosed { .. }, Some(_)) if pins.program.is_none() => {}
                (ProgramImageClaim::Undisclosed { .. }, _) => {
                    return Err(Stale(
                        "the proof runs an undisclosed program that wasn't approved".to_owned(),
                    )
                    .into());
                }
            }
        }
        self.recheck(pins).await?;
        let core = self.core().context("not connected")?;
        let (hash, secrets) = core.sign_and_submit_private(proved.tx).await.map_err(lez)?;
        let notes = secrets.into_iter().zip(proved.check.notes).collect();
        self.pending_note = Some((hash, notes));
        Ok(hash.to_string())
    }

    /// Wait until the transaction is in a block. For a private transaction
    /// sent from here, also record its notes so balances show at once.
    /// Returns the block, plus a warning if that local bookkeeping failed
    /// (the transaction is included either way; the next sync repairs it).
    pub async fn wait_included(&mut self, tx_hash: &str) -> Result<(u64, Option<String>)> {
        let hash = tx_hash.parse().context("tx hash")?;
        let pending = match self.pending_note.take() {
            Some(p) if p.0 == hash => Some(p),
            other => {
                self.pending_note = other;
                None
            }
        };
        let core = self.core_mut().context("not connected")?;
        let (tx, block) = match core.poll_transaction(hash).await {
            Ok(done) => done,
            Err(e) => {
                // Still in flight: keep the secrets for a later wait or sync.
                self.pending_note = pending;
                return Err(e);
            }
        };
        let mut warning = None;
        if let (common::transaction::LeeTransaction::PrivacyPreserving(tx), Some((h, notes))) =
            (&tx, pending)
            && h == hash
        {
            let mask: Vec<AccDecodeData> = notes
                .into_iter()
                .map(|(secret, account)| {
                    account.map_or(AccDecodeData::Skip, |a| AccDecodeData::Decode(secret, a))
                })
                .collect();
            if let Err(e) = core.decode_insert_privacy_preserving_transaction_results(tx, &mask) {
                warning = Some(format!(
                    "included; note not recorded yet ({e:#}); sync to see it"
                ));
            }
        }
        if let Err(e) = self.persist_now() {
            warning = Some(format!("included; saving the wallet failed ({e:#})"));
        }
        Ok((block, warning))
    }

    /// Balance of one of this wallet's accounts: native, or of `token` (a
    /// definition id). Public from the sequencer; private as synced.
    pub async fn balance_of(&mut self, account_id: &str, token: Option<&str>) -> Result<u128> {
        self.connect().await?;
        let (id, _) = parse_account(account_id)?;
        let core = self.core().context("not connected")?;
        let private = if core.get_account_public_signing_key(id).is_some() {
            false
        } else if core.private_account_state(id).is_some() {
            true
        } else {
            bail!("{id} is not an account of this wallet");
        };
        match token {
            None if !private => core.get_account_balance(id).await,
            None => Ok(decode_balance(
                own_shard(core, id, true, NATIVE_TOKEN_PROGRAM_ID)
                    .await?
                    .as_slice(),
            )
            .unwrap_or(0)),
            Some(def) => {
                let def = decode::account_id(def)?;
                let token = token_core::token_account_id();
                let h = holding(&own_shard(core, id, private, token).await?)?;
                let own = h
                    .filter(|h| h.definition_id() == def)
                    .map_or(0, |h| holding_amount(&h));
                if private {
                    return Ok(own);
                }
                // A public account also holds tokens in its token account (ATA).
                let ata = crate::tokens::ata_of(id, def);
                let in_ata = holding(&own_shard(core, ata, false, token).await?)?
                    .filter(|h| h.definition_id() == def)
                    .map_or(0, |h| holding_amount(&h));
                Ok(own.saturating_add(in_ata))
            }
        }
    }
}

/// Route-independent result of building.
struct Built {
    route: Option<Route>,
    summary: Summary,
    program: Option<ProgramCheck>,
    expected_effects: Vec<PublicEffect>,
    recipient: Option<String>,
    from_balance: u128,
    fee: Fee,
    pins: Pins,
    body: Body,
    bound: Vec<u8>,
    chosen_identifier: Option<String>,
}

async fn prepare_transfer(
    core: &WalletCore,
    decoders: &Decoders,
    intent: Intent,
) -> Result<(Intent, Built)> {
    let Intent::Transfer {
        from,
        to,
        amount,
        token,
        to_keys,
    } = intent
    else {
        unreachable!("transfer intent");
    };
    // A `.lez` name (LNS) becomes the private keys or the public account it publishes,
    // here, so routing, approval and the request hash all see the resolved recipient.
    let (to, mut to_keys) = match to {
        Some(name) if to_keys.is_none() && crate::names::is_name(&name) => {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
            match crate::names::resolve(core, &name, now_ms).await? {
                crate::names::NameTarget::Private { npk, vpk } => (
                    None,
                    Some(RecipientKeys {
                        npk: hex::encode(npk),
                        vpk: hex::encode(vpk),
                        identifier: None,
                    }),
                ),
                crate::names::NameTarget::Public(account) => {
                    (Some(format!("Public/{account}")), None)
                }
            }
        }
        to => (to, to_keys),
    };
    ensure!(amount > 0, "amount must be more than zero");
    let (from_id, _) = parse_account(&from)?;
    let to_parsed = to.as_deref().map(parse_account).transpose()?;
    let chosen_identifier = match to_keys.as_mut() {
        Some(k) if k.identifier.is_none() => {
            let mut bytes = [0u8; 32];
            chacha20poly1305::aead::rand_core::RngCore::fill_bytes(
                &mut chacha20poly1305::aead::OsRng,
                &mut bytes,
            );
            let id = Identifier::new(bytes).to_string();
            k.identifier = Some(id.clone());
            Some(id)
        }
        _ => None,
    };
    let r = resolve(core, from_id, to_parsed, to_keys.as_ref())?;
    let from_private = r.route.spends_private();

    // Asset: program, instruction, descriptor, program deps.
    let (program, descriptor, instruction, deps, check) = match token.as_deref() {
        None => (
            NATIVE_TOKEN_PROGRAM_ID,
            None,
            Program::serialize_instruction(native_token::Instruction::Transfer { amount })
                .map_err(|e| anyhow::anyhow!("encode transfer: {e:?}"))?,
            Some(ProgramWithDependencies::native()),
            None,
        ),
        Some(def) => {
            let def = decode::account_id(def)?;
            let token_program = token_core::token_account_id();
            let held = holding(&own_shard(core, from_id, from_private, token_program).await?)?;
            if !from_private
                && held
                    .as_ref()
                    .is_none_or(|h| h.definition_id() != def || holding_amount(h) < amount)
            {
                // Not (enough) in its own slot: its associated token account.
                if r.route == Route::Public {
                    let built =
                        prepare_ata_transfer(core, decoders, from_id, r.to_id, def, amount).await?;
                    let intent = Intent::Transfer {
                        from,
                        to,
                        amount,
                        token,
                        to_keys,
                    };
                    return Ok((intent, built));
                }
            }
            let held = held.with_context(|| format!("{from_id} holds no tokens"))?;
            ensure!(
                held.definition_id() == def,
                "{from_id} holds a different token ({}), not {def}.",
                held.definition_id()
            );
            let descriptor = TokenDescriptor {
                definition_id: def,
                kind: held.kind(),
            };
            let check = decoders.check(core, token_program).await?;
            (
                token_program,
                Some(descriptor),
                Program::serialize_instruction(token_core::Instruction::Transfer {
                    amount_to_transfer: amount,
                    descriptor,
                })
                .map_err(|e| anyhow::anyhow!("encode token transfer: {e:?}"))?,
                // LEZ v0.3.1 no longer ships the token program's ELF (it left
                // the builtin set), so a private token send can't be proved
                // here; public sends only need the program's address.
                None,
                Some(check),
            )
        }
    };
    let asset = descriptor.as_ref().map_or(Asset::Native, Asset::token);
    let from_balance = match &descriptor {
        None if !from_private => core.get_account_balance(from_id).await?,
        _ => {
            let bytes = own_shard(core, from_id, from_private, program).await?;
            if descriptor.is_some() {
                holding(&bytes)?.map_or(0, |h| holding_amount(&h))
            } else {
                decode_balance(&bytes).unwrap_or(0)
            }
        }
    };
    let mention = |ident: AccountIdentity| {
        if program == NATIVE_TOKEN_PROGRAM_ID {
            ident.balance()
        } else {
            ident.select_program_shard(program)
        }
    };
    // A public fungible token send lands in the recipient's token account.
    let lands_in = match (&descriptor, r.route) {
        (Some(d), Route::Public) => {
            Some(recipient_holder(core, r.to_id, d.definition_id, d.kind).await?)
                .filter(|h| *h != r.to_id)
        }
        _ => None,
    };
    let to_ident = match lands_in {
        Some(holder) => AccountIdentity::PublicNoSign(holder),
        None => r.to.clone(),
    };
    let mentions = vec![mention(r.from.clone()), mention(to_ident)];
    let program_pin = check
        .as_ref()
        .and_then(|c| (!c.immutable).then_some((program, c.image_id_words)));
    let proof_program = check.as_ref().map(|c| (program, c.image_id_words));

    let built = if r.route == Route::Public {
        let tx = core
            .prepare_public(
                mentions,
                instruction,
                program,
                None,
                preflight(program, descriptor, amount),
            )
            .await
            .map_err(lez)?;
        let message = tx.message();
        let native_out = if descriptor.is_none() { amount } else { 0 };
        let sender_native = if descriptor.is_none() {
            from_balance
        } else {
            core.get_account_balance(from_id).await?
        };
        check_fee(core, message, from_id, sender_native, native_out).await?;
        let nonces = signer_nonces(&tx)?;
        let mut summary = decode::public(message, decoders);
        summary.signers = nonces.iter().map(|(id, _)| id.to_string()).collect();
        if let Some(holder) = lands_in {
            summary.lines.extend(
                lands_in_lines(core, r.to_id, holder, token_core::token_account_id()).await,
            );
        }
        Built {
            route: Some(Route::Public),
            summary,
            program: check,
            expected_effects: vec![],
            recipient: Some(r.to_id.to_string()),
            from_balance,
            fee: fee_of(message),
            pins: Pins {
                nonces,
                program: program_pin,
            },
            // The message commits to program, accounts, nonces, data and fee.
            bound: message.hash().to_vec(),
            body: Body::Public(tx),
            chosen_identifier,
        }
    } else {
        // The witness reads the sender's nonce inside prepare_private; the
        // same value before and after means that is the one it used.
        let nonce_before = if r.route == Route::Shield {
            Some(nonce_of(core, from_id).await?)
        } else {
            None
        };
        let deps = deps.context(
            "private token transfers aren't available: LEZ v0.3.1 no longer ships the token program",
        )?;
        let tx = core
            .prepare_private(
                mentions,
                instruction,
                &deps,
                preflight(program, descriptor, amount),
            )
            .await
            .map_err(lez)?;
        let (nonces, effects) = match r.route {
            Route::Shield => (
                {
                    let after = nonce_of(core, from_id).await?;
                    ensure!(
                        nonce_before == Some(after),
                        "{from_id} changed while this was prepared; try again"
                    );
                    vec![(from_id, after)]
                },
                vec![PublicEffect::Debit {
                    account: from_id.to_string(),
                    asset: asset.clone(),
                    amount,
                }],
            ),
            Route::Unshield => (
                vec![],
                vec![PublicEffect::Credit {
                    account: r.to_id.to_string(),
                    asset: asset.clone(),
                    amount,
                }],
            ),
            _ => (vec![], vec![]),
        };
        let (title, detail) = match r.route {
            Route::Shield => ("Shield", "into your private account"),
            Route::Unshield => (
                "Unshield",
                "to a public account (the amount becomes visible)",
            ),
            _ => ("Send privately", "nothing about it is public"),
        };
        let summary = Summary {
            title: title.to_owned(),
            program: program.to_string(),
            lines: vec![format!("{amount} {detail}"), format!("to {}", r.to_id)],
            outflows: vec![Flow {
                account: from_id.to_string(),
                asset: asset.clone(),
                amount,
            }],
            inflows: vec![Flow {
                account: r.to_id.to_string(),
                asset,
                amount,
            }],
            ..Summary::default()
        };
        let mut bound = serde_json::to_vec(&effects).unwrap_or_default();
        bound.extend_from_slice(r.to_id.to_string().as_bytes());
        for (_, n) in &nonces {
            bound.extend_from_slice(&n.to_le_bytes());
        }
        Built {
            route: Some(r.route),
            summary,
            program: check,
            expected_effects: effects.clone(),
            recipient: Some(r.to_id.to_string()),
            from_balance,
            fee: Fee::default(),
            pins: Pins {
                nonces,
                program: program_pin,
            },
            bound,
            body: Body::Private {
                tx: Box::new(tx),
                check: ProofCheck {
                    effects,
                    notes: r.notes,
                    program: proof_program,
                },
            },
            chosen_identifier,
        }
    };
    let intent = Intent::Transfer {
        from,
        to,
        amount,
        token,
        to_keys,
    };
    Ok((intent, built))
}

async fn prepare_call(core: &WalletCore, decoders: &Decoders, intent: &Intent) -> Result<Built> {
    let Intent::Call {
        from,
        program,
        accounts,
        data,
    } = intent
    else {
        unreachable!("call intent");
    };
    let (from, _) = parse_account(from)?;
    let program = decode::account_id(program)?;
    ensure!(
        core.get_account_public_signing_key(from).is_some(),
        "{from} is not a public account of this wallet"
    );
    ensure!(
        !accounts.is_empty() && accounts.len() <= MAX_CALL_ACCOUNTS,
        "a call takes 1–{MAX_CALL_ACCOUNTS} accounts"
    );
    let data = STANDARD.decode(data).context("call data must be base64")?;
    ensure!(
        data.len() <= MAX_CALL_DATA,
        "call data is over {MAX_CALL_DATA} bytes"
    );
    let mut mentions = Vec::with_capacity(accounts.len());
    for a in accounts {
        let (id, marked) = parse_account(&a.account)?;
        ensure!(
            marked != Some(true),
            "a public call can't take private account {id}"
        );
        let shard = a
            .shard
            .as_deref()
            .map(decode::account_id)
            .transpose()?
            .unwrap_or(program);
        let ident = if a.signer {
            ensure!(
                core.get_account_public_signing_key(id).is_some(),
                "{id} must sign, but this wallet doesn't hold its key"
            );
            AccountIdentity::Public(id)
        } else {
            AccountIdentity::PublicNoSign(id)
        };
        mentions.push(ident.select_program_shard(shard));
    }
    let check = if program == NATIVE_TOKEN_PROGRAM_ID {
        None
    } else {
        Some(decoders.check(core, program).await?)
    };
    let tx = core
        .prepare_public(mentions, data, program, Some(from), |_| Ok(()))
        .await
        .map_err(lez)?;
    let message = tx.message();
    // The testimonial decoder follows the image (in a header nobody can
    // change), never the address.
    let decoders = match &check {
        Some(c) if crate::testimonial::trusted(c) => {
            if let Ok(testimonial_core::Instruction::Post { submission, .. }) =
                borsh::from_slice(&message.instruction_data)
            {
                ensure!(
                    crate::testimonial::record(core, program, &submission, from)
                        .await?
                        .is_none(),
                    "{from} already posted a testimonial for {submission}"
                );
            }
            decoders.clone().with(program, decode::Decoder::Testimonial)
        }
        _ => decoders.clone(),
    };
    let summary = decode::public(message, &decoders);
    let from_balance = core.get_account_balance(from).await?;
    let native_out = summary
        .outflows
        .iter()
        .filter(|f| f.asset == Asset::Native && f.account == from.to_string())
        .fold(0u128, |a, f| a.saturating_add(f.amount));
    check_fee(core, message, from, from_balance, native_out).await?;
    let nonces = signer_nonces(&tx)?;
    let mut summary = summary;
    summary.signers = nonces.iter().map(|(id, _)| id.to_string()).collect();
    let program_pin = check
        .as_ref()
        .and_then(|c| (!c.immutable).then_some((program, c.image_id_words)));
    Ok(Built {
        route: None,
        summary,
        program: check,
        expected_effects: vec![],
        recipient: None,
        from_balance,
        fee: fee_of(message),
        pins: Pins {
            nonces,
            program: program_pin,
        },
        bound: message.hash().to_vec(),
        body: Body::Public(tx),
        chosen_identifier: None,
    })
}

async fn prepare_testimonial(
    core: &WalletCore,
    decoders: &Decoders,
    intent: Intent,
    chain: &str,
) -> Result<(Intent, Built)> {
    use crate::testimonial;
    let Intent::Testimonial {
        from,
        program,
        submission,
        username,
        text,
    } = intent
    else {
        unreachable!("testimonial intent");
    };
    // Input first: it needs no chain reads.
    testimonial_core::check_post(&submission, username.as_deref(), &text)?;
    let (author, marked) = parse_account(&from)?;
    ensure!(
        marked != Some(true) && core.get_account_public_signing_key(author).is_some(),
        "{author} is not a public account of this wallet (the author is shown on-chain, so it must be public)"
    );
    let program = match program {
        Some(p) => p,
        None => testimonial::default_program(chain)
            .with_context(|| format!("no testimonial program is known for {chain}; name one"))?,
    };
    let program_id = decode::account_id(&program)?;
    testimonial::check_program(core, program_id).await?;
    let page = testimonial::open_page(&testimonial::pages(core, program_id, &submission).await?);
    ensure!(
        testimonial::record(core, program_id, &submission, author)
            .await?
            .is_none(),
        "{author} already posted a testimonial for {submission}"
    );
    let call = testimonial::post_call(
        program_id,
        author,
        &submission,
        page,
        username.as_deref(),
        &text,
        testimonial::now_ms(),
    )?;
    let built = prepare_call(core, decoders, &call).await?;
    ensure!(
        !built.summary.unknown,
        "the testimonial post did not decode"
    );
    Ok((
        Intent::Testimonial {
            from,
            program: Some(program),
            submission,
            username,
            text,
        },
        built,
    ))
}

/// Send `def` out of `owner`'s associated token account (public route).
/// Where a public token send lands. A fungible token goes to the
/// recipient's associated token account for it (Phantom's model; LEZ
/// creates it on arrival, no recipient signature needed), so any account can
/// hold any number of tokens. Two exceptions keep sending straight to `to`:
/// it already holds this token in its own slot (an older-style holder, or a
/// token account pasted as the recipient: never "the ATA of an ATA"), or the
/// token isn't fungible (NFTs have their own route, stage N2).
pub(crate) async fn recipient_holder(
    core: &WalletCore,
    to: AccountId,
    def: AccountId,
    kind: TokenKind,
) -> Result<AccountId> {
    if kind != TokenKind::Fungible {
        return Ok(to);
    }
    let own = holding(&own_shard(core, to, false, token_core::token_account_id()).await?)?;
    if own.is_some_and(|h| h.definition_id() == def) {
        return Ok(to);
    }
    Ok(crate::tokens::ata_of(to, def))
}

/// The review lines that say where the tokens land, and a warning when the
/// recipient may itself be a token account.
async fn lands_in_lines(
    core: &WalletCore,
    to: AccountId,
    holder: AccountId,
    token_program: AccountId,
) -> Vec<String> {
    let exists = own_shard(core, holder, false, token_program)
        .await
        .is_ok_and(|b| !b.is_empty());
    let short = decode::shorten_ids(&holder.to_string());
    let mut lines = vec![if exists {
        format!("Arrives in the recipient's token account {short}")
    } else {
        format!("Creates the recipient's token account {short} for this token")
    }];
    if maybe_token_account(core, to).await {
        lines.push(format!(
            "Check the recipient: {} holds a token and has never signed, so it may be someone's token account. Tokens sent to it would be stuck; ask for their wallet address.",
            decode::shorten_ids(&to.to_string())
        ));
    }
    lines
}

/// `id` holds a token in its own slot and has never signed: it may be
/// someone's token account (an ATA, which no key controls) pasted as a
/// recipient. Anything sent to it, or to its own token accounts, is stuck.
/// (A fresh key account that was sent a token looks the same: a warning,
/// not a refusal.)
pub(crate) async fn maybe_token_account(core: &WalletCore, id: AccountId) -> bool {
    let holds = own_shard(core, id, false, token_core::token_account_id())
        .await
        .ok()
        .and_then(|b| holding(&b).ok().flatten())
        .is_some();
    holds && nonce_of(core, id).await.is_ok_and(|n| n == 0)
}

async fn prepare_ata_transfer(
    core: &WalletCore,
    decoders: &Decoders,
    owner: AccountId,
    to: AccountId,
    def: AccountId,
    amount: u128,
) -> Result<Built> {
    let token_program = token_core::token_account_id();
    let ata_program = associated_token_account_core::ata_account_id();
    let ata = crate::tokens::ata_of(owner, def);
    let held = holding(&own_shard(core, ata, false, token_program).await?)?
        .filter(|h| h.definition_id() == def);
    let from_balance = held.as_ref().map_or(0, holding_amount);
    if from_balance < amount {
        // Its own slot may hold some too (the caller found too little there).
        let own = holding(&own_shard(core, owner, false, token_program).await?)?
            .filter(|h| h.definition_id() == def)
            .map_or(0, |h| holding_amount(&h));
        if own > 0 && from_balance > 0 {
            bail!(
                "{owner} holds this token in two places: {own} in the account itself and {from_balance} in its token account. One send comes from one of them, so send at most {}.",
                own.max(from_balance)
            );
        }
        bail!(
            "Not enough of this token: {owner} holds {}.",
            own.max(from_balance)
        );
    }
    let held = held.with_context(|| format!("{owner} holds no token {def}"))?;
    // Tokens land in the recipient's own token account for this token (its
    // ATA), created on arrival; NFTs keep their own route (stage N2).
    let lands_in = recipient_holder(core, to, def, held.kind()).await?;
    if lands_in == to {
        let recipient = holding(&own_shard(core, to, false, token_program).await?)?;
        ensure!(
            recipient.is_none_or(|h| h.definition_id() == def),
            "The recipient's account already holds a different token, and a LEZ account holds one token at a time."
        );
    }
    let check = decoders.check(core, ata_program).await?;
    let data =
        Program::serialize_instruction(associated_token_account_core::Instruction::Transfer {
            token_program_id: token_program,
            descriptor: TokenDescriptor {
                definition_id: def,
                kind: held.kind(),
            },
            amount,
        })
        .map_err(|e| anyhow::anyhow!("encode ATA transfer: {e:?}"))?;
    let tx = core
        .prepare_public(
            vec![
                AccountIdentity::Public(owner).balance(),
                AccountIdentity::PublicNoSign(ata).select_program_shard(token_program),
                AccountIdentity::PublicNoSign(lands_in).select_program_shard(token_program),
            ],
            data,
            ata_program,
            None,
            |_| Ok(()),
        )
        .await
        .map_err(lez)?;
    let message = tx.message();
    let owner_native = core.get_account_balance(owner).await?;
    check_fee(core, message, owner, owner_native, 0).await?;
    let nonces = signer_nonces(&tx)?;
    let mut summary = decode::public(message, decoders);
    summary.signers = nonces.iter().map(|(id, _)| id.to_string()).collect();
    if lands_in != to {
        summary
            .lines
            .extend(lands_in_lines(core, to, lands_in, token_program).await);
    }
    Ok(Built {
        route: Some(Route::Public),
        summary,
        program: Some(check.clone()),
        expected_effects: vec![],
        recipient: Some(to.to_string()),
        from_balance,
        fee: fee_of(message),
        pins: Pins {
            nonces,
            program: (!check.immutable).then_some((ata_program, check.image_id_words)),
        },
        bound: message.hash().to_vec(),
        body: Body::Public(tx),
        chosen_identifier: None,
    })
}

/// Each signer with the nonce the message carries for it (same order:
/// LEZ lists signers' nonces in signature order, a co-signing payer last).
fn signer_nonces(tx: &PreparedPublicTx) -> Result<Vec<(AccountId, u128)>> {
    ensure!(
        tx.signers().len() == tx.message().nonces.len(),
        "the message carries {} nonces for {} signers",
        tx.message().nonces.len(),
        tx.signers().len()
    );
    Ok(tx
        .signers()
        .iter()
        .map(|s| match s {
            TxSigner::Local(id) => *id,
            TxSigner::Keycard { account_id, .. } => *account_id,
        })
        .zip(tx.message().nonces.iter().map(|n| n.0))
        .collect())
}

impl Prepared {
    /// Split off the proving work (runs without the session: CPU only).
    pub fn into_proving(self) -> Result<(ProvingJob, Pins)> {
        match self.body {
            Body::Private { tx, check } => Ok((ProvingJob { tx, check }, self.pins)),
            Body::Public(_) => bail!("a public transaction has nothing to prove"),
        }
    }
}

/// The proving half of a private transaction. `run` blocks for minutes.
pub struct ProvingJob {
    tx: Box<PreparedPrivateTx>,
    check: ProofCheck,
}

impl ProvingJob {
    pub fn run(self) -> Result<Proved> {
        let tx = self.tx.prove().map_err(lez)?;
        Ok(Proved {
            tx,
            check: self.check,
        })
    }
}

pub(crate) type NoteSecrets = Vec<(SharedSecretKey, Option<AccountId>)>;
