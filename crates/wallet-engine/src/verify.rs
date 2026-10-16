//! Program source verification.
//!
//! In 0.3 a program lives at an account whose loader shard holds a
//! [`ProgramHeader`] `{image_id, program_first_segment, immutable}`. The
//! header is what runs, so every check reads it live:
//!
//! - **builtin**: the address is derived from a builtin name. Its image must
//!   equal the one compiled from our pinned LEZ source, which our evidence
//!   file (`registry/builtins.json`) records as reproduced in the pinned
//!   RISC Zero docker builder.
//! - **registry**: `registry/programs.json` claims `{repo, commit, guestPath,
//!   bin, dockerTag, imageId}` for a program account. `verify-program`
//!   rebuilds it and caches `verified_local` by `(zone, account, image)`.
//! - anything else is `unknown`; a live image that differs from the
//!   registry's (or builtin's) is `mismatch`.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, Result, bail, ensure};
use lee::{AccountId, ProgramShardSelector, program::Program};
use lee_core::program::{PROGRAM_LOADER_ACCOUNT_ID, ProgramHeader};
use serde::{Deserialize, Serialize};
use wallet::WalletCore;

/// Programs whose source Logos Kit ships a claim for.
const REGISTRY: &str = include_str!("../../../registry/programs.json");
/// Builtin images reproduced from LEZ source (see `logos-kit verify-program --builtins`).
const BUILTINS_EVIDENCE: &str = include_str!("../../../registry/builtins.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Rebuilt from source (here or by our CI for builtins) to this image.
    VerifiedLocal,
    /// A registry entry names this image; not rebuilt yet.
    Claimed,
    /// No source known.
    Unknown,
    /// The live image differs from what the source builds.
    Mismatch,
}

/// What the approval sheet shows about the program a transaction runs.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramCheck {
    pub account: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub image_id: String,
    #[serde(skip)]
    pub image_id_words: [u32; 8],
    pub immutable: bool,
    pub builtin: bool,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// Why the status is what it is, in plain words.
    pub note: String,
    /// `name` is one the user gave it (Settings → Programs you named).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub named_by_user: bool,
}

/// A program the user named themselves, so approvals can say "Payroll
/// (named by you)" instead of "Unknown program". A name is the user's own
/// label, never a source claim: it doesn't change the verification status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserProgram {
    pub zone: String,
    pub account: String,
    pub name: String,
    pub added_ms: u64,
}

/// Where a program's source lives and how it builds reproducibly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub repo: String,
    pub commit: String,
    /// Directory holding the guest methods' `Cargo.toml`.
    pub guest_path: String,
    /// Output file name under `riscv32im-risc0-zkvm-elf/docker/`.
    pub bin: String,
    pub docker_tag: String,
    /// Cargo features (`--no-default-features --features …`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryEntry {
    name: String,
    /// CAIP-2 chain the account lives on (a deploy is per zone).
    #[serde(default)]
    chain: Option<String>,
    account: String,
    image_id: String,
    source: Source,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinEvidence {
    pub name: String,
    pub image_id: String,
    pub source: Source,
    /// Date the rebuild matched (YYYY-MM-DD).
    pub reproduced: String,
}

/// A cached local rebuild.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verified {
    pub zone: String,
    pub account: String,
    pub image_id: String,
    pub source: Source,
}

pub fn image_hex(id: &[u32; 8]) -> String {
    let bytes: Vec<u8> = id.iter().flat_map(|w| w.to_le_bytes()).collect();
    hex::encode(bytes)
}

/// 0.3 builtins: name, address, image compiled from our pinned LEZ.
///
/// LEZ v0.3.1 dropped token, associated_token_account and amm from the
/// builtin set (and from its source tree), so they are no longer listed: an
/// account at their old builtin-derived address is just an unknown program.
pub fn builtins() -> Vec<(&'static str, AccountId, [u32; 8])> {
    vec![
        (
            "clock",
            programs::clock_account_id(),
            programs::clock().id(),
        ),
        ("fee", programs::fee_account_id(), programs::fee().id()),
        (
            "bridge",
            programs::bridge_account_id(),
            programs::bridge().id(),
        ),
        (
            "bridge_lock",
            programs::bridge_lock_account_id(),
            programs::bridge_lock().id(),
        ),
        (
            "wrapped_token",
            programs::wrapped_token_account_id(),
            programs::wrapped_token().id(),
        ),
        (
            "sequencer_stake",
            programs::sequencer_stake_account_id(),
            programs::sequencer_stake().id(),
        ),
        (
            "cross_zone_outbox",
            programs::cross_zone_outbox_account_id(),
            programs::cross_zone_outbox().id(),
        ),
        (
            "cross_zone_inbox",
            programs::cross_zone_inbox_account_id(),
            programs::cross_zone_inbox().id(),
        ),
        (
            "ping_sender",
            programs::ping_sender_account_id(),
            programs::ping_sender().id(),
        ),
        (
            "ping_receiver",
            programs::ping_receiver_account_id(),
            programs::ping_receiver().id(),
        ),
    ]
}

fn registry() -> Vec<RegistryEntry> {
    serde_json::from_str(REGISTRY).unwrap_or_default()
}

pub fn builtin_evidence() -> Vec<BuiltinEvidence> {
    serde_json::from_str(BUILTINS_EVIDENCE).unwrap_or_default()
}

/// The registry's source claim for `account`, if any.
pub fn registry_source(account: &str) -> Option<(String, Source)> {
    registry()
        .into_iter()
        .find(|e| e.account == account)
        .map(|e| (e.name, e.source))
}

/// The account the registry names for program `name` on `chain`.
pub fn registry_program(name: &str, chain: &str) -> Option<String> {
    registry()
        .into_iter()
        .find(|e| e.name == name && e.chain.as_deref() == Some(chain))
        .map(|e| e.account)
}

/// The live header at `program`, or `None` if nothing is deployed there.
pub async fn read_header(core: &WalletCore, program: AccountId) -> Result<Option<ProgramHeader>> {
    let account = core
        .get_account_view(ProgramShardSelector::new(
            program,
            PROGRAM_LOADER_ACCOUNT_ID,
        ))
        .await?;
    let shard = account.data.shard(PROGRAM_LOADER_ACCOUNT_ID);
    if shard.is_empty() {
        return Ok(None);
    }
    ProgramHeader::from_bytes(shard.as_ref())
        .map(Some)
        .context("the program header doesn't decode")
}

/// Status of `program` as it is on chain now (no local rebuild cache).
pub async fn check(core: &WalletCore, program: AccountId) -> Result<ProgramCheck> {
    check_cached(core, program, "", &[], &[]).await
}

impl crate::decode::Decoders {
    /// [`check`] for an approval: upgraded by the local rebuilds it carries.
    pub(crate) async fn check(
        &self,
        core: &WalletCore,
        program: AccountId,
    ) -> Result<ProgramCheck> {
        check_cached(core, program, &self.zone, &self.verified, &self.named).await
    }
}

/// [`check`], also consulting this wallet's cache of local rebuilds.
pub async fn check_cached(
    core: &WalletCore,
    program: AccountId,
    zone: &str,
    cache: &[Verified],
    named: &[UserProgram],
) -> Result<ProgramCheck> {
    let header = read_header(core, program)
        .await?
        .with_context(|| format!("no program is deployed at {program}"))?;
    let mut out = classify(program, &header, cache, zone);
    // The user's own name, only where no source names it.
    if out.name.is_none()
        && let Some(n) = named
            .iter()
            .find(|n| n.zone == zone && n.account == out.account)
    {
        out.name = Some(n.name.clone());
        out.named_by_user = true;
    }
    Ok(out)
}

fn classify(
    program: AccountId,
    header: &ProgramHeader,
    cache: &[Verified],
    zone: &str,
) -> ProgramCheck {
    let account = program.to_string();
    let image = image_hex(&header.image_id);
    let mut out = ProgramCheck {
        account: account.clone(),
        name: None,
        image_id: image.clone(),
        image_id_words: header.image_id,
        immutable: header.immutable,
        builtin: false,
        status: Status::Unknown,
        source: None,
        note: "no source is known for this program".to_owned(),
        named_by_user: false,
    };
    let cached = cache
        .iter()
        .find(|v| v.zone == zone && v.account == account && v.image_id == image);

    if let Some((name, _, compiled)) = builtins().into_iter().find(|(_, id, _)| *id == program) {
        out.builtin = true;
        out.name = Some(name.to_owned());
        let evidence = builtin_evidence()
            .into_iter()
            .find(|e| e.name == name && e.image_id == image);
        (out.status, out.note) = if header.image_id != compiled {
            (
                Status::Mismatch,
                format!(
                    "builtin {name} runs an image that differs from LEZ {} (the zone may run another release)",
                    &crate::LEZ_REV[..8]
                ),
            )
        } else if let Some(e) = evidence {
            out.source = Some(e.source);
            (
                Status::VerifiedLocal,
                format!(
                    "builtin {name}, rebuilt from LEZ source on {}",
                    e.reproduced
                ),
            )
        } else {
            (
                Status::Claimed,
                format!("builtin {name}, same image as the LEZ release we pin"),
            )
        };
        return out;
    }

    // Our own program is known by its image wherever it is deployed; the
    // build record says which commit and builder reproduce it.
    // A public transaction names the account, not the image, so a copy
    // someone can still upgrade is only as good as its owner: verified only
    // when immutable or the registry's own deployment.
    if let Some(build) = crate::testimonial::build().filter(|b| b.image_id == image) {
        out.name = Some("testimonial".to_owned());
        out.source = Some(build.source.clone());
        let registered = registry()
            .iter()
            .any(|e| e.name == "testimonial" && e.account == account);
        (out.status, out.note) = if header.immutable || registered {
            (
                Status::VerifiedLocal,
                format!(
                    "Logos Kit testimonial program, the image our docker build reproduced on {}",
                    build.built
                ),
            )
        } else {
            (
                Status::Claimed,
                "an upgradeable copy of the Logos Kit testimonial program: its owner can change it"
                    .to_owned(),
            )
        };
        if !out.immutable && registered {
            out.note.push_str("; its owner can still upgrade it");
        }
        return out;
    }

    if let Some(entry) = registry().into_iter().find(|e| e.account == account) {
        out.name = Some(entry.name.clone());
        out.source = Some(entry.source.clone());
        (out.status, out.note) = if entry.image_id != image {
            (
                Status::Mismatch,
                "the deployed image differs from the registry's source".to_owned(),
            )
        } else if cached.is_some() {
            (
                Status::VerifiedLocal,
                "rebuilt from source on this machine".to_owned(),
            )
        } else {
            (
                Status::Claimed,
                "the registry names its source; run verify-program to rebuild it".to_owned(),
            )
        };
        return out;
    }

    if let Some(v) = cached {
        out.source = Some(v.source.clone());
        out.status = Status::VerifiedLocal;
        out.note = "rebuilt from source on this machine".to_owned();
    }
    if !out.immutable {
        out.note.push_str("; its owner can still upgrade it");
    }
    out
}

/// Rebuild `source` reproducibly (git checkout of the exact commit, then
/// `cargo risczero build` in the pinned docker builder) and return its image
/// id. Blocking; takes minutes. Needs git, docker and cargo-risczero.
pub fn build_image_id(source: &Source, work: &Path) -> Result<[u32; 8]> {
    image_in(&build(source, work)?, &source.bin)
}

/// Check out and build `source`; returns the directory holding the `.bin`s.
pub fn build(source: &Source, work: &Path) -> Result<PathBuf> {
    ensure!(
        source.commit.len() == 40 && source.commit.chars().all(|c| c.is_ascii_hexdigit()),
        "the source commit must be a full 40-hex git hash"
    );
    let guest = Path::new(&source.guest_path);
    ensure!(
        !source.guest_path.is_empty()
            && guest
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "the guest path must be relative, inside the checkout"
    );
    ensure!(
        !source.bin.is_empty()
            && source
                .bin
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
            && !source.bin.starts_with('.'),
        "bin must be a plain file name"
    );
    ensure!(
        !source.docker_tag.is_empty()
            && source
                .docker_tag
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-'),
        "docker tag may only use letters, digits, '.', '_' and '-'"
    );
    ensure!(
        source.repo.starts_with("https://")
            || source.repo.starts_with("file://")
            || Path::new(&source.repo).is_absolute(),
        "repo must be an https:// or file:// URL, or an absolute local path"
    );
    std::fs::create_dir_all(work)?;
    // cargo-risczero strips the docker context (the working directory) off
    // the manifest path, so both must be spelled canonically (macOS: /var →
    // /private/var).
    let work = &work.canonicalize()?;
    let checkout = work.join("src");
    if !checkout.join(".git").exists() {
        std::fs::create_dir_all(&checkout)?;
        run(Command::new("git").arg("init").arg("-q").arg(&checkout))?;
    }
    run(Command::new("git")
        .arg("-C")
        .arg(&checkout)
        .args(["fetch", "-q", "--depth", "1", "--"])
        .arg(&source.repo)
        .arg(&source.commit))?;
    run(Command::new("git").arg("-C").arg(&checkout).args([
        "checkout",
        "-q",
        "--force",
        "FETCH_HEAD",
    ]))?;
    let head = output(
        Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["rev-parse", "HEAD"]),
    )?;
    ensure!(
        head.trim() == source.commit,
        "checked out {} instead of {}",
        head.trim(),
        source.commit
    );
    let target = work.join("target");
    let out = target.join("riscv32im-risc0-zkvm-elf/docker");
    // Stale outputs must never pass for this build's.
    let _ = std::fs::remove_dir_all(&out);
    let manifest = checkout.join(&source.guest_path).join("Cargo.toml");
    let mut build = Command::new("cargo");
    // The docker context is the working directory (LEZ's Justfile runs from
    // the repo root), so build from inside the checkout.
    build
        .current_dir(&checkout)
        .args(["risczero", "build", "--manifest-path"])
        .arg(&manifest)
        .env("RISC0_DOCKER_CONTAINER_TAG", &source.docker_tag)
        .env("CARGO_TARGET_DIR", &target)
        // cargo-risczero prints image ids on stdout; ours carries only JSON.
        .stdout(std::io::stderr());
    if let Some(f) = &source.features {
        build.args(["--no-default-features", "--features", f]);
    }
    run(&mut build)?;
    Ok(out)
}

/// The image id of `bin` in a build output directory.
pub fn image_in(dir: &Path, bin: &str) -> Result<[u32; 8]> {
    let path = dir.join(bin);
    let bytes = std::fs::read(&path).with_context(|| format!("build output {}", path.display()))?;
    let program = Program::new(Cow::Owned(bytes))
        .map_err(|e| anyhow::anyhow!("not a RISC Zero program binary: {e}"))?;
    Ok(program.id())
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("running {:?}", cmd.get_program()))?;
    if !status.success() {
        bail!("{:?} failed ({status})", cmd.get_program());
    }
    Ok(())
}

fn output(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().context("running git")?;
    ensure!(out.status.success(), "git rev-parse failed");
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The local rebuild cache (`verified.json`). Not secret; it only upgrades
/// `claimed`/`unknown` to `verified_local` for exact images. Read from the
/// wallet's data dir and from the shared one (`~/.logos-kit`, the CLI's
/// default home), so a program rebuilt with `logos-kit verify-program`
/// shows as verified in Basecamp too.
pub fn load_cache(dir: &Path) -> Vec<Verified> {
    let mut all = read_cache(dir);
    if let Some(shared) = shared_dir().filter(|s| s != dir) {
        for v in read_cache(&shared) {
            if !all
                .iter()
                .any(|a| a.zone == v.zone && a.account == v.account)
            {
                all.push(v);
            }
        }
    }
    all
}

fn read_cache(dir: &Path) -> Vec<Verified> {
    std::fs::read(dir.join("verified.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Where every Logos Kit wallet on this machine shares its rebuilds.
pub fn shared_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".logos-kit"))
}

/// Record a rebuild in `dir` and in the shared cache.
pub fn save_cache(dir: &Path, entry: Verified) -> Result<()> {
    write_cache(dir, &entry)?;
    if let Some(shared) = shared_dir().filter(|s| s != dir) {
        std::fs::create_dir_all(&shared)?;
        write_cache(&shared, &entry)?;
    }
    Ok(())
}

fn write_cache(dir: &Path, entry: &Verified) -> Result<()> {
    let mut all = read_cache(dir);
    all.retain(|v| !(v.zone == entry.zone && v.account == entry.account));
    all.push(entry.clone());
    let tmp = dir.join("verified.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&all)?)?;
    std::fs::rename(&tmp, dir.join("verified.json"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(image_id: [u32; 8], immutable: bool) -> ProgramHeader {
        ProgramHeader {
            image_id,
            program_first_segment: AccountId::new([1; 32]),
            immutable,
        }
    }

    #[test]
    fn builtin_matches_or_mismatches_its_compiled_image() {
        let clock = programs::clock_account_id();
        let ok = classify(clock, &header(programs::clock().id(), true), &[], "z");
        assert!(ok.builtin);
        assert_ne!(ok.status, Status::Mismatch);
        let bad = classify(clock, &header([7; 8], true), &[], "z");
        assert_eq!(bad.status, Status::Mismatch);
    }

    #[test]
    fn unknown_program_is_upgraded_only_by_an_exact_cached_rebuild() {
        let p = AccountId::new([42; 32]);
        let h = header([5; 8], false);
        assert_eq!(classify(p, &h, &[], "z").status, Status::Unknown);
        let source = Source {
            repo: "r".into(),
            commit: "c".into(),
            guest_path: "g".into(),
            bin: "b".into(),
            docker_tag: "t".into(),
            features: None,
        };
        let cached = Verified {
            zone: "z".into(),
            account: p.to_string(),
            image_id: image_hex(&[5; 8]),
            source,
        };
        assert_eq!(
            classify(p, &h, std::slice::from_ref(&cached), "z").status,
            Status::VerifiedLocal
        );
        // Another zone, or an upgraded image, doesn't inherit it.
        assert_eq!(
            classify(p, &h, std::slice::from_ref(&cached), "y").status,
            Status::Unknown
        );
        assert_eq!(
            classify(p, &header([6; 8], false), &[cached], "z").status,
            Status::Unknown
        );
    }
}
