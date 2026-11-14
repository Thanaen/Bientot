//! `plugin.json` as an author may have written it: no file panics the parser, a refusal names
//! its field in both languages, and a manifest that parses keeps the rules the host relies on.

#![no_main]

use bientot_plugin_api::{Language, Manifest};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let manifest = match Manifest::parse(bytes, &["Open settings", "Ouvrir les paramètres"]) {
        Ok(manifest) => manifest,
        Err(error) => {
            error.message(Language::En);
            error.message(Language::Fr);
            return;
        }
    };
    assert!(
        !manifest.locales.is_empty(),
        "a manifest has a fallback locale"
    );
    assert!(
        manifest.memory_min <= manifest.memory,
        "memory_min is within memory"
    );
    for language in [Language::En, Language::Fr] {
        manifest.display_name(language);
        for command in &manifest.commands {
            assert_eq!(manifest.command(&command.name), Some(command));
        }
    }
});
