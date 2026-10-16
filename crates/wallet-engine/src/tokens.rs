//! Token holdings.
//!
//! In 0.3 every account has one shard per program, so an account's own
//! token shard holds at most one token. Other tokens sit in associated token
//! accounts (ATAs): PDAs of the ATA program keyed by (owner, definition).
//! Holdings are therefore found in two places: each of our accounts' token
//! shard (public: live; private: as synced), and the ATA of each of our
//! public accounts for every token this wallet tracks in the zone.

use anyhow::{Context as _, Result};
use associated_token_account_core::{compute_ata_seed, get_associated_token_account_id};
use lee::{AccountId, ProgramShardSelector};
use serde::Serialize;
use token_core::{TokenDefinition, TokenHolding, TokenKind};
use wallet::WalletCore;

use crate::{
    decode,
    session::{AccountKind, Session},
    tx::{amount, holding_amount, own_shard},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// The account's own token shard.
    Account,
    /// Its associated token account.
    Ata,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Holding {
    /// Our account (the owner, for an ATA).
    pub account: String,
    pub private: bool,
    pub via: Via,
    /// Where the tokens sit (the ATA address, or the account itself).
    pub holder: String,
    pub definition: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub kind: &'static str,
    #[serde(with = "amount")]
    pub amount: u128,
    /// How far to trust it, and how to show it.
    #[serde(flatten)]
    pub info: TokenInfo,
}

/// What the wallet knows about a token definition beyond its name.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfo {
    pub tier: crate::trust::Tier,
    /// Why it looks like spam ("Name contains a link").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spam_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// Display decimals; `None`: unknown (amounts show as whole units).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
    /// Where `decimals` came from: "list", "you" or "unknown".
    pub decimals_source: &'static str,
    pub pinned: bool,
}

impl Session {
    /// Tier, symbol, decimals and pin for `definition` named `name` here.
    pub fn token_info(&self, definition: &str, name: Option<&str>) -> TokenInfo {
        let chain = self.zone().chain.clone();
        let hidden = self.hidden_tokens();
        let added = self.tracked_tokens();
        let (tier, reason) = crate::trust::tier(
            &chain,
            definition,
            name,
            &crate::trust::Choices {
                hidden: &hidden,
                added: &added,
            },
        );
        let listed = crate::trust::find(&chain, definition);
        let (decimals, decimals_source) = match (listed, self.token_decimals(definition)) {
            (Some(l), _) => (Some(l.decimals), "list"),
            (None, Some(d)) => (Some(d), "you"),
            (None, None) => (None, "unknown"),
        };
        TokenInfo {
            tier,
            spam_reason: reason.map(|r| r.text()),
            symbol: listed.map(|l| l.symbol.clone()),
            decimals,
            decimals_source,
            pinned: self.pinned_tokens().iter().any(|p| p == definition),
        }
    }

    /// A definition's name, read once per session and network (definitions
    /// can't change; the same address on another network is another token).
    pub async fn cached_definition_name(&mut self, definition: AccountId) -> Option<String> {
        let key = format!("{}:{definition}", self.zone().id);
        if let Some(n) = self.token_names.get(&key) {
            return n.clone();
        }
        let core = self.core()?;
        let name = definition_name(core, definition).await;
        // A failed read isn't cached, so the next sync tries again.
        if name.is_some() {
            self.token_names.insert(key, name.clone());
        }
        name
    }
}

const fn kind_name(k: TokenKind) -> &'static str {
    match k {
        TokenKind::Fungible => "fungible",
        TokenKind::NftMaster => "nft_master",
        TokenKind::NftPrintedCopy => "nft_copy",
    }
}

/// The ATA address of `owner` for `definition` under the builtin programs.
pub fn ata_of(owner: AccountId, definition: AccountId) -> AccountId {
    get_associated_token_account_id(
        &associated_token_account_core::ata_account_id(),
        &compute_ata_seed(owner, definition, token_core::token_account_id()),
    )
}

/// A token definition's name, read from its definition account.
pub async fn definition_name(core: &WalletCore, definition: AccountId) -> Option<String> {
    let token = token_core::token_account_id();
    let account = core
        .get_account_view(ProgramShardSelector::new(definition, token))
        .await
        .ok()?;
    match borsh::from_slice::<TokenDefinition>(account.data.shard(token).as_ref()).ok()? {
        TokenDefinition::Fungible { name, .. } | TokenDefinition::NonFungible { name, .. } => {
            Some(name)
        }
    }
}

async fn public_holding(core: &WalletCore, id: AccountId) -> Result<Option<TokenHolding>> {
    let token = token_core::token_account_id();
    let account = core
        .get_account_view(ProgramShardSelector::new(id, token))
        .await?;
    let shard = account.data.shard(token);
    if shard.is_empty() {
        return Ok(None);
    }
    Ok(borsh::from_slice(shard.as_ref()).ok())
}

impl Session {
    /// Every token this wallet holds in the current zone.
    pub async fn holdings(&mut self) -> Result<Vec<Holding>> {
        self.connect().await?;
        let accounts = self.accounts()?;
        // Tokens to look for in each account's token account (ATA): the
        // ones the user added and the ones the chain scan saw arrive.
        let mut wanted: Vec<String> = self.tracked_tokens();
        if let Ok(record) = self.load_history()
            && let Some(seen) = record.get("tokens").and_then(serde_json::Value::as_object)
        {
            wanted.extend(seen.keys().cloned());
        }
        wanted.sort();
        wanted.dedup();
        let tracked: Vec<AccountId> = wanted
            .iter()
            .filter_map(|t| decode::account_id(t).ok())
            .collect();
        let core = self.core().context("not connected")?;
        let token = token_core::token_account_id();
        let mut out = Vec::new();
        for a in &accounts {
            let id = decode::account_id(&a.account_id)?;
            let private = a.kind == AccountKind::Private;
            let own = if private {
                own_shard(core, id, true, token)
                    .await
                    .ok()
                    .and_then(|b| borsh::from_slice::<TokenHolding>(&b).ok())
            } else {
                // One unreadable account doesn't hide the rest.
                public_holding(core, id).await.ok().flatten()
            };
            if let Some(h) = own {
                out.push((a.account_id.clone(), private, Via::Account, id, h));
            }
            if private {
                continue;
            }
            for def in &tracked {
                let ata = ata_of(id, *def);
                if let Some(h) = public_holding(core, ata).await.ok().flatten() {
                    out.push((a.account_id.clone(), false, Via::Ata, ata, h));
                }
            }
        }
        let mut holdings = Vec::with_capacity(out.len());
        for (account, private, via, holder, h) in out {
            let def = h.definition_id();
            let name = self.cached_definition_name(def).await;
            let info = self.token_info(&def.to_string(), name.as_deref());
            holdings.push(Holding {
                account,
                private,
                via,
                holder: holder.to_string(),
                definition: h.definition_id().to_string(),
                name,
                kind: kind_name(h.kind()),
                amount: holding_amount(&h),
                info,
            });
        }
        Ok(holdings)
    }
}

/// A `Call` creating a fungible token: `definition` (a fresh public account
/// of ours) becomes its definition, and all `supply` goes to `holder`.
pub fn create_token_intent(
    holder: &str,
    definition: &str,
    name: &str,
    supply: u128,
) -> Result<crate::tx::Intent> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    anyhow::ensure!(
        !name.is_empty() && name.len() <= 32,
        "token name must be 1–32 bytes"
    );
    let data = borsh::to_vec(&token_core::Instruction::NewFungibleDefinition {
        name: name.to_owned(),
        total_supply: supply,
    })?;
    let signer = |account: &str| crate::tx::CallAccount {
        account: account.to_owned(),
        shard: None,
        signer: true,
    };
    Ok(crate::tx::Intent::Call {
        from: holder.to_owned(),
        program: token_core::token_account_id().to_string(),
        accounts: vec![signer(definition), signer(holder)],
        data: STANDARD.encode(data),
    })
}

/// What a token-program account holds, read live.
pub enum TokenShard {
    Empty,
    Definition(TokenDefinition),
    Holding(TokenHolding),
    Metadata(token_core::TokenMetadata),
    Unreadable,
}

/// Read `id`'s token-program shard.
pub async fn read_token_shard(core: &WalletCore, id: AccountId) -> Result<TokenShard> {
    let token = token_core::token_account_id();
    let account = core
        .get_account_view(ProgramShardSelector::new(id, token))
        .await?;
    let shard = account.data.shard(token);
    if shard.is_empty() {
        return Ok(TokenShard::Empty);
    }
    let bytes = shard.as_ref();
    Ok(if let Ok(d) = borsh::from_slice::<TokenDefinition>(bytes) {
        TokenShard::Definition(d)
    } else if let Ok(h) = borsh::from_slice::<TokenHolding>(bytes) {
        TokenShard::Holding(h)
    } else if let Ok(m) = borsh::from_slice::<token_core::TokenMetadata>(bytes) {
        TokenShard::Metadata(m)
    } else {
        TokenShard::Unreadable
    })
}

/// A token found by its ID, for the "Add a token" preview and token details.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenPreview {
    pub definition: String,
    pub name: String,
    /// "fungible" (collections answer [`LookupProblem::Collection`]).
    pub kind: &'static str,
    #[serde(with = "amount")]
    pub total_supply: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_id: Option<String>,
    /// The metadata (only for Verified and Added tokens: decision D6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetadataView>,
    /// This wallet's balance across its accounts (own slots and ATAs).
    #[serde(with = "amount")]
    pub your_balance: u128,
    /// Already in the list as Added or Verified.
    pub already_added: bool,
    /// It looks like a verified token (or LGO): the real one's name and ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imitates: Option<Imitated>,
    /// "Logos Kit token list v0.1.0" for Verified tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listed_in: Option<String>,
    #[serde(flatten)]
    pub info: TokenInfo,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataView {
    pub standard: &'static str,
    pub uri: String,
    pub creators: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imitated {
    pub name: String,
    /// The real token's ID (`None` for LGO, the network's own coin).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
}

/// Why an ID can't be added as a token, in the words the tray shows.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "problem", rename_all = "snake_case")]
pub enum LookupProblem {
    /// "That isn't a valid ID."
    Invalid,
    /// "Nothing exists at this ID on <network>."
    Nothing,
    /// "This is a token account. Paste the token's ID instead." (+ its token)
    Holding { definition: String },
    /// "This is a personal address, not a token."
    Personal,
    /// "This is an NFT collection. It will appear under Collectibles."
    Collection { name: String },
    /// "This account holds something the wallet can't read as a token."
    Unreadable,
}

impl LookupProblem {
    pub fn text(&self, network: &str) -> String {
        match self {
            Self::Invalid => "That isn't a valid ID.".to_owned(),
            Self::Nothing => format!("Nothing exists at this ID on {network}."),
            Self::Holding { .. } => {
                "This is a token account. Paste the token's ID instead.".to_owned()
            }
            Self::Personal => "This is a personal address, not a token.".to_owned(),
            Self::Collection { .. } => {
                "This is an NFT collection. It will appear under Collectibles.".to_owned()
            }
            Self::Unreadable => {
                "This account holds something the wallet can't read as a token.".to_owned()
            }
        }
    }
}

impl Session {
    /// Look a token up by its ID: a preview, or why the ID isn't a token.
    pub async fn lookup_token(
        &mut self,
        id: &str,
    ) -> Result<std::result::Result<TokenPreview, LookupProblem>> {
        self.connect().await?;
        let Ok(id) = decode::account_id(id.trim()) else {
            return Ok(Err(LookupProblem::Invalid));
        };
        let core = self.core().context("not connected")?;
        let shard = read_token_shard(core, id).await?;
        let def = match shard {
            TokenShard::Definition(TokenDefinition::Fungible {
                name,
                total_supply,
                metadata_id,
            }) => (name, total_supply, metadata_id),
            TokenShard::Definition(TokenDefinition::NonFungible { name, .. }) => {
                return Ok(Err(LookupProblem::Collection { name }));
            }
            TokenShard::Holding(h) => {
                return Ok(Err(LookupProblem::Holding {
                    definition: h.definition_id().to_string(),
                }));
            }
            TokenShard::Metadata(_) | TokenShard::Unreadable => {
                return Ok(Err(LookupProblem::Unreadable));
            }
            TokenShard::Empty => {
                let funded = core.get_account_balance(id).await.unwrap_or(0) > 0;
                let ours = self
                    .accounts()?
                    .iter()
                    .any(|a| a.account_id == id.to_string());
                return Ok(Err(if funded || ours {
                    LookupProblem::Personal
                } else {
                    LookupProblem::Nothing
                }));
            }
        };
        let (name, total_supply, metadata_id) = def;
        let definition = id.to_string();
        self.token_names.insert(
            format!("{}:{definition}", self.zone().id),
            Some(name.clone()),
        );
        let info = self.token_info(&definition, Some(&name));
        let chain = self.zone().chain.clone();
        let trusted = matches!(
            info.tier,
            crate::trust::Tier::Verified | crate::trust::Tier::Added
        );
        let metadata = match (trusted, metadata_id) {
            (true, Some(m)) => {
                match read_token_shard(self.core().context("not connected")?, m).await {
                    Ok(TokenShard::Metadata(m)) => Some(MetadataView {
                        standard: match m.standard {
                            token_core::MetadataStandard::Simple => "simple",
                            token_core::MetadataStandard::Expanded => "expanded",
                        },
                        uri: m.uri,
                        creators: m.creators,
                    }),
                    _ => None,
                }
            }
            _ => None,
        };
        let your_balance = self
            .holdings()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|h| h.definition == definition)
            .fold(0u128, |a, h| a.saturating_add(h.amount));
        let listed_in = crate::trust::find(&chain, &definition).map(|_| crate::trust::list_label());
        // A listed token looks like itself.
        let imitates = match crate::trust::spam(&chain, &name).filter(|_| listed_in.is_none()) {
            Some(crate::trust::SpamReason::Lookalike { of }) => Some(Imitated {
                definition: crate::trust::listed(&chain)
                    .into_iter()
                    .find(|t| t.name == of || t.symbol == of)
                    .map(|t| t.address.clone()),
                name: of,
            }),
            _ => None,
        };
        Ok(Ok(TokenPreview {
            already_added: matches!(
                info.tier,
                crate::trust::Tier::Verified | crate::trust::Tier::Added
            ),
            definition,
            name,
            kind: "fungible",
            total_supply,
            metadata_id: metadata_id.map(|m| m.to_string()),
            metadata,
            your_balance,
            imitates,
            listed_in,
            info,
        }))
    }

    /// Add a token by ID (shows in the main list; never raises it to
    /// Verified). `decimals` for tokens the list doesn't describe.
    pub fn add_token(&mut self, definition: &str, decimals: Option<u8>) -> Result<()> {
        decode::account_id(definition)?;
        self.track_token(definition)?;
        self.set_token_hidden(definition, false)?;
        if decimals.is_some() {
            self.set_token_decimals(definition, decimals)?;
        }
        Ok(())
    }
}

/// `raw` base units with `decimals`, trailing zeros trimmed ("1.5").
pub fn format_units(raw: u128, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let digits = format!("{raw:0>width$}", width = usize::from(decimals) + 1);
    let (whole, frac) = digits.split_at(digits.len() - usize::from(decimals));
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        whole.to_owned()
    } else {
        format!("{whole}.{frac}")
    }
}
