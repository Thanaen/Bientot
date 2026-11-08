//! The revocation list as the app fetches it: no response panics the check, and any JSON object
//! signed the way `bientot-plugin sign-revocations` signs is never refused for its signature, so
//! that the signer and the app agree on the canonical form of every list.

#![no_main]

use std::sync::OnceLock;

use bientot_plugins::base64;
use bientot_plugins::revocation::{self, Unavailable};
use ed25519_dalek::{Signer, SigningKey};
use jiff::Timestamp;
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fn maintainer() -> &'static SigningKey {
    static KEY: OnceLock<SigningKey> = OnceLock::new();
    KEY.get_or_init(|| SigningKey::from_bytes(&[7; 32]))
}

fuzz_target!(|bytes: &[u8]| {
    let key = maintainer();
    let now: Timestamp = "2026-11-09T08:00:00Z".parse().expect("a time");
    let _ = revocation::verify(bytes, &key.verifying_key(), None, now);

    let Ok(Value::Object(mut object)) = serde_json::from_slice::<Value>(bytes) else {
        return;
    };
    object.remove("signature");
    let Ok(signed) = revocation::canonical(&Value::Object(object.clone())) else {
        return;
    };
    let signature = key.sign(signed.as_bytes());
    object.insert(
        "signature".into(),
        Value::String(base64::encode(&signature.to_bytes())),
    );
    let served = serde_json::to_vec_pretty(&Value::Object(object)).expect("a list serializes");
    match revocation::verify(&served, &key.verifying_key(), None, now) {
        Err(Unavailable::BadSignature) => panic!("a list signed here does not check"),
        Err(Unavailable::TooLarge) | Err(Unavailable::Malformed(_)) | Err(Unavailable::Expired) => {
        }
        Err(error) => panic!("unexpected refusal with no last list: {error}"),
        Ok(list) => assert!(list.expires > now),
    }
});
