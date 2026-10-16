//! Who signed what, and when: a scan of the chain's blocks for the adoption
//! evidence. LP-0021 counts testimonials from accounts with *prior* activity,
//! so the evidence needs each author's own transactions strictly before its
//! post, not a nonce read today (which also counts later activity).
//!
//! The scan reads `getBlockRange` in chunks from block 1 to the tip and keeps,
//! per signer, `(block, tx hash, program)`. With a cache file it resumes from
//! the last block it saw, so a daily export only reads the new blocks.

use std::{collections::HashMap, path::Path};

use anyhow::Result;
use common::transaction::LeeTransaction;
use lee::AccountId;
use serde::{Deserialize, Serialize};
use wallet::WalletCore;

/// Blocks per `getBlockRange` call (the wallet's own poller uses 100).
const CHUNK: u64 = 100;
/// Refuse to scan more than this many blocks in one run.
const MAX_BLOCKS: u64 = 5_000_000;

/// One transaction an account signed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedTx {
    pub block: u64,
    pub hash: String,
    /// The public program called; `None` for a privacy-preserving tx.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    /// Blocks `1..=scanned` are in `by_signer`.
    pub scanned: u64,
    pub by_signer: HashMap<String, Vec<SignedTx>>,
}

impl History {
    /// Load `cache` (if any), then read the blocks after it up to `tip`.
    pub async fn scan(core: &WalletCore, tip: u64, cache: Option<&Path>) -> Result<Self> {
        let mut history: Self = match cache.map(std::fs::read) {
            Some(Ok(bytes)) => serde_json::from_slice(&bytes).unwrap_or_default(),
            _ => Self::default(),
        };
        // A cache from a longer chain (a reset) is useless: start over.
        if history.scanned > tip {
            history = Self::default();
        }
        anyhow::ensure!(
            tip - history.scanned <= MAX_BLOCKS,
            "refusing to scan {} blocks in one run",
            tip - history.scanned
        );
        let client = core.client_owned();
        let mut start = history.scanned + 1;
        while start <= tip {
            let end = (start + CHUNK - 1).min(tip);
            let blocks = chain_index::get_blocks(&client, start, end).await?;
            for block in blocks {
                let id = block.header.block_id;
                for tx in &block.body.transactions {
                    let hash = tx.hash().to_string();
                    let (signers, program) = match tx {
                        LeeTransaction::Public(t) => (
                            t.witness_set()
                                .signatures_and_public_keys()
                                .iter()
                                .map(|(_, pk)| AccountId::from(pk))
                                .collect::<Vec<_>>(),
                            Some(t.message().program_account_id.to_string()),
                        ),
                        LeeTransaction::PrivacyPreserving(t) => (
                            t.witness_set()
                                .signatures_and_public_keys()
                                .iter()
                                .map(|(_, pk)| AccountId::from(pk))
                                .collect::<Vec<_>>(),
                            None,
                        ),
                    };
                    for s in signers {
                        history
                            .by_signer
                            .entry(s.to_string())
                            .or_default()
                            .push(SignedTx {
                                block: id,
                                hash: hash.clone(),
                                program: program.clone(),
                            });
                    }
                }
            }
            history.scanned = end;
            start = end + 1;
        }
        if let Some(path) = cache {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, serde_json::to_vec(&history)?)?;
            std::fs::rename(&tmp, path)?;
        }
        Ok(history)
    }

    /// `author`'s first transaction calling `program` (its post).
    pub fn first_call(&self, author: &str, program: &str) -> Option<&SignedTx> {
        self.by_signer
            .get(author)?
            .iter()
            .find(|t| t.program.as_deref() == Some(program))
    }

    /// What `author` signed strictly before `block`, other than calls to
    /// `exclude` (the testimonial program itself).
    pub fn before(&self, author: &str, block: u64, exclude: &str) -> Vec<SignedTx> {
        self.by_signer
            .get(author)
            .map(|txs| {
                txs.iter()
                    .filter(|t| t.block < block && t.program.as_deref() != Some(exclude))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}
