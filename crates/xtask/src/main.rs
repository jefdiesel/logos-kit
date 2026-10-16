//! Repo automation. `cargo xtask <command>`.
//!
//! Commands:
//! - `fingerprint [url]`: identify the LEZ protocol version a sequencer runs.
//!   There is no version RPC; v0.3 exposes `getFeeState`, v0.2.x has `pinata`
//!   in `getProgramIds`.
//! - `vectors`: regenerate `protocol/vectors/*.json` from LEZ code (see vectors.rs).
//! - `lez-vendor` / `lez-export`: materialize `vendor/lez` (LEZ rev + our
//!   patches) / write its commits back to `vendor/lez-patches` (see lez.rs).
//! - `types`: regenerate `crates/lwsp-types/src/generated.rs` from the LWS-0 JSON Schema.

mod lez;
mod types;
mod vectors;

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// LEZ revision the vectors come from; must match the workspace `rev` pins.
pub const LEZ_REV: &str = "1a003bfbfb89fcf987f62975cd08a401169e1b8c";

const DEFAULT_SEQUENCER: &str = "https://testnet.lez.logos.co";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("fingerprint") => {
            let url = args.get(1).map_or(DEFAULT_SEQUENCER, String::as_str);
            fingerprint(url)
        }
        Some("lez-vendor") => lez::vendor(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")),
        Some("lez-export") => lez::export(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")),
        Some("types") => {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            types::run(&root)
        }
        Some("vectors") => {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            vectors::run(&root)
        }
        _ => {
            eprintln!(
                "usage: cargo xtask <fingerprint [sequencer-url] | vectors | types | lez-vendor | lez-export>"
            );
            std::process::exit(2);
        }
    }
}

/// JSON-RPC call. Returns `Ok(Err(error_object))` for protocol-level errors
/// (e.g. "Method not found") so callers can use them as signals.
fn rpc(
    agent: &ureq::Agent,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Result<Value, Value>> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let mut res = agent
        .post(url)
        .send_json(&body)
        .with_context(|| format!("POST {url} {method}"))?;
    let v: Value = res
        .body_mut()
        .read_json()
        .context("decode JSON-RPC response")?;
    if let Some(err) = v.get("error") {
        return Ok(Err(err.clone()));
    }
    match v.get("result") {
        Some(r) => Ok(Ok(r.clone())),
        None => bail!("malformed JSON-RPC response for {method}: {v}"),
    }
}

fn fingerprint(url: &str) -> Result<()> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent("logos-kit-xtask/0.1")
        .build();
    let agent: ureq::Agent = config.into();

    let fee_state = rpc(&agent, url, "getFeeState", json!([]))?;
    let program_ids = rpc(&agent, url, "getProgramIds", json!([]))?;
    let last_block = rpc(&agent, url, "getLastBlockId", json!([]))?;

    let program_names: Vec<String> = match &program_ids {
        Ok(Value::Object(map)) => map.keys().cloned().collect(),
        _ => vec![],
    };
    let has_pinata = program_names.iter().any(|n| n.contains("pinata"));

    // The only reliable test: does the newest block decode with the LEZ types
    // we're pinned to (v0.3.0-rc1)? 0.2.5 already had getFeeState and no
    // piñata, so those alone mislabel it (2026-09-27, the halted devnet).
    let head = match &last_block {
        Ok(v) => v
            .as_u64()
            .map(|id| rpc(&agent, url, "getBlock", json!([id])))
            .transpose()?,
        Err(_) => None,
    };
    let (decodes, head_ms) = match head {
        Some(Ok(Value::String(b64))) => {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
            match borsh::from_slice::<common::block::Block>(&bytes) {
                Ok(b) => (true, Some(b.header.timestamp)),
                // The header's id / prev hash / hash / timestamp prefix is the
                // same in 0.2.x, so the time still reads.
                Err(_) => (
                    false,
                    bytes
                        .get(72..80)
                        .and_then(|t| t.try_into().ok())
                        .map(u64::from_le_bytes),
                ),
            }
        }
        _ => (false, None),
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let head_age_secs = head_ms.map(|t| (now_ms.saturating_sub(u128::from(t)) / 1000) as u64);

    let version = match (decodes, &fee_state, has_pinata) {
        (true, Ok(_), _) => "0.3",
        (false, Ok(_), false) => "0.2.5 (not our 0.3 block format)",
        (_, Err(_), true) => "0.2.x",
        _ => "unknown",
    };

    let report = json!({
        "sequencer": url,
        "version": version,
        "lastBlockId": last_block.as_ref().ok(),
        "headDecodesAsPinned": decodes,
        "headAgeSecs": head_age_secs,
        "halted": head_age_secs.map(|s| s > 600),
        "programs": program_names,
        "feeState": fee_state.as_ref().ok(),
        "feeStateError": fee_state.as_ref().err(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
