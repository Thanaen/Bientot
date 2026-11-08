use serde_json::{Value, json};

use super::*;
use crate::locale::LocaleTexts;

fn manifest() -> Manifest {
    let value = json!({
        "id": "bientot.focus",
        "name": { "en": "Focus" },
        "version": "0.1.0",
        "api": "1.0",
        "locales": ["en", "fr"],
        "capabilities": ["read:day", "ui:flyout"],
        "commands": [{ "name": "toggle-range", "title": { "en": "Show today or this week" } }]
    });
    Manifest::parse(value.to_string().as_bytes(), &[]).unwrap()
}

fn en() -> LocaleTexts {
    LocaleTexts::from_pairs([
        ("longest", "Longest focus"),
        ("since-break", "Since your break"),
        ("leave", "You can leave at"),
        ("show-week", "This week"),
        ("hello", "Hello {name}, {missing} stays"),
        ("streak", "Streak"),
    ])
}

fn fr() -> LocaleTexts {
    LocaleTexts::from_pairs([("longest", "Plus longue concentration")])
}

fn prepare(value: Value) -> Result<PreparedCard, ViewError> {
    let manifest = manifest();
    let (en, fr) = (en(), fr());
    let context = ViewContext {
        texts: TextLookup {
            preferred: Some(&fr),
            fallback: &en,
        },
        manifest: &manifest,
    };
    Card::from_value(value)?.prepare(&context)
}

fn card(lines: Value) -> Value {
    json!({ "type": "card", "lines": lines })
}

fn text(key: &str) -> Value {
    json!({ "type": "text", "key": key })
}

#[test]
fn the_contract_example_prepares() {
    let example = json!({
        "type": "card",
        "lines": [
            { "type": "row", "children": [
                { "type": "text", "key": "longest", "role": "body" },
                { "type": "duration", "value": 5400 }
            ]},
            { "type": "row", "children": [
                { "type": "text", "key": "since-break", "role": "body" },
                { "type": "duration", "value": 0, "since": "2026-11-08T13:42:00Z" }
            ]},
            { "type": "row", "children": [
                { "type": "text", "key": "leave", "role": "muted" },
                { "type": "time", "at": "2026-11-08T16:55:00Z" }
            ]}
        ],
        "actions": [
            { "type": "button", "command": "toggle-range", "label": { "type": "text", "key": "show-week" } }
        ]
    });
    let card = prepare(example).unwrap();
    assert_eq!(card.lines.len(), 3);
    let Prepared::Row(first) = &card.lines[0] else {
        panic!()
    };
    assert_eq!(
        first[0],
        Prepared::Text {
            text: "Plus longue concentration".into(),
            role: Role::Body
        }
    );
    let Prepared::Row(second) = &card.lines[1] else {
        panic!()
    };
    assert_eq!(
        first[1],
        Prepared::Duration {
            value: 5400,
            since: None
        }
    );
    assert!(matches!(second[0], Prepared::Text { ref text, .. } if text == "Since your break"));
    assert_eq!(card.actions[0].label, "This week");
    assert_eq!(card.actions[0].command, "toggle-range");
}

#[test]
fn texts_substitute_only_known_names() {
    let card = prepare(card(
        json!([{ "type": "text", "key": "hello", "args": { "name": "Jo" } }]),
    ))
    .unwrap();
    assert_eq!(
        card.lines[0],
        Prepared::Text {
            text: "Hello Jo, {missing} stays".into(),
            role: Role::Body
        }
    );
    assert_eq!(
        substitute("{a}{b}{", &BTreeMap::from([("a".into(), "{b}".into())])),
        "{b}{b}{"
    );
}

#[test]
fn limits_refuse_the_whole_card() {
    let mut deep = text("streak");
    for _ in 0..6 {
        deep = json!({ "type": "stack", "children": [deep] });
    }
    assert_eq!(prepare(card(json!([deep]))), Err(ViewError::TooDeep));

    let mut five_deep = text("streak");
    for _ in 0..5 {
        five_deep = json!({ "type": "stack", "children": [five_deep] });
    }
    assert!(prepare(card(json!([five_deep]))).is_ok());

    let many: Vec<Value> = (0..64).map(|_| text("streak")).collect();
    assert_eq!(
        prepare(card(json!([{ "type": "row", "children": many }]))),
        Err(ViewError::TooManyNodes)
    );
    let fits: Vec<Value> = (0..62).map(|_| text("streak")).collect();
    assert!(prepare(card(json!([{ "type": "row", "children": fits }]))).is_ok());

    let long = "x".repeat(190);
    assert!(matches!(
        prepare(card(
            json!([{ "type": "text", "key": "hello", "args": { "name": long } }])
        )),
        Err(ViewError::TextTooLong(_))
    ));
    let args: serde_json::Map<String, Value> =
        (0..9).map(|i| (format!("a{i}"), json!("x"))).collect();
    assert_eq!(
        prepare(card(
            json!([{ "type": "text", "key": "hello", "args": args }])
        )),
        Err(ViewError::TooManyArgs)
    );
}

#[test]
fn texts_commands_and_primitives_must_exist() {
    assert_eq!(
        prepare(card(json!([text("nope")]))),
        Err(ViewError::UnknownText("nope".into()))
    );
    assert_eq!(
        prepare(card(json!([{ "type": "chart", "points": [1, 2] }]))),
        Err(ViewError::UnknownPrimitive)
    );
    let button = json!({ "type": "button", "command": "explode", "label": { "type": "text", "key": "show-week" } });
    assert_eq!(
        prepare(json!({ "type": "card", "lines": [], "actions": [button] })),
        Err(ViewError::UnknownCommand("explode".into()))
    );
}

#[test]
fn buttons_live_only_in_actions() {
    let button = json!({ "type": "button", "command": "toggle-range", "label": { "type": "text", "key": "show-week" } });
    assert_eq!(
        prepare(card(json!([button.clone()]))),
        Err(ViewError::ButtonOutsideActions)
    );
    assert_eq!(
        prepare(json!({ "type": "card", "lines": [], "actions": [text("streak")] })),
        Err(ViewError::NotAButton)
    );
    assert_eq!(
        prepare(
            json!({ "type": "card", "lines": [], "actions": [button.clone(), button.clone(), button] })
        ),
        Err(ViewError::TooManyActions)
    );
}

#[test]
fn values_are_checked() {
    assert_eq!(
        prepare(card(json!([{ "type": "duration", "value": -1 }]))),
        Err(ViewError::BadDuration)
    );
    assert_eq!(
        prepare(card(json!([{ "type": "duration", "value": 604801 }]))),
        Err(ViewError::BadDuration)
    );
    assert!(matches!(
        prepare(card(json!([{ "type": "time", "at": "tomorrow" }]))),
        Err(ViewError::Malformed(_))
    ));
    let progress = |value: f64, max: f64| {
        card(
            json!([{ "type": "progress", "value": value, "max": max, "label": { "type": "text", "key": "streak" } }]),
        )
    };
    assert!(prepare(progress(3.0, 4.0)).is_ok());
    assert_eq!(prepare(progress(5.0, 4.0)), Err(ViewError::BadProgress));
    assert_eq!(prepare(progress(0.0, 0.0)), Err(ViewError::BadProgress));
    assert!(matches!(
        prepare(card(json!([{ "type": "progress", "value": 1, "max": 2 }]))),
        Err(ViewError::Malformed(_))
    ));
    assert!(matches!(
        prepare(json!({ "type": "page", "lines": [] })),
        Err(ViewError::Malformed(_))
    ));
}

#[test]
fn extra_lines_are_cut_to_four() {
    let lines: Vec<Value> = (0..6).map(|_| text("streak")).collect();
    assert_eq!(prepare(card(json!(lines))).unwrap().lines.len(), MAX_LINES);
}

#[test]
fn unknown_roles_and_tones_still_draw() {
    let card = prepare(card(json!([
        { "type": "text", "key": "streak", "role": "shout" },
        { "type": "badge", "text": { "type": "text", "key": "streak" }, "tone": "rainbow" }
    ])))
    .unwrap();
    assert_eq!(
        card.lines[0],
        Prepared::Text {
            text: "Streak".into(),
            role: Role::Unknown
        }
    );
    assert_eq!(
        card.lines[1],
        Prepared::Badge {
            text: "Streak".into(),
            tone: Tone::Unknown
        }
    );
}
