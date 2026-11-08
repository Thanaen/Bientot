use std::collections::{BTreeMap, BTreeSet};

use bientot_plugin_api::Capability;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;

use super::*;
use crate::package;
use crate::state::{Grant, InstalledPlugin, SCHEMA, Status};

fn maintainer() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

/// `fields`, signed by `key` over their canonical JSON, as the server serves the list.
fn signed(fields: Value, key: &SigningKey) -> Vec<u8> {
    let signature = key.sign(canonical(&fields).unwrap().as_bytes());
    let mut object = fields.as_object().unwrap().clone();
    object.insert(
        "signature".into(),
        Value::String(base64::encode(&signature.to_bytes())),
    );
    serde_json::to_vec_pretty(&Value::Object(object)).unwrap()
}

fn list(version: u64, revoked: Value) -> Value {
    json!({
        "version": version,
        "expires": "2027-11-09T00:00:00Z",
        "level2_disabled": false,
        "revoked": revoked,
    })
}

fn now() -> Timestamp {
    "2026-11-09T08:00:00Z".parse().unwrap()
}

fn id(text: &str) -> PluginId {
    PluginId::parse(text).unwrap()
}

fn plugin(trust: Trust, status: Status) -> InstalledPlugin {
    InstalledPlugin {
        version: "1.0.0".into(),
        trust,
        files: BTreeMap::new(),
        manifest_hash: "cd".repeat(32),
        granted: Grant {
            capabilities: BTreeSet::from([Capability::ReadDay]),
            manifest_hash: "cd".repeat(32),
        },
        status,
        failures: Vec::new(),
        card_preference: false,
        hidden_on: None,
        rank: 0,
        pending: None,
    }
}

fn unsigned() -> Trust {
    Trust::Unsigned {
        package_hash: "00".repeat(32),
    }
}

fn signed_by(author: &SigningKey) -> Trust {
    Trust::Signed {
        key: base64::encode(author.verifying_key().as_bytes()),
        announced: Vec::new(),
    }
}

#[test]
fn a_list_signed_by_the_maintainer_parses_with_its_reasons() {
    let key = maintainer();
    let bytes = signed(
        list(
            3,
            json!([
                { "id": "com.example.bad", "reason": { "en": "Sends data", "fr": "Envoie des données" } },
                { "key": "ab12-cd34-ef56-7890-ab12-cd34-ef56-7890", "reason": { "en": "Key leaked" } }
            ]),
        ),
        &key,
    );
    let list = verify(&bytes, &key.verifying_key(), None, now()).unwrap();
    assert_eq!(list.version, 3);
    assert!(!list.level2_disabled);
    assert_eq!(
        list.revoked,
        [
            Revoked {
                target: Target::Plugin(id("com.example.bad")),
                reason_en: "Sends data".into(),
                reason_fr: "Envoie des données".into(),
            },
            Revoked {
                target: Target::Key("ab12cd34ef567890ab12cd34ef567890".into()),
                reason_en: "Key leaked".into(),
                reason_fr: "Key leaked".into(),
            },
        ]
    );
}

#[test]
fn a_list_is_refused_when_its_signature_version_expiry_or_size_is_wrong() {
    let key = maintainer();
    let verifying = key.verifying_key();
    let good = verify(&signed(list(3, json!([])), &key), &verifying, None, now()).unwrap();

    let stranger = SigningKey::from_bytes(&[9; 32]);
    let by_stranger = signed(list(4, json!([])), &stranger);
    assert_eq!(
        verify(&by_stranger, &verifying, Some(&good), now()),
        Err(Unavailable::BadSignature)
    );

    // A field changed after signing.
    let mut tampered: Value = serde_json::from_slice(&signed(list(4, json!([])), &key)).unwrap();
    tampered["level2_disabled"] = Value::Bool(true);
    let tampered = serde_json::to_vec(&tampered).unwrap();
    assert_eq!(
        verify(&tampered, &verifying, Some(&good), now()),
        Err(Unavailable::BadSignature)
    );

    let older = signed(list(2, json!([])), &key);
    assert_eq!(
        verify(&older, &verifying, Some(&good), now()),
        Err(Unavailable::Older {
            version: 2,
            last: 3
        })
    );
    // The same version again is the same list fetched again.
    assert!(
        verify(
            &signed(list(3, json!([])), &key),
            &verifying,
            Some(&good),
            now()
        )
        .is_ok()
    );

    let mut expired = list(4, json!([]));
    expired["expires"] = json!("2026-11-01T00:00:00Z");
    assert_eq!(
        verify(&signed(expired, &key), &verifying, Some(&good), now()),
        Err(Unavailable::Expired)
    );

    let huge = vec![b' '; MAX_BYTES + 1];
    assert_eq!(
        verify(&huge, &verifying, None, now()),
        Err(Unavailable::TooLarge)
    );
    assert!(matches!(
        verify(b"[]", &verifying, None, now()),
        Err(Unavailable::Malformed(_))
    ));
}

#[test]
fn the_signature_covers_the_canonical_json_whatever_the_server_spacing() {
    assert_eq!(
        canonical(&json!({ "b": [1, { "d": true, "c": null }], "a": "é\"" })).unwrap(),
        r#"{"a":"é\"","b":[1,{"c":null,"d":true}]}"#
    );
    assert!(canonical(&json!({ "version": 1.5 })).is_err());
}

#[test]
fn a_revoked_id_or_key_turns_plugins_revoked_and_a_later_list_turns_them_off() {
    let author = SigningKey::from_bytes(&[3; 32]);
    let fingerprint = package::fingerprint(&author.verifying_key());
    let mut state = StateFile {
        schema: SCHEMA,
        plugins: BTreeMap::from([
            (id("com.example.bad"), plugin(unsigned(), Status::Active)),
            (
                id("com.example.leaked"),
                plugin(signed_by(&author), Status::Off),
            ),
            (id("com.example.good"), plugin(unsigned(), Status::Active)),
        ]),
    };
    let key = maintainer();
    let revoking = verify(
        &signed(
            list(
                1,
                json!([
                    { "id": "com.example.bad", "reason": { "en": "Malware", "fr": "Logiciel malveillant" } },
                    { "key": fingerprint, "reason": { "en": "Key leaked", "fr": "Clé compromise" } }
                ]),
            ),
            &key,
        ),
        &key.verifying_key(),
        None,
        now(),
    )
    .unwrap();
    assert_eq!(
        apply(&mut state, &revoking),
        [id("com.example.bad"), id("com.example.leaked")]
    );
    assert_eq!(
        state.plugins[&id("com.example.bad")].status,
        Status::Revoked {
            reason_en: "Malware".into(),
            reason_fr: "Logiciel malveillant".into()
        }
    );
    assert!(matches!(
        state.plugins[&id("com.example.leaked")].status,
        Status::Revoked { .. }
    ));
    assert!(state.plugins[&id("com.example.good")].is_active());
    assert_eq!(apply(&mut state, &revoking), Vec::<PluginId>::new());

    let empty = verify(
        &signed(list(2, json!([])), &key),
        &key.verifying_key(),
        Some(&revoking),
        now(),
    )
    .unwrap();
    apply(&mut state, &empty);
    for revoked in ["com.example.bad", "com.example.leaked"] {
        assert_eq!(state.plugins[&id(revoked)].status, Status::Off, "{revoked}");
    }
}

#[test]
fn the_last_good_list_is_kept_and_a_bad_one_leaves_it_in_force() {
    let dir = tempfile::tempdir().unwrap();
    let plugins = PluginsDir::new(dir.path().join("plugins"));
    let key = maintainer();
    let mut state = StateFile {
        schema: SCHEMA,
        plugins: BTreeMap::from([(id("com.example.bad"), plugin(unsigned(), Status::Active))]),
    };
    let revoking = signed(
        list(
            5,
            json!([{ "id": "com.example.bad", "reason": { "en": "Malware" } }]),
        ),
        &key,
    );
    let (list, changed) =
        accept(&plugins, &mut state, &revoking, &key.verifying_key(), now()).unwrap();
    assert_eq!((list.version, changed), (5, vec![id("com.example.bad")]));
    assert_eq!(
        last_good(&plugins, &key.verifying_key()).map(|l| l.version),
        Some(5)
    );

    // An older list that would bring the plugin back is refused; the kept one stays.
    let older = signed(list_with_level2(4, true), &key);
    assert_eq!(
        accept(&plugins, &mut state, &older, &key.verifying_key(), now()).map(|(l, _)| l),
        Err(Unavailable::Older {
            version: 4,
            last: 5
        })
    );
    assert!(matches!(
        state.plugins[&id("com.example.bad")].status,
        Status::Revoked { .. }
    ));
    let kept = last_good(&plugins, &key.verifying_key()).unwrap();
    assert_eq!(kept.version, 5);
    // Long after it expires, the kept list is still the one in force.
    assert!(kept.expires < "2030-01-01T00:00:00Z".parse().unwrap());
    assert_eq!(
        last_good(&plugins, &SigningKey::from_bytes(&[9; 32]).verifying_key()),
        None
    );
}

#[test]
fn level_2_can_be_stopped_from_a_distance() {
    let key = maintainer();
    let list = verify(
        &signed(list_with_level2(1, true), &key),
        &key.verifying_key(),
        None,
        now(),
    )
    .unwrap();
    assert!(list.level2_disabled);
}

fn list_with_level2(version: u64, disabled: bool) -> Value {
    let mut fields = list(version, json!([]));
    fields["level2_disabled"] = Value::Bool(disabled);
    fields
}
