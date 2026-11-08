//! The view tree a plugin sends with `bt:ui@1 set`: no tree panics the reader or the checks, and
//! a card that passes holds to the limits the flyout draws within.

#![no_main]

use std::sync::OnceLock;

use bientot_plugin_api::locale::TextLookup;
use bientot_plugin_api::view::{MAX_ACTIONS, MAX_LINES, ViewContext};
use bientot_plugin_api::{Card, LocaleTexts, Manifest};
use libfuzzer_sys::fuzz_target;

struct Plugin {
    manifest: Manifest,
    en: LocaleTexts,
    fr: LocaleTexts,
}

fn plugin() -> &'static Plugin {
    static PLUGIN: OnceLock<Plugin> = OnceLock::new();
    PLUGIN.get_or_init(|| Plugin {
        manifest: Manifest::parse(
            br#"{"id":"bientot.focus","name":{"en":"Focus"},"version":"0.1.0","api":"1.0",
                "locales":["en","fr"],"capabilities":["read:day","ui:flyout"],
                "commands":[{"name":"toggle-range","title":{"en":"Show today or this week"}}]}"#,
            &[],
        )
        .expect("the fuzz manifest parses"),
        en: LocaleTexts::from_pairs([
            ("longest", "Longest focus"),
            ("since-break", "Since your break"),
            ("leave", "You can leave at"),
            ("show-week", "This week"),
            ("hello", "Hello {name}, {missing} stays"),
        ]),
        fr: LocaleTexts::from_pairs([("longest", "Plus longue concentration")]),
    })
}

fuzz_target!(|bytes: &[u8]| {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return;
    };
    let Ok(card) = Card::from_value(value) else {
        return;
    };
    let plugin = plugin();
    let context = ViewContext {
        texts: TextLookup {
            preferred: Some(&plugin.fr),
            fallback: &plugin.en,
        },
        manifest: &plugin.manifest,
    };
    let Ok(prepared) = card.prepare(&context) else {
        return;
    };
    assert!(prepared.lines.len() <= MAX_LINES);
    assert!(prepared.actions.len() <= MAX_ACTIONS);
    for action in &prepared.actions {
        assert!(plugin.manifest.command(&action.command).is_some());
    }
    assert_eq!(
        card.prepare(&context).ok(),
        Some(prepared),
        "checks are deterministic"
    );
});
