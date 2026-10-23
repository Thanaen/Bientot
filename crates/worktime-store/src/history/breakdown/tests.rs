use serde_json::Value;

use super::*;

/// `tests/fixtures/breakdowns.json` holds what the .NET codec reads from each text, written back
/// by its serializer, or null when it rejects the text.
#[test]
fn breakdowns_read_and_write_like_dotnet() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../../tests/fixtures/breakdowns.json")).unwrap();
    assert!(cases.len() > 40);
    for case in cases {
        let input = case["input"].as_str().unwrap();
        let read = deserialize(input)
            .ok()
            .map(|breakdown| serialize(&breakdown).unwrap());
        assert_eq!(read.as_deref(), case["read"].as_str(), "{input:?}");
    }
}

#[test]
fn strings_are_escaped_like_the_default_dotnet_encoder() {
    let mut json = String::new();

    write_json_string(
        &mut json,
        "x\"y\\z<>&'+`\u{7f}\u{1}\t\n\r\u{8}\u{c} \u{e9} \u{1F600} \u{2028} \u{a0} \u{feff}/~",
    );

    assert_eq!(
        json,
        r#""x\u0022y\\z\u003C\u003E\u0026\u0027\u002B\u0060\u007F\u0001\t\n\r\b\f \u00E9 \uD83D\uDE00 \u2028 \u00A0 \uFEFF/~""#
    );
}

#[test]
fn an_empty_breakdown_is_written_compactly() {
    assert_eq!(
        serialize(&DailySummaryBreakdown::default()).unwrap(),
        r#"{"version":1,"entries":[]}"#
    );
}
