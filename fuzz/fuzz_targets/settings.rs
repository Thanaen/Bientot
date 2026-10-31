//! `settings.json` as a user may have edited it: no file panics the load, and settings that load
//! save to a file that loads as the same settings, with the members they do not know kept.

#![no_main]

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use bientot_store::settings::SettingsStore;
use libfuzzer_sys::fuzz_target;

fn store() -> &'static SettingsStore {
    static STORE: OnceLock<SettingsStore> = OnceLock::new();
    STORE.get_or_init(|| {
        let directory = scratch_directory("settings");
        SettingsStore::new(directory.join("settings.json")).expect("the scratch path is absolute")
    })
}

fn scratch_directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("bientot-fuzz-{name}-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("the scratch directory is created");
    directory
}

fuzz_target!(|bytes: &[u8]| {
    let store = store();
    fs::write(store.path(), bytes).expect("the scratch file is written");
    let Some(settings) = store.peek() else {
        return;
    };
    store.save(&settings).expect("settings that load save");
    assert_eq!(store.peek(), Some(settings));
    let saved = fs::read(store.path()).expect("the saved file reads");
    if let Some(before) = members(bytes) {
        let after = members(&saved).expect("the saved file is a JSON object");
        for name in before {
            assert!(
                after.iter().any(|kept| kept.eq_ignore_ascii_case(&name)),
                "the member {name:?} was dropped"
            );
        }
    }
});

/// The names of the top-level members of a JSON object, when the bytes are one.
fn members(bytes: &[u8]) -> Option<Vec<String>> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match serde_json::from_slice(bytes).ok()? {
        serde_json::Value::Object(members) => {
            Some(members.into_iter().map(|(name, _)| name).collect())
        }
        _ => None,
    }
}
