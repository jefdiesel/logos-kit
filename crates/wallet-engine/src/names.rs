//! Paying a `.lez` name (LNS, Logos Names): resolving it to the private receiving
//! keys, or the public account, its holder published.
//!
//! The chain never sees a name. A name is turned into an identifier and a key here;
//! the registry keeps, at an address derived from the identifier, a record whose
//! profile is sealed under the key. Spec:
//! <https://github.com/jefdiesel/logos-names/blob/main/SPEC.md>.

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead as _, KeyInit as _, Payload},
};
use anyhow::{Context as _, Result, anyhow, bail};
use borsh::BorshDeserialize;
use lee::{AccountId, ProgramShardSelector};
use lee_core::program::PdaSeed;
use sha2::{Digest as _, Sha256};
use wallet::WalletCore;

/// The LNS registry on the official LEZ testnet. `LOGOS_KIT_LNS_REGISTRY` overrides it.
pub const TESTNET_REGISTRY: &str = "2PXg3SSmh5hvECTz4xcfV2d5YvxwyF6yVhpnjTokzCkX";
const SUFFIX: &str = ".lez";
const MASTER_SALT: &[u8] = b"logos-names/v2";
const MASTER_ITERATIONS: u32 = 100_000;
const ID_DOMAIN: &[u8] = b"logos-names/v2/id";
const KEY_DOMAIN: &[u8] = b"logos-names/v2/key";
const RECORD_SEED_DOMAIN: &[u8] = b"logos-names/v2/name/";
const RECORD_AAD: &[u8] = b"logos-names/v2/record";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// A lapsed name still resolves for this long; then anyone may register it.
const GRACE_MS: u64 = 30 * 24 * 60 * 60 * 1000;

#[derive(BorshDeserialize)]
#[cfg_attr(test, derive(borsh::BorshSerialize))]
struct NameRecord {
    _version: u8,
    _owner_tag: [u8; 32],
    expires: u64,
    sealed: Vec<u8>,
}

#[derive(BorshDeserialize)]
#[cfg_attr(test, derive(borsh::BorshSerialize))]
struct Profile {
    name: String,
    account: Option<[u8; 32]>,
    npk: Option<[u8; 32]>,
    vpk: Vec<u8>,
    _texts: Vec<(String, String)>,
}

/// Where a payment to a name goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameTarget {
    /// The name's private receiving keys: the chain sees neither recipient nor amount.
    Private { npk: [u8; 32], vpk: Vec<u8> },
    /// A public account its holder chose to publish.
    Public(AccountId),
}

/// `jef.lez`, any case.
#[must_use]
pub fn is_name(text: &str) -> bool {
    text.trim().to_ascii_lowercase().ends_with(SUFFIX)
}

/// Resolves `name` (`jef.lez`) through the LNS registry: its private keys when it has
/// them, else its public account.
pub async fn resolve(core: &WalletCore, name: &str, now_ms: u64) -> Result<NameTarget> {
    let registry: AccountId = std::env::var("LOGOS_KIT_LNS_REGISTRY")
        .unwrap_or_else(|_| TESTNET_REGISTRY.to_owned())
        .parse()
        .map_err(|_| anyhow!("LOGOS_KIT_LNS_REGISTRY is not an account id"))?;
    let label = normalize(name);
    let (id, key) = derive(&label);
    let record = record_address(&registry, &id);
    let account = core
        .get_account_view(ProgramShardSelector::new(record, registry))
        .await
        .with_context(|| format!("read {label}{SUFFIX}'s record"))?;
    target(&label, &key, account.data.shard(registry).as_ref(), now_ms)
}

fn normalize(name: &str) -> String {
    let name = name.trim().to_ascii_lowercase();
    name.strip_suffix(SUFFIX).unwrap_or(&name).to_owned()
}

fn derive(label: &str) -> ([u8; 32], [u8; 32]) {
    let master =
        pbkdf2::pbkdf2_hmac_array::<Sha256, 32>(label.as_bytes(), MASTER_SALT, MASTER_ITERATIONS);
    (hash(ID_DOMAIN, &master), hash(KEY_DOMAIN, &master))
}

fn record_address(registry: &AccountId, id: &[u8; 32]) -> AccountId {
    AccountId::for_public_pda(registry, &PdaSeed::new(hash(RECORD_SEED_DOMAIN, id)))
}

fn target(label: &str, key: &[u8; 32], record: &[u8], now_ms: u64) -> Result<NameTarget> {
    if record.is_empty() {
        bail!("{label}{SUFFIX} is not registered");
    }
    let record = NameRecord::try_from_slice(record).context("malformed LNS record")?;
    if now_ms >= record.expires.saturating_add(GRACE_MS) {
        bail!("{label}{SUFFIX} has expired");
    }
    let profile = open(key, &record.sealed)
        .filter(|p| p.name == label)
        .ok_or_else(|| anyhow!("{label}{SUFFIX} has no readable record"))?;
    match (profile.npk, profile.vpk.is_empty(), profile.account) {
        (Some(npk), false, _) => Ok(NameTarget::Private {
            npk,
            vpk: profile.vpk,
        }),
        (_, _, Some(account)) => Ok(NameTarget::Public(AccountId::new(account))),
        _ => bail!("{label}{SUFFIX} has no payment destination"),
    }
}

fn open(key: &[u8; 32], sealed: &[u8]) -> Option<Profile> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return None;
    }
    let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
    let plain = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key))
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: RECORD_AAD,
            },
        )
        .ok()?;
    // Zero-padded: decode from the start and ignore the padding.
    Profile::deserialize(&mut plain.as_slice()).ok()
}

fn hash(domain: &[u8], data: &[u8]) -> [u8; 32] {
    Sha256::new()
        .chain_update(domain)
        .chain_update(data)
        .finalize()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // jef's record on the testnet registry above, fetched 2026-10-16.
    const JEF_RECORD_HEX: &str = concat!(
        "04f14ac788881df29017740ab2d0b382811fc305efaffda30d97f5cfd04c8d12892fd05b9ca80100001c0500",
        "005639544cfd5db0b24de6742fb54530bf7df45b6d445e5f22a93cc132f275a618017967d2bca1ae1a2efd1f",
        "518802f95c831a3990083e047be664d072bc66b149c133ee3347ad8b6e775a1d883a3c7a0a6bdcaf9eef2238",
        "e19fc73235d05e1cd2873fadfc9bc092265efefdca85a06989346c61df7631776d2693082c7c8e088ae62b05",
        "cfdf784a2d1a9ae95aca9f3d4b5b79789147a3d5675b232b6ee96fa49ef3bec0fca14d3330301f3529b9cf90",
        "18c13e2bedd0d0c4727938443468d1f162a28d32a031da9078f3b118f7744d7133862704f5c29896b78a4194",
        "972699fd3816e18e820287b3b7232040d12c7dc294a220acb5a17ab9b0bccec8ed21a47b2eeb34cf709d13c1",
        "166bcd14f034623af10dca2e33a751bb54d1628f6fba4537fcf61f2b306fc35538aae381381af6ea825cc308",
        "be79c4439ec3a3aa105eb300271f8be145bb904aa831333236e0252510871706a4e642bf2cabe733e6c95795",
        "3960bcb0c3a2358bc0ca44b0ce6d8b295bb3ba9e1c235f6db985ae8d9108dd611c708bafc0b0c41accea4423",
        "fab2c832018963112a947ac195fdf2c4a6313362d259c819d28999ed08e07c2b4fbaa5be0f5b26499ec41c64",
        "d2b96646639ff0ee6bbe1dec9e84b841f6c44f6be55d48f2ff0e77f85ad02c12cbcb2c7e78afaaf64070754c",
        "5e749874922d2b3e36ae173a710d387bcb1d0c42324960ef2abd716f4b6f963c48f236946fb386248586cb15",
        "da8c986698e0fec3dee4d435eb79bd5ec5d755ce513fb1ed20ab2eadc87ad498fab2ab45f818e4a4d4ed5abf",
        "098ca128d468b117baa7fa88e65a242e1e0b586e8d83357eed12443b296e1e7f89a076ad5f8acd84951d9d3c",
        "0f7b8c057e65845a9e24c7a07d3f3017524d6d08ec84139bea7ef22fa286965b5e9c6ee9b01eef84569bed37",
        "f2fc26c40533ab0cc366c93916748a2925fc25e981d9d6a6c9e1f0131bcb820ea22eb6a0b1bdf9ec4be29b3e",
        "88b8508ffbd77a135ad0cecbbffedd78d07f4d715cdeb73c97f8a92d0b2b9542b4b7c69e57409b429700ca82",
        "460b7b122d8653ae183cfa93b66f8966d6e7ced0ae43cecd01801202a39e9c57c2059124de408107d9e6e529",
        "55329340f0d2c6130757aaa67f206c39cf39239a836c9c8abbfe3d6ed58cef44cff7e59662a8bffbacb0ec09",
        "6790862ede6e9e392a68f0b3b3154f27eb3592ddc004d1bf50d189afa3213445cce3739feb0eff14fd095b29",
        "b60949afa2d526295727418edc63fddc5a98f5f70c07ccb41765848c69f4a70081afa85a63b9b1af2b56ac19",
        "b36923c774867a3254319ed3b520fd9ac72113601fcae5d25cf8902388edc4921835a05637133f2cd894ce14",
        "7426c7aa8d30e88c93c998408cf5cd996df6dfce18442a2cd1e4c8ffc1916c8bea00828dd73e395a023e9d92",
        "6a4445514a00802d33abd2b9d1011d35e908d873799c3f74b36ae5272b73fdace91714089b3af15b6b8a0a3e",
        "b34b55e8b697a27623c1809f7388a06b48c65720014528927d1dd6b0421a2add2f52b59a4f39ec54b85fc32a",
        "9096f218ea7effc1e6979f591890890ef16e63d192c86388d44a4ffca062963de595182dff46f62d6477427e",
        "976b9de0a3bea0aa923ab83e3ad430f0aaef7fcef1c2769848b690f9b8440ffb47875568d712281bdba713eb",
        "14fdd2bf7dee2ce8e367220a89dc3efe7eb3fe945b824e051d56764511e51637f6fa6ac82203618f1ccae961",
        "0fb84589b70b0f59e5e8611abef082914a4f9c84007cd790dad4255cc70425f1643e8008c608e86e7c7097ec",
        "98929326f99704e0cdb41719f9191781ff5248ca213c28d9fa33bdd7e1c5240b4c",
    );

    #[test]
    fn resolves_a_real_record() {
        let registry: AccountId = TESTNET_REGISTRY.parse().unwrap();
        let (id, key) = derive("jef");
        assert_eq!(
            record_address(&registry, &id).to_string(),
            "3pNoRoWnhMacEP2LvfjHYkvDso8URHsioZ8hwEN73D6E"
        );
        let record = hex::decode(JEF_RECORD_HEX).unwrap();
        let NameTarget::Private { vpk, .. } =
            target("jef", &key, &record, 1_792_160_000_000).unwrap()
        else {
            panic!("jef publishes private receiving keys");
        };
        assert_eq!(vpk.len(), 1184);
        // The wrong key, an empty record and a long-lapsed name are refused.
        assert!(target("jef", &derive("logos").1, &record, 1_792_160_000_000).is_err());
        assert!(target("jef", &key, &[], 1_792_160_000_000).is_err());
        assert!(target("jef", &key, &record, 1_900_000_000_000).is_err());
    }

    /// A record sealed the way an LNS client seals it, for names the testnet lacks.
    fn sealed_record(label: &str, profile: &Profile) -> Vec<u8> {
        let (_, key) = derive(label);
        let mut plain = borsh::to_vec(profile).unwrap();
        plain.resize(plain.len().div_ceil(256) * 256, 0);
        let nonce = [7u8; NONCE_LEN];
        let mut sealed = nonce.to_vec();
        sealed.extend(
            Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key))
                .encrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &plain,
                        aad: RECORD_AAD,
                    },
                )
                .unwrap(),
        );
        borsh::to_vec(&NameRecord {
            _version: 3,
            _owner_tag: [0; 32],
            expires: 1_800_000_000_000,
            sealed,
        })
        .unwrap()
    }

    #[test]
    fn a_public_only_name_pays_its_account_and_private_keys_win_over_it() {
        let account = [9u8; 32];
        let public_only = Profile {
            name: "shop".into(),
            account: Some(account),
            npk: None,
            vpk: Vec::new(),
            _texts: Vec::new(),
        };
        let (_, key) = derive("shop");
        let now = 1_792_160_000_000;
        assert_eq!(
            target("shop", &key, &sealed_record("shop", &public_only), now).unwrap(),
            NameTarget::Public(AccountId::new(account))
        );
        let both = Profile {
            npk: Some([1; 32]),
            vpk: vec![2; 1184],
            ..public_only
        };
        assert!(matches!(
            target("shop", &key, &sealed_record("shop", &both), now).unwrap(),
            NameTarget::Private { .. }
        ));
        assert!(is_name("JEF.lez") && is_name(" jef.lez ") && !is_name("jef"));
    }

    /// Reads the live LEZ testnet: `cargo test -p wallet-engine --lib names:: -- --ignored`.
    #[test]
    #[ignore = "reads the live LEZ testnet"]
    fn resolves_names_on_the_testnet() {
        let registry: AccountId = TESTNET_REGISTRY.parse().unwrap();
        let read = |label: &str| -> Vec<u8> {
            let (id, _) = derive(label);
            let body = serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "getAccount",
                "params": [record_address(&registry, &id).to_string()],
            });
            let reply: serde_json::Value = ureq::post("https://testnet.lez.logos.co")
                .send_json(body)
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap();
            reply["result"]["data"]["shards"][TESTNET_REGISTRY]
                .as_array()
                .map(|bytes| bytes.iter().map(|b| b.as_u64().unwrap() as u8).collect())
                .unwrap_or_default()
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        for label in ["jef", "logos"] {
            let (_, key) = derive(label);
            let target = target(label, &key, &read(label), now)
                .unwrap_or_else(|e| panic!("{label}.lez: {e}"));
            assert!(
                matches!(target, NameTarget::Private { ref vpk, .. } if vpk.len() == 1184),
                "{label}.lez"
            );
        }
        let (_, key) = derive("lns-unregistered-check");
        assert!(
            target(
                "lns-unregistered-check",
                &key,
                &read("lns-unregistered-check"),
                now
            )
            .is_err()
        );
    }
}
