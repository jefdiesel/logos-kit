//! Logos Kit wallet engine.
//!
//! S1 linked the LEZ v0.3 wallet stack; S2 adds the keystore, sessions,
//! zones, sync and auto-lock. Policy, approvals and proving arrive in S3–S4
//! (see docs/dev/PLAN.md).

pub mod activity;
pub mod api;
pub mod auto_lock;
pub mod backup;
pub mod decode;
pub mod engine;
pub mod faucet;
pub mod ffi;
pub mod incoming;
pub mod message;
pub mod policy;
pub mod proving;
pub mod service;
pub mod session;
pub mod testimonial;
pub mod tokens;
pub mod trust;
pub mod tx;
pub mod vault;
pub mod verify;

/// LEZ account id, for callers without a LEZ dependency.
pub use lee::AccountId;

/// LEZ revision this engine is built against (see docs/dev/pins.md).
pub const LEZ_REV: &str = "1a003bfbfb89fcf987f62975cd08a401169e1b8c";

#[cfg(test)]
mod tests {
    #[test]
    fn links_lez_crates() {
        // Touch a type from each crate so the link is real.
        let _ = std::any::type_name::<wallet::WalletCore>();
        let _ = std::any::type_name::<lee_core::account::Account>();
        assert_eq!(super::LEZ_REV.len(), 40);
    }

    #[test]
    fn dispatch_derives_the_vector_accounts() {
        let out = super::api::dispatch(
            r#"{"method":"derivePublicAccounts","params":{"mnemonic":"abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about","count":3}}"#,
        );
        let paths: Vec<&str> = out["result"]["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths, ["/0", "/1", "/0/0"]);
        assert_eq!(super::api::dispatch("nope")["error"]["code"], -32700);
    }
}
