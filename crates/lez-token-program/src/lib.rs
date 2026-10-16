//! `token_program::Effect`, copied verbatim from LEZ v0.3.0
//! (`lez/programs/token/src/lib.rs` at `db66590a`).
//!
//! LEZ v0.3.1 removed the token, ATA and AMM programs from its tree and from
//! the builtin genesis set (logos-execution-zone #926 "grand wallet CLI cut").
//! The engine still decodes the token program's effects, so the type lives
//! here. Only the effect enum is kept; the program logic is not.

use borsh::{BorshDeserialize, BorshSerialize};
use lee_core::account::{AccountId, ShardData};
pub use token_core as core;
use token_core::{TokenDescriptor, TokenKind};

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Effect {
    Withdraw {
        descriptor: TokenDescriptor,
        amount: u128,
    },
    Deposit {
        descriptor: TokenDescriptor,
        amount: u128,
    },
    Create(ShardData),
    CheckHoldingKind(TokenKind),
    InitializeHolding {
        descriptor: TokenDescriptor,
        is_authorized: bool,
    },
    MintSupply {
        amount: u128,
    },
    BurnSupply {
        kind: TokenKind,
        amount: u128,
    },
    BurnHolding {
        descriptor: TokenDescriptor,
        amount: u128,
    },
    PrintCopy {
        definition_id: AccountId,
    },
}
