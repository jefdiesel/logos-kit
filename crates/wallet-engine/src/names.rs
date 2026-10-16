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
use argon2::{Algorithm, Argon2, Params, Version};
use borsh::BorshDeserialize;
use lee::{AccountId, ProgramShardSelector};
use lee_core::program::PdaSeed;
use sha2::{Digest as _, Sha256};
use wallet::WalletCore;

/// The LNS registry on the official LEZ testnet. `LOGOS_KIT_LNS_REGISTRY` overrides it.
pub const TESTNET_REGISTRY: &str = "38jEfVV1RCyMzu1FGbYcCn5FqxCxKyY58VgzjruAh2gq";
const SUFFIX: &str = ".lez";
/// One fixed salt for every network: a name keeps its identifier and key from
/// testnet to mainnet.
const MASTER_SALT: &[u8] = b"logos-names/v5";
/// Argon2id memory in KiB (64 MiB), passes and lanes.
const MASTER_MEMORY_KIB: u32 = 65_536;
const MASTER_PASSES: u32 = 3;
const MASTER_LANES: u32 = 1;
const ID_DOMAIN: &[u8] = b"logos-names/v5/id";
const KEY_DOMAIN: &[u8] = b"logos-names/v5/key";
const RECORD_SEED_DOMAIN: &[u8] = b"logos-names/v5/name/";
const RECORD_AAD: &[u8] = b"logos-names/v5/record";
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
    let params = Params::new(MASTER_MEMORY_KIB, MASTER_PASSES, MASTER_LANES, Some(32))
        .expect("the Argon2 parameters are valid");
    let mut master = [0_u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(label.as_bytes(), MASTER_SALT, &mut master)
        .expect("Argon2 accepts these inputs");
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
    // A name held by a deposit has `expires == u64::MAX` and never lapses.
    if record.expires != u64::MAX && now_ms >= record.expires.saturating_add(GRACE_MS) {
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

    // jef's record on the testnet registry above (v5, held by a deposit, so
    // `expires` is u64::MAX), fetched 2026-10-16.
    const JEF_RECORD_HEX: &str = concat!(
        "051f7a805e586fde56a650481a62c11fd879b1831d43ecce56376a340c556ab992ffffffffffffffff1c0700",
        "00a024fe171e22a88540cab1e7e43c32d7d4b6ea212446bb237f1582b7f88139c30937a272d40a8a42de50ae",
        "507786beb9ece14459206f5f4b516008031ee92260f4a494f4c7af459ad50988f5757ec99b05fa45ebd56fea",
        "3d30c6d80d72d98113c250296d0d7e70d84b20eaf0bd8fd20ba8afc594768246119e85732a7b7fddd2dadc47",
        "5d09887e0f7fc5a42e039d7b3090b53fb57b79c2278f79243d40cee3d843717ad352f4609e7f3057fd570260",
        "5463054fd928bc818dd0d3fbf66e8512fe3efe6954e3b7fc361d685fabf14112f4a33d643b0124ca57c56ef6",
        "98ab00eb0b536e2dd1b42787870e0b6f33fec7a063c213233b408ad70381555defd01ad7d1b78bcdb616dfd4",
        "f734b55f35ebdfa483ddc6dd83cc200b2ed4e8dbf45adc98b1f15f4c31fecd36a46d0a82727b3c83411fe689",
        "0a5bb7cb2d9c8f3439a624bd930c7d338a728cc851dcefe1325ae4c5e4ac517e6a36df5932b360eb878b824f",
        "2b2a243aad79fbeaa74800b2566864ff31f24001805b8983cc99ea4d1d556a0efcaf4739bfafca575b9f38c6",
        "10df3007872ee0b5d66910ce5f50536090a49a098ed762ee7fbcd97c7f38f396bce5c82f930988f09dc1dc60",
        "322e69d2d960e42f9ab3763f2605c92c0dc34d1627ded81633e3bad2045d15db9d9d1c346887a3508c773a3a",
        "fe0dd4680b679d073fa2bdeab64d8f34bb85a78fbfe3558e3baacbafdf73e0584cd393b27218d1794af002c3",
        "5b43ee7e0a48cc09f31e0e9076933b16ec5bf5b02afc8511ccde19d3a2f90e2db4452639141828d0e348e33f",
        "fbdbef7eccd1377fd2c7552114565794dc66b1c53848e8d155b7e64edf99a428ce0c97451915608f96370ec5",
        "5f855dadaae1e4592ec66ad8b7abfdbfaf98bfbe068bbe93976f61a1ce233fba5485f6ca54710280cac810ba",
        "7106aba95adcace1e9c7c16a5569a720acae4fc13f8aca4ffb1e40bf3feeb2c98bfd54f437112664d41d3834",
        "a78a1d1cf9c185c5826a7a706be97ac1afcce4245a61fba380285ef1f4d1fbab9d9f1a5607e34f543481c214",
        "fefa5530c4e51cc7ae074ae11563ae74e6939bc648d1a0d0946a97bdcad8c7702cf8c534020e7edefea3c27b",
        "6a0f4628468ba57968c8ee9809afd1a00a737863eaa9dccbe0d85b02d8af8882df78b2cc100cd00d2402a1af",
        "fdab62cce7d57ad17a6832fb9c547fd5831f1585fb25ba15ae23c89e59fdc189c4cbed4d116a6dec1f2b6708",
        "a650b71d1e033840fe8f19b6542678150a2ac1987ed5c4f54f07797e3da192f4fcebe6a8d2fc8b2447698b2d",
        "fe067cff8162fa58e8b0d79f819ea419f5a1ee2cd79020c1a9436a3084c4681d9252560f5e186f36b3d48850",
        "57d58a60e4c8f360d2fbd7f2e27598f78c1e5abfbfb7b617521184c74996676209ce2c5e8451f887ed3caf68",
        "57ae27fddfd239d5713141ee4245807ee3249f60bcd9ee48ee44b12d2320d8fd6b54aefbc75e4957a5744726",
        "d8401e5028e3d9bb6b5bd70e4785fac6e1d7332f7892bd2265dbfbba336b7457eebd4fd6aa93c671dd5258bd",
        "ae89baa4fc65a7a7f83741f4866ba35944996919d46fa614408c9dd5338b3ad38204c7e2e97e3818a4ed9520",
        "3b7629cf6981ee8505983aa5c351f11e22cc358aa2cb8c8894d482b7f857f75adf295704bbdd1272acc1e775",
        "d47b0c321c1a20d007edc5bd6f5778db003d63f5df44c90f114af2b9515265781fa931fb34800c7bd0ce1b8f",
        "5d79b5a5c4daa3e4cb8bc80fc73598ed4bf7eb92074c933bc390e4aad3c879e1b17c6714b688ad5186d8e903",
        "140ce62355164489d75ff313e619f5523f49207934477d717a78effcc5ff008477a331a785ddf759643f4131",
        "fad408366c695c9e2e39448b89b92e36ab433931862731a0dc77cd48af0e0a1ad3828323e5f40bbe2ade3d05",
        "e9d60b5d9697868fe7f43f4c704d87247b05e96eb4a34cdeab40143b352becb0f78b365a6356a463d2243e60",
        "2389c0c0c50bbdc10f884861af74be4b5fa30d31ad44783125bb7afd1b0830faf8de98e073ade1a876267c76",
        "e679b59105baf75aa6bac7526ed5f4c7c9c042203a95ccf261d067da08bfe7523cb7cc3fbbe295764b80d033",
        "0939e4544f19a023a72132a58d8fc0fd1aa5f4f624a186275127c5a50462528b1d82c96790531cfebb2d4d1f",
        "909704090592854a95caf5d5bcdc95b0b76e0930e6e6973259a7ba84a5e061ad96c4ba2a74f650c345006000",
        "107212f1251ce40923fb867b91d42ead65c314177451449bb9f603fda7c34911c6f4f6a0bbb98e23202660c1",
        "c94f91ef3c1cadee1a1a5e0103470ae6ef27d911486968361341c188346bc9d965b46120016ded026a1d92cf",
        "f43cdc56b321c18f8e1b72f489244eab5d6c5ffe4d993403ef90253a011072dcf352396af90c3ae9eab24f3a",
        "2675dc0e7e00a3384159302ec02a1652c5a26bac4db760c08fa74151466e1839430366a4d90bc52310fdb529",
        "37b94e70a85f4b303eb8d5431eef629dfca480cc6987ae8eb20d16bf16f91210c9b0a620636d649eb14de820",
        "d23c75ba14b145a82196f53f878973ccaa",
    );

    #[test]
    fn derives_the_specification_vectors() {
        let (id, key) = derive("jef");
        assert_eq!(
            hex::encode(id),
            "7041d9e447f06bdfe327d870402b1524cfe38bd03445b8fc3d0e87bc978eb763"
        );
        assert_eq!(
            hex::encode(key),
            "aad91a1a3dd0fb56a213212fbea58fe25d9139db42fc59ce2116c3cfa3440088"
        );
    }

    #[test]
    fn resolves_a_real_record() {
        let registry: AccountId = TESTNET_REGISTRY.parse().unwrap();
        let (id, key) = derive("jef");
        assert_eq!(
            record_address(&registry, &id).to_string(),
            "Cu8tskY2uaMA14EiMFJd8jGD5MreMw1XkvRheTJZvAk"
        );
        let record = hex::decode(JEF_RECORD_HEX).unwrap();
        let NameTarget::Private { vpk, .. } =
            target("jef", &key, &record, 1_792_160_000_000).unwrap()
        else {
            panic!("jef publishes private receiving keys");
        };
        assert_eq!(vpk.len(), 1184);
        // The wrong key and an empty record are refused.
        assert!(target("jef", &derive("logos").1, &record, 1_792_160_000_000).is_err());
        assert!(target("jef", &key, &[], 1_792_160_000_000).is_err());
        // Held by a deposit: never lapses.
        assert!(target("jef", &key, &record, u64::MAX - 1).is_ok());
    }

    #[test]
    fn a_term_name_lapses_after_its_grace() {
        let (_, key) = derive("jef");
        let mut record = hex::decode(JEF_RECORD_HEX).unwrap();
        record[33..41].copy_from_slice(&1_000_u64.to_le_bytes());
        assert!(target("jef", &key, &record, 1_000 + GRACE_MS - 1).is_ok());
        assert!(target("jef", &key, &record, 1_000 + GRACE_MS).is_err());
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
            _version: 5,
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
