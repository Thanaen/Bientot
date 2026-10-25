use serde_json::Value;

use super::*;

/// The secrets of `DpapiSecretStoreTests.SavedSecretSurvivesReopeningTheStore`.
pub(super) const SECRETS: [&str; 3] = [
    "fictional-activitywatch-token-123456789",
    "clé-secrète-世界-🔑\nsecond line",
    "",
];

pub(super) fn id(identifier: &str) -> SecretId {
    SecretId::new(identifier).unwrap()
}

/// The behaviour every store shares, from `DpapiSecretStoreTests`. `open` opens the same store
/// again, as a later launch would.
pub(super) fn saved_secret_survives_reopening<S: SecretStore>(open: impl Fn() -> S, secret: &str) {
    open().save(&id("activitywatch"), secret).unwrap();

    assert_eq!(
        open().get(&id("activitywatch")).unwrap().as_deref(),
        Some(secret)
    );
}

pub(super) fn replacement_and_deletion_leave_other_identifiers_intact<S: SecretStore>(
    open: impl Fn() -> S,
) {
    let store = open();
    assert_eq!(store.get(&id("first")).unwrap(), None);
    store.save(&id("first"), "original-secret").unwrap();
    store.save(&id("second"), "independent-secret").unwrap();
    assert_eq!(
        store.get(&id("first")).unwrap().as_deref(),
        Some("original-secret")
    );

    store.save(&id("first"), "replacement-secret").unwrap();
    let reopened = open();
    assert_eq!(
        reopened.get(&id("first")).unwrap().as_deref(),
        Some("replacement-secret")
    );

    reopened.delete(&id("first")).unwrap();
    reopened.delete(&id("first")).unwrap();
    assert_eq!(reopened.get(&id("first")).unwrap(), None);
    assert_eq!(
        reopened.get(&id("second")).unwrap().as_deref(),
        Some("independent-secret")
    );
}

#[test]
fn memory_store_keeps_saved_secrets() {
    for secret in SECRETS {
        let store = MemorySecretStore::default();
        saved_secret_survives_reopening(|| &store, secret);
    }
}

#[test]
fn memory_store_replaces_and_deletes_one_identifier_at_a_time() {
    let store = MemorySecretStore::default();
    replacement_and_deletion_leave_other_identifiers_intact(|| &store);
}

#[test]
fn blank_identifiers_are_rejected() {
    for identifier in ["", " ", "\t\r\n", "\u{3000}"] {
        assert_eq!(
            SecretId::new(identifier),
            Err(BlankSecretId),
            "{identifier:?}"
        );
    }
    assert_eq!(id(" padded ").as_str(), " padded ");
}

/// `tests/fixtures/secrets.json` holds the file name and the DPAPI entropy the .NET
/// `DpapiSecretStore` derives from each identifier.
#[test]
fn file_names_and_entropy_match_dotnet() {
    let cases: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/secrets.json")).unwrap();
    let cases = cases.as_array().unwrap();
    assert_eq!(cases.len(), 7);
    for case in cases {
        let identifier = case["identifier"].as_str().unwrap();
        let id = id(identifier);
        assert_eq!(id.file_name(), case["file"], "{identifier:?}");
        let entropy: String = id
            .entropy()
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect();
        assert_eq!(entropy, case["entropy"], "{identifier:?}");
    }
}
