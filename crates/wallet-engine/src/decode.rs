//! Decoders: what a transaction does, read from the exact bytes that get signed.
//!
//! The approval sheet never trusts an app's description of a transaction. It
//! shows a [`Summary`] decoded here from the message itself: who pays, what
//! leaves which account, and any use of a token authority (LEZ has no
//! allowances, but a token definition's key controls minting, so using it is
//! flagged the way EVM wallets flag unlimited approvals). A program without a
//! decoder yields `unknown: true`, which the user must acknowledge explicitly.
//!
//! Private transactions expose only their public side after proving; its
//! effects decode to [`PublicEffect`]s, which must equal what was approved.

use std::str::FromStr as _;

use lee::{
    AccountId, privacy_preserving_transaction::message::Message as PrivateMessage,
    public_transaction::Message as PublicMessage,
};
use lee_core::native_token::{self, NATIVE_TOKEN_PROGRAM_ID};
use serde::{Deserialize, Serialize};
use token_core::{TokenDescriptor, TokenKind};

use crate::tx::amount;

/// Which decoder reads a program's instructions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decoder {
    Native,
    Token,
    AssociatedToken,
    /// Only attached to a program whose live image is our testimonial build.
    Testimonial,
}

/// The `(program account, decoder)` list. Defaults to the 0.3 builtins; a zone
/// can add entries (e.g. a token program deployed at another address).
#[derive(Clone, Debug)]
pub struct Decoders {
    entries: Vec<(AccountId, Decoder)>,
    /// Zone id and this wallet's local rebuilds (`verified.json`), so an
    /// approval shows a program the user rebuilt as verified.
    pub(crate) zone: String,
    pub(crate) verified: Vec<crate::verify::Verified>,
    /// Programs the user named (shown as "named by you").
    pub(crate) named: Vec<crate::verify::UserProgram>,
}

impl Default for Decoders {
    fn default() -> Self {
        Self {
            entries: vec![
                (NATIVE_TOKEN_PROGRAM_ID, Decoder::Native),
                (token_core::token_account_id(), Decoder::Token),
                (
                    associated_token_account_core::ata_account_id(),
                    Decoder::AssociatedToken,
                ),
            ],
            zone: String::new(),
            verified: Vec::new(),
            named: Vec::new(),
        }
    }
}

impl Decoders {
    /// Consult `verified` (local rebuilds for `zone`) when checking programs.
    pub fn with_rebuilds(mut self, zone: &str, verified: Vec<crate::verify::Verified>) -> Self {
        zone.clone_into(&mut self.zone);
        self.verified = verified;
        self
    }

    /// Show the user's own names for programs no source names.
    pub fn with_named(mut self, named: Vec<crate::verify::UserProgram>) -> Self {
        self.named = named;
        self
    }

    pub fn with(mut self, program: AccountId, decoder: Decoder) -> Self {
        self.entries.retain(|(p, _)| *p != program);
        self.entries.push((program, decoder));
        self
    }

    pub fn get(&self, program: AccountId) -> Option<Decoder> {
        self.entries
            .iter()
            .find(|(p, _)| *p == program)
            .map(|(_, d)| *d)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Asset {
    Native,
    Token {
        definition: String,
        #[serde(skip_serializing_if = "is_fungible")]
        nft: Option<&'static str>,
    },
}

const fn is_fungible(nft: &Option<&'static str>) -> bool {
    nft.is_none()
}

impl Asset {
    pub(crate) fn token(d: &TokenDescriptor) -> Self {
        Self::Token {
            definition: d.definition_id.to_string(),
            nft: match d.kind {
                TokenKind::Fungible => None,
                TokenKind::NftMaster => Some("nft_master"),
                TokenKind::NftPrintedCopy => Some("nft_copy"),
            },
        }
    }
}

/// Value moving out of or into one account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Flow {
    pub account: String,
    pub asset: Asset,
    #[serde(with = "amount")]
    pub amount: u128,
}

/// What the approval sheet shows. Built only from the message bytes.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub title: String,
    pub program: String,
    /// Plain-language details, one per line.
    pub lines: Vec<String>,
    pub outflows: Vec<Flow>,
    pub inflows: Vec<Flow>,
    /// Uses or grants of a token authority. Shown prominently.
    pub authorities: Vec<String>,
    /// This wallet's accounts that sign (and so authorize whatever the
    /// program does with them). Shown on every sheet.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub signers: Vec<String>,
    /// No decoder for this program (or the data didn't decode): the user must
    /// acknowledge that the wallet can't say what it does.
    pub unknown: bool,
}

fn short(id: &AccountId) -> String {
    let s = id.to_string();
    if s.len() > 14 {
        format!("{}…{}", &s[..6], &s[s.len() - 4..])
    } else {
        s
    }
}

/// Decode a public transaction's message.
pub fn public(message: &PublicMessage, decoders: &Decoders) -> Summary {
    let program = message.program_account_id;
    let accounts: Vec<AccountId> = message
        .shard_selectors
        .iter()
        .map(|s| s.account_id)
        .collect();
    let shards: Vec<AccountId> = message
        .shard_selectors
        .iter()
        .map(|s| s.program_account_id)
        .collect();
    let data = &message.instruction_data;
    let token_program = token_core::token_account_id();
    // A decoder describes the bytes only when every row selects the shard it
    // assumes; anything else is shown as unknown.
    let decoded = match decoders.get(program) {
        Some(Decoder::Native) if shards.iter().all(|s| *s == NATIVE_TOKEN_PROGRAM_ID) => {
            native(data, &accounts)
        }
        Some(Decoder::Token) if shards.iter().all(|s| *s == program) => {
            token(data, &accounts, program)
        }
        Some(Decoder::AssociatedToken)
            if shards.first() == Some(&NATIVE_TOKEN_PROGRAM_ID)
                && shards.iter().skip(1).all(|s| *s == token_program) =>
        {
            ata(data, &accounts)
        }
        Some(Decoder::Testimonial) if shards.iter().all(|s| *s == program) => {
            testimonial(data, &accounts, program)
        }
        _ => None,
    };
    let mut summary = decoded.unwrap_or_else(|| Summary {
        title: "Call a program the wallet can't read".to_owned(),
        lines: vec![
            format!("{} account(s): {}", accounts.len(), {
                let v: Vec<String> = accounts.iter().map(short).collect();
                v.join(", ")
            }),
            format!("{} byte(s) of instruction data", data.len()),
        ],
        unknown: true,
        ..Summary::default()
    });
    summary.program = program.to_string();
    summary
}

/// A post decodes only if its accounts are exactly the ones the program
/// derives for it; otherwise the program would refuse it anyway, and the
/// sheet shows it as unknown.
fn testimonial(data: &[u8], accounts: &[AccountId], program: AccountId) -> Option<Summary> {
    let testimonial_core::Instruction::Post {
        submission,
        page,
        username,
        text,
        timestamp_ms,
    } = borsh::from_slice(data).ok()?;
    testimonial_core::check_post(&submission, username.as_deref(), &text).ok()?;
    let p = program.value();
    let (author, stats, record, previous) = match (page, accounts) {
        (0, [a, s, r]) => (a, s, r, None),
        (1.., [a, s, r, prev]) => (a, s, r, Some(prev)),
        _ => return None,
    };
    if *stats.value() != testimonial_core::stats_account(p, &submission, page)
        || *record.value() != testimonial_core::record_account(p, &submission, author.value())
        || previous.is_some_and(|prev| {
            *prev.value() != testimonial_core::stats_account(p, &submission, page - 1)
        })
    {
        return None;
    }
    let mut lines = vec![format!("Submission: {submission}")];
    lines.push(match &username {
        Some(u) => format!("Name: {u}"),
        None => "Name: none".to_owned(),
    });
    // One sheet line per fact: a newline in the text must not fake another.
    lines.push(format!(
        "Text: \u{201c}{}\u{201d}",
        text.replace('\n', " \u{23ce} ")
    ));
    lines.push(format!("Time: {}", crate::testimonial::iso(timestamp_ms)));
    lines.push(format!(
        "Posted publicly and permanently from {}",
        short(author)
    ));
    Some(Summary {
        title: "Post a testimonial".to_owned(),
        lines,
        ..Summary::default()
    })
}

fn native(data: &[u8], accounts: &[AccountId]) -> Option<Summary> {
    let native_token::Instruction::Transfer { amount } = borsh::from_slice(data).ok()?;
    let [from, to] = accounts else { return None };
    Some(Summary {
        title: "Send".to_owned(),
        lines: vec![format!("{amount} to {}", short(to))],
        outflows: vec![Flow {
            account: from.to_string(),
            asset: Asset::Native,
            amount,
        }],
        inflows: vec![Flow {
            account: to.to_string(),
            asset: Asset::Native,
            amount,
        }],
        ..Summary::default()
    })
}

fn token(data: &[u8], accounts: &[AccountId], program: AccountId) -> Option<Summary> {
    use token_core::{Instruction as I, NewTokenDefinition};
    let instruction: I = borsh::from_slice(data).ok()?;
    let mint_authority = |definition: &AccountId| {
        format!(
            "{} becomes a token definition: whoever holds its key can mint more",
            short(definition)
        )
    };
    let s = match (instruction, accounts) {
        (
            I::Transfer {
                amount_to_transfer: amount,
                descriptor,
            },
            [from, to],
        ) => Summary {
            title: "Send tokens".to_owned(),
            lines: vec![format!(
                "{amount} of token {} to {}",
                short(&descriptor.definition_id),
                short(to)
            )],
            outflows: vec![Flow {
                account: from.to_string(),
                asset: Asset::token(&descriptor),
                amount,
            }],
            inflows: vec![Flow {
                account: to.to_string(),
                asset: Asset::token(&descriptor),
                amount,
            }],
            ..Summary::default()
        },
        (I::NewFungibleDefinition { name, total_supply }, [definition, holding]) => Summary {
            title: format!("Create token “{name}”"),
            lines: vec![format!(
                "Total supply {total_supply}, all to {}",
                short(holding)
            )],
            // The whole supply lands in the holder's token slot.
            inflows: vec![Flow {
                account: holding.to_string(),
                asset: Asset::Token {
                    definition: definition.to_string(),
                    nft: None,
                },
                amount: total_supply,
            }],
            authorities: vec![mint_authority(definition)],
            ..Summary::default()
        },
        (
            I::NewDefinitionWithMetadata {
                new_definition,
                metadata,
            },
            [definition, holding, _metadata],
        ) => {
            let (name, supply, kind) = match new_definition {
                NewTokenDefinition::Fungible { name, total_supply } => {
                    (name, total_supply, "token")
                }
                NewTokenDefinition::NonFungible {
                    name,
                    printable_supply,
                } => (name, printable_supply, "NFT"),
            };
            Summary {
                title: format!("Create {kind} “{name}”"),
                lines: vec![
                    format!("Supply {supply}, all to {}", short(holding)),
                    format!("Metadata at {}", metadata.uri),
                ],
                authorities: vec![mint_authority(definition)],
                ..Summary::default()
            }
        }
        (I::InitializeAccount { .. }, [definition, holding]) => Summary {
            title: "Open a token holding".to_owned(),
            lines: vec![format!(
                "{} will hold token {}",
                short(holding),
                short(definition)
            )],
            ..Summary::default()
        },
        (
            I::Burn {
                amount_to_burn,
                kind,
            },
            [definition, holding],
        ) => Summary {
            title: "Burn tokens".to_owned(),
            lines: vec![format!(
                "Destroy {amount_to_burn} of token {}",
                short(definition)
            )],
            outflows: vec![Flow {
                account: holding.to_string(),
                asset: Asset::token(&TokenDescriptor {
                    definition_id: *definition,
                    kind,
                }),
                amount: amount_to_burn,
            }],
            ..Summary::default()
        },
        (I::Mint { amount_to_mint }, [definition, holding]) => Summary {
            title: "Mint tokens".to_owned(),
            lines: vec![format!(
                "Create {amount_to_mint} new token {} for {}",
                short(definition),
                short(holding)
            )],
            inflows: vec![Flow {
                account: holding.to_string(),
                asset: Asset::Token {
                    definition: definition.to_string(),
                    nft: None,
                },
                amount: amount_to_mint,
            }],
            authorities: vec![format!(
                "Uses the mint authority of token {}",
                short(definition)
            )],
            ..Summary::default()
        },
        (I::PrintNft { definition_id }, [master, copy]) => Summary {
            title: "Print an NFT copy".to_owned(),
            lines: vec![format!(
                "From master {} into {}",
                short(master),
                short(copy)
            )],
            authorities: vec![format!(
                "Uses the print authority of NFT {}",
                short(&definition_id)
            )],
            ..Summary::default()
        },
        _ => return None,
    };
    let mut s = s;
    if program != token_core::token_account_id() {
        s.lines.push(format!(
            "Runs on a token program at {}, not the builtin one",
            short(&program)
        ));
    }
    Some(s)
}

fn ata(data: &[u8], accounts: &[AccountId]) -> Option<Summary> {
    use associated_token_account_core::Instruction as I;
    let instruction: I = borsh::from_slice(data).ok()?;
    // The ATA program hands its PDA authority to whatever token program the
    // instruction names: only the builtin one's effects are known.
    let (I::Create {
        token_program_id, ..
    }
    | I::Transfer {
        token_program_id, ..
    }
    | I::Burn {
        token_program_id, ..
    }) = &instruction;
    if *token_program_id != token_core::token_account_id() {
        return None;
    }
    let s = match (instruction, accounts) {
        (I::Create { .. }, [owner, definition, ata]) => Summary {
            title: "Open a token account".to_owned(),
            lines: [Some(format!(
                "{} for token {} (owned by {})",
                short(ata),
                short(definition),
                short(owner)
            ))]
            .into_iter()
            .flatten()
            .collect(),
            ..Summary::default()
        },
        (
            I::Transfer {
                token_program_id: _,
                descriptor,
                amount,
            },
            [_owner, from_ata, to],
        ) => Summary {
            title: "Send tokens".to_owned(),
            lines: [Some(format!(
                "{amount} of token {} to {}",
                short(&descriptor.definition_id),
                short(to)
            ))]
            .into_iter()
            .flatten()
            .collect(),
            outflows: vec![Flow {
                account: from_ata.to_string(),
                asset: Asset::token(&descriptor),
                amount,
            }],
            inflows: vec![Flow {
                account: to.to_string(),
                asset: Asset::token(&descriptor),
                amount,
            }],
            ..Summary::default()
        },
        (
            I::Burn {
                token_program_id: _,
                kind,
                amount,
            },
            [_owner, from_ata, definition],
        ) => Summary {
            title: "Burn tokens".to_owned(),
            lines: [Some(format!(
                "Destroy {amount} of token {}",
                short(definition)
            ))]
            .into_iter()
            .flatten()
            .collect(),
            outflows: vec![Flow {
                account: from_ata.to_string(),
                asset: Asset::token(&TokenDescriptor {
                    definition_id: *definition,
                    kind,
                }),
                amount,
            }],
            ..Summary::default()
        },
        _ => return None,
    };
    Some(s)
}

/// One public-state change a private transaction makes, decoded from its
/// proved message. Anything we have no decoder for stays raw (`Other`), so
/// an unexpected effect can never compare equal to an expected one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PublicEffect {
    Debit {
        account: String,
        asset: Asset,
        #[serde(with = "amount")]
        amount: u128,
    },
    Credit {
        account: String,
        asset: Asset,
        #[serde(with = "amount")]
        amount: u128,
    },
    Other {
        account: String,
        program: String,
        shard: String,
        data: String,
    },
}

/// The public effects of a proved private transaction, in message order.
pub fn private_effects(message: &PrivateMessage) -> Vec<PublicEffect> {
    let token_program = token_core::token_account_id();
    let mut out = Vec::new();
    for action in &message.public_actions {
        let account = action.account_id.to_string();
        for effect in &action.effects {
            let decoded = if effect.program_account_id == NATIVE_TOKEN_PROGRAM_ID
                && effect.shard_program_account_id == NATIVE_TOKEN_PROGRAM_ID
            {
                match borsh::from_slice::<native_token::Effect>(&effect.data) {
                    Ok(native_token::Effect::Debit(amount)) => Some(PublicEffect::Debit {
                        account: account.clone(),
                        asset: Asset::Native,
                        amount,
                    }),
                    Ok(native_token::Effect::Credit(amount)) => Some(PublicEffect::Credit {
                        account: account.clone(),
                        asset: Asset::Native,
                        amount,
                    }),
                    Err(_) => None,
                }
            } else if effect.program_account_id == token_program
                && effect.shard_program_account_id == token_program
            {
                match borsh::from_slice::<token_program::Effect>(&effect.data) {
                    Ok(token_program::Effect::Withdraw { descriptor, amount }) => {
                        Some(PublicEffect::Debit {
                            account: account.clone(),
                            asset: Asset::token(&descriptor),
                            amount,
                        })
                    }
                    Ok(token_program::Effect::Deposit { descriptor, amount }) => {
                        Some(PublicEffect::Credit {
                            account: account.clone(),
                            asset: Asset::token(&descriptor),
                            amount,
                        })
                    }
                    _ => None,
                }
            } else {
                None
            };
            out.push(decoded.unwrap_or_else(|| PublicEffect::Other {
                account: account.clone(),
                program: effect.program_account_id.to_string(),
                shard: effect.shard_program_account_id.to_string(),
                data: hex::encode(&effect.data),
            }));
        }
    }
    out
}

/// Long base58 ids in a message, shortened the way the UI shows them
/// ("7Hk2Qe…9Qpa"), so error copy stays readable.
pub fn shorten_ids(message: &str) -> String {
    const B58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut out = String::with_capacity(message.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() >= 32 {
            out.push_str(&run[..6]);
            out.push('…');
            out.push_str(&run[run.len() - 4..]);
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in message.chars() {
        if B58.contains(c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Parse an account id the way every caller-facing API does.
pub fn account_id(s: &str) -> anyhow::Result<AccountId> {
    AccountId::from_str(s).map_err(|e| anyhow::anyhow!("not a LEZ account id: {s} ({e})"))
}

#[cfg(test)]
mod tests {
    use lee::{AccountId, ProgramShardSelector, program::Program};

    use super::*;

    fn msg(program: AccountId, accounts: &[AccountId], data: Vec<u8>) -> PublicMessage {
        PublicMessage::new_preserialized(
            program,
            accounts
                .iter()
                .map(|a| ProgramShardSelector::new(*a, program))
                .collect(),
            vec![],
            data,
            None,
        )
    }

    #[test]
    fn native_and_token_transfers_decode() {
        let (a, b) = (AccountId::new([1; 32]), AccountId::new([2; 32]));
        let d = Decoders::default();
        let data =
            Program::serialize_instruction(native_token::Instruction::Transfer { amount: 7 })
                .unwrap();
        let s = public(&msg(NATIVE_TOKEN_PROGRAM_ID, &[a, b], data), &d);
        assert!(!s.unknown);
        assert_eq!(s.outflows[0].amount, 7);
        assert_eq!(s.outflows[0].account, a.to_string());

        let def = AccountId::new([9; 32]);
        let data = Program::serialize_instruction(token_core::Instruction::Transfer {
            amount_to_transfer: 5,
            descriptor: TokenDescriptor {
                definition_id: def,
                kind: TokenKind::Fungible,
            },
        })
        .unwrap();
        let s = public(&msg(token_core::token_account_id(), &[a, b], data), &d);
        assert_eq!(
            s.outflows[0].asset,
            Asset::Token {
                definition: def.to_string(),
                nft: None
            }
        );
    }

    #[test]
    fn a_foreign_shard_or_token_program_makes_it_unknown() {
        let (a, b) = (AccountId::new([1; 32]), AccountId::new([2; 32]));
        let d = Decoders::default();
        // A native transfer whose recipient row selects another program's shard.
        let data =
            Program::serialize_instruction(native_token::Instruction::Transfer { amount: 7 })
                .unwrap();
        let mut m = msg(NATIVE_TOKEN_PROGRAM_ID, &[a, b], data);
        m.shard_selectors[1].program_account_id = AccountId::new([9; 32]);
        assert!(public(&m, &d).unknown);

        // An ATA transfer that hands the PDA authority to a non-builtin token program.
        let ata = associated_token_account_core::ata_account_id();
        let data =
            Program::serialize_instruction(associated_token_account_core::Instruction::Transfer {
                token_program_id: AccountId::new([7; 32]),
                descriptor: TokenDescriptor {
                    definition_id: AccountId::new([3; 32]),
                    kind: TokenKind::Fungible,
                },
                amount: 1,
            })
            .unwrap();
        let token = token_core::token_account_id();
        let m = PublicMessage::new_preserialized(
            ata,
            vec![
                ProgramShardSelector::native_balance(a),
                ProgramShardSelector::new(b, token),
                ProgramShardSelector::new(AccountId::new([4; 32]), token),
            ],
            vec![],
            data,
            None,
        );
        assert!(public(&m, &d).unknown);
    }

    #[test]
    fn authority_use_is_flagged_and_unknown_is_unknown() {
        let (def, holder) = (AccountId::new([3; 32]), AccountId::new([4; 32]));
        let d = Decoders::default();
        let data =
            Program::serialize_instruction(token_core::Instruction::Mint { amount_to_mint: 1 })
                .unwrap();
        let s = public(
            &msg(token_core::token_account_id(), &[def, holder], data),
            &d,
        );
        assert_eq!(s.authorities.len(), 1);

        let s = public(&msg(AccountId::new([8; 32]), &[def], vec![1, 2, 3]), &d);
        assert!(s.unknown);
        // Trailing bytes after a valid instruction don't decode either.
        let mut data =
            Program::serialize_instruction(native_token::Instruction::Transfer { amount: 1 })
                .unwrap();
        data.push(0);
        assert!(public(&msg(NATIVE_TOKEN_PROGRAM_ID, &[def, holder], data), &d).unknown);
    }
}
