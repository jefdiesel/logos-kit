//! `cargo xtask vectors`: byte-exact reference vectors produced by LEZ's own
//! code at the pinned rev, for the TypeScript codec/keys packages to match.
//!
//! Every value here comes from an upstream LEZ function (`Message::hash`,
//! borsh, `KeyTree`, `WitnessSet`, `LeeTransaction::hash`). The one thing we
//! do ourselves is BIP-340 signing with fixed aux randomness, because LEZ's
//! `Signature::new_with_aux_random` is crate-private. We call the same k256
//! function it calls and then verify through LEZ's public `is_valid_for`.
//!
//! Output (checked in, never hand-edited):
//! - `protocol/vectors/public_tx.json`
//! - `protocol/vectors/keys.json`
//! - `protocol/vectors/programs.json` (builtin ids, instruction data, our
//!   testimonial program's accounts, from the programs' own crates)

use std::{fs, path::Path, str::FromStr as _};

use anyhow::{Context as _, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use common::transaction::LeeTransaction;
use key_protocol::key_management::{
    key_tree::{
        KeyTreePrivate, KeyTreePublic, chain_index::ChainIndex, keys_public::ChildKeysPublic,
    },
    secret_holders::SeedHolder,
};
use lee::{
    AccountId, FeeDeclaration, PublicKey, PublicTransaction, Signature,
    public_transaction::{Message, WitnessSet},
};
use lee_core::{
    account::{Nonce, ProgramShardSelector},
    native_token::{self, NATIVE_TOKEN_PROGRAM_ID},
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

/// LEZ's public-message hash prefix. LEZ keeps it private, so we restate it
/// here and prove it against `Message::hash` for every vector we emit.
const PUBLIC_PREFIX: &[u8; 32] = b"/LEE/v0.3/Message/Public/\x00\x00\x00\x00\x00\x00\x00";

/// BIP-39 reference mnemonic (Trezor vectors). Test-only; never fund it.
const MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
/// Fixed aux randomness so the signed vector is deterministic.
const AUX_RAND: [u8; 32] = [0x11; 32];

pub fn run(repo_root: &Path) -> Result<()> {
    let root = repo_root.canonicalize()?;
    check_lez_rev(&root)?;
    let out = root.join("protocol/vectors");
    fs::create_dir_all(&out)?;
    write_json(&out.join("public_tx.json"), &public_tx_vectors()?)?;
    write_json(&out.join("keys.json"), &key_vectors()?)?;
    write_json(&out.join("programs.json"), &program_vectors()?)?;
    println!("wrote {}/{{public_tx,keys,programs}}.json", out.display());
    Ok(())
}

/// `LEZ_REV` must be the base of the tree Cargo builds (vendor/lez), or
/// `meta.lezRev` lies.
fn check_lez_rev(root: &Path) -> Result<()> {
    let tree = root.join("vendor/lez");
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(&tree)
        .args(["merge-base", "--is-ancestor", crate::LEZ_REV, "HEAD"])
        .status()
        .context("git merge-base in vendor/lez (run `cargo xtask lez-vendor`)")?;
    ensure!(
        ok.success(),
        "vendor/lez is not based on LEZ_REV {}",
        crate::LEZ_REV
    );
    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut s = serde_json::to_string_pretty(value)?;
    s.push('\n');
    fs::write(path, s).with_context(|| format!("write {}", path.display()))
}

fn meta() -> Value {
    json!({
        "generator": "cargo xtask vectors",
        "lezRev": crate::LEZ_REV,
        "note": "Generated from LEZ code at lezRev. Do not edit by hand.",
    })
}

fn account_json(id: &AccountId) -> Value {
    json!({ "base58": id.to_string(), "hex": hex::encode(id.value()) })
}

fn message_json(msg: &Message) -> Result<Value> {
    let borsh = borsh::to_vec(msg)?;
    let recomputed: [u8; 32] = Sha256::new()
        .chain_update(PUBLIC_PREFIX)
        .chain_update(&borsh)
        .finalize()
        .into();
    ensure!(
        recomputed == msg.hash(),
        "PUBLIC_PREFIX no longer matches LEZ's Message::hash"
    );
    Ok(json!({
        "fields": {
            "programAccountId": account_json(&msg.program_account_id),
            "shardSelectors": msg.shard_selectors.iter().map(|s| json!({
                "accountId": account_json(&s.account_id),
                "programAccountId": account_json(&s.program_account_id),
            })).collect::<Vec<_>>(),
            "nonces": msg.nonces.iter().map(|n| n.0.to_string()).collect::<Vec<_>>(),
            "instructionData": hex::encode(&msg.instruction_data),
            "fee": msg.fee.map(|f| json!({
                "payer": account_json(&f.payer),
                "gasLimit": f.gas_limit.to_string(),
                "tip": f.tip.to_string(),
                "maxFee": f.max_fee.to_string(),
            })),
        },
        "borsh": hex::encode(&borsh),
        "hash": hex::encode(msg.hash()),
    }))
}

/// The four wire layouts LEZ pins in `public_transaction/message.rs` tests,
/// plus one signed native transfer shaped like the official wallet's.
fn public_tx_vectors() -> Result<Value> {
    let program = AccountId::new([
        1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0,
        0, 0,
    ]);
    let named = ProgramShardSelector::new(AccountId::new([42; 32]), AccountId::new([43; 32]));
    let pinned = |sel: ProgramShardSelector, data: Vec<u8>, fee: Option<FeeDeclaration>| {
        Message::new_preserialized(program, vec![sel], vec![Nonce(5)], data, fee)
    };
    let cases = [
        ("exempt", pinned(named, vec![], None)),
        (
            "balance_shard_selector",
            pinned(
                ProgramShardSelector::native_balance(AccountId::new([42; 32])),
                vec![],
                None,
            ),
        ),
        ("nonempty_instruction", pinned(named, vec![7, 8, 9], None)),
        (
            "charged",
            pinned(
                named,
                vec![],
                Some(FeeDeclaration::new(AccountId::new([7; 32]), 9, 3, 100)),
            ),
        ),
    ];
    let mut messages = Vec::new();
    for (name, msg) in &cases {
        let mut v = message_json(msg)?;
        v["name"] = json!(name);
        messages.push(v);
    }

    Ok(json!({
        "meta": meta(),
        "prefix": hex::encode(PUBLIC_PREFIX),
        "messages": messages,
        "signed": [signed_native_transfer()?],
    }))
}

fn signed_native_transfer() -> Result<Value> {
    let seed = seed_holder()?;
    let mut tree = KeyTreePublic::new(&seed);
    let (from_id, from_path) = tree.generate_new_public_node_layered().context("node")?;
    let (to_id, _) = tree.generate_new_public_node_layered().context("node")?;
    let from = tree.get_node(from_id).context("from node")?;

    let amount: u128 = 1_000;
    let instruction = native_token::Instruction::Transfer { amount };
    let instruction_data = borsh::to_vec(&instruction)?;
    let (gas_limit, tip, max_fee) = (50_000_u64, 0_u64, 1_000_000_u128);
    let msg = Message::new_preserialized(
        NATIVE_TOKEN_PROGRAM_ID,
        vec![
            ProgramShardSelector::native_balance(from_id),
            ProgramShardSelector::native_balance(to_id),
        ],
        vec![Nonce(0)],
        instruction_data,
        Some(FeeDeclaration::new(from_id, gas_limit, tip, max_fee)),
    );
    let msg_hash = msg.hash();

    // Same call as LEZ's crate-private `Signature::new_with_aux_random`.
    let signing_key = k256::schnorr::SigningKey::from_bytes(from.ssk.value())?;
    let sig_bytes = signing_key
        .sign_prehash_with_aux_rand(&msg_hash, &AUX_RAND)?
        .to_bytes();
    let signature = Signature::from_str(&hex::encode(sig_bytes))?;
    let public_key = PublicKey::new_from_private_key(&from.ssk);
    ensure!(
        signature.is_valid_for(&msg_hash, &public_key),
        "LEZ rejects our signature"
    );
    ensure!(
        AccountId::from(&public_key) == from_id,
        "account id mismatch"
    );

    let witness = WitnessSet::from_raw_parts(vec![(signature.clone(), public_key.clone())]);
    ensure!(witness.is_valid_for(&msg), "witness set invalid");
    let tx = PublicTransaction::new(msg.clone(), witness);
    let wire = LeeTransaction::Public(tx.clone());
    let wire_borsh = borsh::to_vec(&wire)?;

    let mut v = message_json(&msg)?;
    v["name"] = json!("native_transfer");
    v["mnemonic"] = json!(MNEMONIC);
    v["signerPath"] = json!(from_path.to_string());
    v["instruction"] = json!({ "Transfer": { "amount": amount.to_string() } });
    v["signingKey"] = json!(hex::encode(from.ssk.value()));
    v["auxRand"] = json!(hex::encode(AUX_RAND));
    v["publicKey"] = json!(hex::encode(public_key.value()));
    v["signature"] = json!(signature.to_string());
    v["transaction"] = json!({
        "borsh": hex::encode(borsh::to_vec(&tx)?),
        "hash": hex::encode(tx.hash()),
    });
    // `sendTransaction` takes base64(borsh(LeeTransaction)); the RPC returns this hash.
    v["rpc"] = json!({
        "sendTransactionParam": B64.encode(&wire_borsh),
        "txHash": wire.hash().to_string(),
    });
    Ok(v)
}

fn seed_holder() -> Result<SeedHolder> {
    let mnemonic = bip39::Mnemonic::parse(MNEMONIC)?;
    // The official wallet uses an empty passphrase (lez/wallet/src/storage.rs).
    Ok(SeedHolder::from_mnemonic(&mnemonic, ""))
}

/// `SeedHolder.seed` is private; this is the same `to_seed` call its
/// `from_mnemonic` makes. `key_vectors` checks both give the same root.
fn seed_bytes() -> Result<[u8; 64]> {
    Ok(bip39::Mnemonic::parse(MNEMONIC)?.to_seed(""))
}

fn public_node_json(path: &str, node: &ChildKeysPublic) -> Value {
    json!({
        "path": path,
        "sk": hex::encode(node.sk.value()),
        "ssk": hex::encode(node.ssk.value()),
        "pk": hex::encode(node.pk.value()),
        "cc": hex::encode(node.cc),
        "accountId": account_json(&node.account_id()),
    })
}

fn key_vectors() -> Result<Value> {
    let seed = seed_holder()?;
    let seed_bytes = seed_bytes()?;

    let root = ChildKeysPublic::root(seed_bytes);
    let mut public = vec![public_node_json("/", &root)];
    let mut tree = KeyTreePublic::new(&seed);
    let tree_root = tree.key_map.get(&ChainIndex::root()).context("tree root")?;
    ensure!(
        tree_root.pk == root.pk,
        "seed derivation differs from SeedHolder"
    );
    for _ in 0..3 {
        let (id, path) = tree
            .generate_new_public_node_layered()
            .context("public node")?;
        public.push(public_node_json(
            &path.to_string(),
            tree.get_node(id).context("node")?,
        ));
    }

    // Private keys include ML-KEM material; emit LEZ's own serde form so the
    // shape tracks upstream instead of our reading of its internals.
    let mut private_tree = KeyTreePrivate::new(&seed);
    let mut private = Vec::new();
    for _ in 0..2 {
        let path = private_tree
            .create_private_accounts_key_node_layered()
            .context("private node")?;
        let node = private_tree.key_map.get(&path).context("private node")?;
        let chain = &node.value.0;
        let account_ids: Vec<Value> =
            key_protocol::key_management::key_tree::traits::KeyTreeNode::account_ids(node)
                .map(|id| account_json(&id))
                .collect();
        private.push(json!({
            "path": path.to_string(),
            "keyChain": serde_json::to_value(chain)?,
            "ccc": hex::encode(node.ccc),
            "accountIds": account_ids,
        }));
    }

    Ok(json!({
        "meta": meta(),
        "mnemonic": MNEMONIC,
        "passphrase": "",
        "seed": hex::encode(seed_bytes),
        "public": public,
        "private": private,
    }))
}

fn program_vectors() -> Result<Value> {
    use testimonial_core as tm;
    let definition = AccountId::new([7; 32]);
    let native = borsh::to_vec(&native_token::Instruction::Transfer {
        amount: 12_345_678_901_234_567_890_123,
    })?;
    let token = borsh::to_vec(&token_core::Instruction::Transfer {
        amount_to_transfer: u128::MAX,
        descriptor: token_core::TokenDescriptor {
            definition_id: definition,
            kind: token_core::TokenKind::Fungible,
        },
    })?;
    let program = AccountId::new([9; 32]);
    let author = AccountId::new([1; 32]);
    let sub = "LP-0021/logos-kit";
    let mut testimonials = Vec::new();
    for (page, username, text) in [
        (0_u32, Some("abu"), "I use Logos Kit \u{2713}\nsecond line"),
        (3, None, "no name"),
    ] {
        let data = borsh::to_vec(&tm::Instruction::Post {
            submission: sub.to_owned(),
            page,
            username: username.map(str::to_owned),
            text: text.to_owned(),
            timestamp_ms: 1_800_000_000_000,
        })?;
        testimonials.push(json!({
            "program": program.to_string(), "author": author.to_string(), "submission": sub,
            "page": page, "username": username, "text": text, "timestampMs": 1_800_000_000_000_u64,
            "stats": AccountId::new(tm::stats_account(program.value(), sub, page)).to_string(),
            "previousStats": page.checked_sub(1).map(|p| AccountId::new(tm::stats_account(program.value(), sub, p)).to_string()),
            "record": AccountId::new(tm::record_account(program.value(), sub, author.value())).to_string(),
            "data": hex::encode(data),
        }));
    }
    Ok(json!({
        "meta": meta(),
        "builtins": {
            "native": NATIVE_TOKEN_PROGRAM_ID.to_string(),
            "token": token_core::token_account_id().to_string(),
            "associatedTokenAccount": associated_token_account_core::ata_account_id().to_string(),
            "programLoader": lee_core::program::PROGRAM_LOADER_ACCOUNT_ID.to_string(),
        },
        "nativeTransfer": { "amount": "12345678901234567890123", "data": hex::encode(native) },
        "tokenTransfer": {
            "amount": u128::MAX.to_string(), "definition": definition.to_string(),
            "kind": "fungible", "data": hex::encode(token),
        },
        "testimonial": testimonials,
        "associatedTokenAccount": {
            "owner": author.to_string(),
            "definition": definition.to_string(),
            "ata": associated_token_account_core::get_associated_token_account_id(
                &associated_token_account_core::ata_account_id(),
                &associated_token_account_core::compute_ata_seed(author, definition, token_core::token_account_id()),
            ).to_string(),
        },
    }))
}
