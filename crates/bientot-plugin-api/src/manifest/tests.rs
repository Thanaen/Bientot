use serde_json::{Value, json};

use super::*;

fn sample() -> Value {
    json!({
        "id": "bientot.focus",
        "name": { "en": "Focus", "fr": "Concentration" },
        "version": "0.1.0",
        "api": "1.0",
        "memory": "4MiB",
        "locales": ["en", "fr"],
        "capabilities": ["read:day", "ui:flyout"],
        "events": ["day.refreshed", "language.changed"],
        "commands": [{
            "name": "toggle-range",
            "title": { "en": "Show today or this week", "fr": "Afficher aujourd’hui ou la semaine" },
            "windows": ["flyout", "week"]
        }]
    })
}

const BUILTIN: &[&str] = &["Open settings", "Ouvrir les réglages"];

fn parse(value: &Value) -> Result<Manifest, ManifestError> {
    Manifest::parse(value.to_string().as_bytes(), BUILTIN)
}

fn refusal(value: &Value) -> (String, Problem) {
    let error = parse(value).expect_err("refused");
    (error.field, error.problem)
}

fn with(key: &str, value: Value) -> Value {
    let mut manifest = sample();
    manifest[key] = value;
    manifest
}

fn without(key: &str) -> Value {
    let mut manifest = sample();
    manifest.as_object_mut().unwrap().remove(key);
    manifest
}

#[test]
fn the_contract_example_reads() {
    let manifest = parse(&sample()).unwrap();
    assert_eq!(manifest.id.as_str(), "bientot.focus");
    assert_eq!(manifest.display_name(Language::Fr), "Concentration");
    assert_eq!(manifest.version.to_string(), "0.1.0");
    assert_eq!(manifest.api_minor, 0);
    assert_eq!(manifest.memory, 4 << 20);
    assert_eq!(manifest.memory_min, 1 << 20);
    assert_eq!(manifest.fallback_locale(), "en");
    assert!(manifest.has(Capability::ReadDay) && manifest.has(Capability::UiFlyout));
    assert_eq!(manifest.events.len(), 2);
    assert_eq!(manifest.command("toggle-range").unwrap().windows.len(), 2);
}

#[test]
fn defaults_apply() {
    let mut value = sample();
    let object = value.as_object_mut().unwrap();
    for key in ["memory", "events", "commands", "capabilities"] {
        object.remove(key);
    }
    let manifest = parse(&value).unwrap();
    assert_eq!(manifest.memory, DEFAULT_MEMORY);
    assert_eq!(manifest.memory_min, DEFAULT_MEMORY_MIN);
    assert!(
        manifest.capabilities.is_empty()
            && manifest.events.is_empty()
            && manifest.commands.is_empty()
    );
    assert!(manifest.description.0.is_empty());
}

#[test]
fn required_keys_are_named() {
    for key in ["id", "version", "api", "locales", "name"] {
        assert_eq!(
            refusal(&without(key)),
            (key.to_owned(), Problem::Missing),
            "{key}"
        );
    }
}

#[test]
fn unknown_and_later_keys_are_refused_by_name() {
    assert_eq!(
        refusal(&with("colour", json!("red"))),
        ("colour".to_owned(), Problem::UnknownKey)
    );
    for later in ["settings", "storage", "timers", "net"] {
        assert_eq!(
            refusal(&with(later, json!({}))),
            (later.to_owned(), Problem::NotSupportedYet)
        );
    }
    assert!(
        parse(&with(
            "$schema",
            json!("https://bientot.dev/plugins/manifest-1.json")
        ))
        .is_ok()
    );
}

#[test]
fn bad_values_are_refused() {
    assert_eq!(refusal(&with("id", json!("Focus"))).1, Problem::BadId);
    assert_eq!(
        refusal(&with("id", json!("dev.me.focus"))).1,
        Problem::ReservedId
    );
    assert_eq!(
        refusal(&with("id", json!(3))).1,
        Problem::WrongType(Expected::String)
    );
    assert_eq!(
        refusal(&with("version", json!("1.0.0-beta"))).1,
        Problem::BadVersion
    );
    assert_eq!(
        refusal(&with("api", json!("2.0"))).1,
        Problem::UnsupportedApi
    );
    assert_eq!(
        refusal(&with("api", json!("1.1"))).1,
        Problem::UnsupportedApi
    );
    assert_eq!(
        refusal(&with("api", json!("0.9"))).1,
        Problem::UnsupportedApi
    );
    assert_eq!(refusal(&with("memory", json!("4MB"))).1, Problem::BadSize);
    assert_eq!(refusal(&with("memory", json!(4194304))).1, Problem::BadSize);
    assert_eq!(
        refusal(&with("memory", json!("33MiB"))).1,
        Problem::MemoryTooLarge
    );
    assert_eq!(
        refusal(&with("memory_min", json!("8MiB"))).1,
        Problem::MemoryMinAboveMemory
    );
    assert_eq!(refusal(&with("wasi", json!(true))).1, Problem::WasiRefused);
    assert!(parse(&with("wasi", json!(false))).is_ok());
    assert!(parse(&with("memory", json!("512KiB"))).is_ok_and(|m| m.memory_min == 512 << 10));
}

#[test]
fn locales_and_texts_follow_the_rules() {
    assert_eq!(refusal(&with("locales", json!([]))).1, Problem::NoLocales);
    assert_eq!(
        refusal(&with("locales", json!(["EN"]))),
        ("locales[0]".to_owned(), Problem::BadLocale)
    );
    assert_eq!(
        refusal(&with("locales", json!(["en", "en"]))).1,
        Problem::DuplicateLocale
    );
    assert_eq!(
        refusal(&with("locales", json!(["de", "en"]))),
        ("name".to_owned(), Problem::MissingFallbackText)
    );
    assert_eq!(
        refusal(&with("name", json!({ "en": "x".repeat(41) }))),
        ("name.en".to_owned(), Problem::TooLong(MAX_NAME_CHARS))
    );
    assert!(parse(&with("name", json!({ "en": "é".repeat(40) }))).is_ok());
    assert!(
        parse(&with(
            "description",
            json!({ "en": "Your longest focus today" })
        ))
        .is_ok()
    );
    assert_eq!(
        refusal(&with(
            "description",
            json!({ "fr": "Seulement en français" })
        ))
        .1,
        Problem::MissingFallbackText
    );
}

#[test]
fn capabilities_and_events_are_checked() {
    assert_eq!(
        refusal(&with("capabilities", json!(["read:day", "net:fetch"]))),
        ("capabilities[1]".to_owned(), Problem::UnknownCapability)
    );
    assert_eq!(
        refusal(&with("events", json!(["installed"]))).1,
        Problem::UnknownEvent
    );
    assert_eq!(
        refusal(&with("capabilities", json!(["ui:flyout"]))),
        ("events".to_owned(), Problem::NeedsReadDay)
    );
}

#[test]
fn commands_are_checked() {
    let command = |value: Value| with("commands", json!([value]));
    let title = json!({ "en": "Do it" });
    assert_eq!(
        refusal(&command(
            json!({ "name": "go", "title": title, "shortcut": "Ctrl+G" })
        )),
        ("commands[0].shortcut".to_owned(), Problem::ShortcutRefused)
    );
    assert_eq!(
        refusal(&command(json!({ "name": "Go", "title": title }))),
        ("commands[0].name".to_owned(), Problem::BadCommandName)
    );
    assert_eq!(
        refusal(&command(
            json!({ "name": "go", "title": { "en": "open settings " } })
        )),
        ("commands[0].title".to_owned(), Problem::BuiltinTitle)
    );
    assert_eq!(
        refusal(&command(
            json!({ "name": "go", "title": { "en": "Fine", "fr": "Ouvrir les réglages" } })
        ))
        .1,
        Problem::BuiltinTitle
    );
    assert_eq!(
        refusal(&command(
            json!({ "name": "go", "title": title, "windows": ["tray"] })
        ))
        .1,
        Problem::BadWindow
    );
    assert_eq!(
        refusal(&command(
            json!({ "name": "go", "title": title, "windows": [] })
        ))
        .1,
        Problem::NoWindow
    );
    assert_eq!(
        refusal(&command(
            json!({ "name": "go", "title": title, "icon": "x" })
        )),
        ("commands[0].icon".to_owned(), Problem::UnknownKey)
    );
    let same = json!({ "name": "go", "title": title });
    assert_eq!(
        refusal(&with("commands", json!([same, same]))),
        ("commands[1].name".to_owned(), Problem::DuplicateCommand)
    );
    let many: Vec<Value> = (0..9)
        .map(|i| json!({ "name": format!("c{i}"), "title": title }))
        .collect();
    assert_eq!(
        refusal(&with("commands", json!(many))).1,
        Problem::TooManyCommands
    );
    let defaulted = parse(&command(json!({ "name": "go", "title": title }))).unwrap();
    assert_eq!(defaulted.commands[0].windows.len(), 2);
}

#[test]
fn the_file_itself_is_checked() {
    assert_eq!(
        Manifest::parse(b"{", BUILTIN)
            .unwrap_err()
            .problem
            .message(Language::En)
            .split(' ')
            .next(),
        Some("the")
    );
    assert_eq!(
        Manifest::parse(b"[]", BUILTIN).unwrap_err().problem,
        Problem::WrongType(Expected::Object)
    );
    let big = vec![b' '; MAX_MANIFEST_BYTES + 1];
    assert_eq!(
        Manifest::parse(&big, BUILTIN).unwrap_err().problem,
        Problem::FileTooLarge
    );
}

#[test]
fn messages_name_the_field_in_both_languages() {
    let error = parse(&with(
        "commands",
        json!([{ "name": "go", "title": { "en": "Open settings" } }]),
    ))
    .unwrap_err();
    assert_eq!(
        error.message(Language::En),
        "`commands[0].title`: this title is already used by Bientôt"
    );
    assert_eq!(
        error.message(Language::Fr),
        "`commands[0].title`: ce titre est déjà utilisé par Bientôt"
    );
}

#[test]
fn sizes_parse() {
    assert_eq!(parse_size("4MiB"), Some(4 << 20));
    assert_eq!(parse_size("64KiB"), Some(64 << 10));
    for bad in [
        "",
        "MiB",
        "4 MiB",
        "4mib",
        "-1MiB",
        "0MiB",
        "1.5MiB",
        "9999999MiB",
    ] {
        assert_eq!(parse_size(bad), None, "{bad}");
    }
}
