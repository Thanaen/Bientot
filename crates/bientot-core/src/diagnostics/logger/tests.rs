use super::{dotnet_double, json_timestamp, redact};
use crate::diagnostics::json::JsonObject;

#[test]
fn doubles_are_written_as_dotnet_writes_them() {
    let cases = [
        (0.0, "0"),
        (-0.0, "-0"),
        (5.0, "5"),
        (1.5, "1.5"),
        (0.1 + 0.2, "0.30000000000000004"),
        (0.0001, "0.0001"),
        (0.000_012_5, "1.25E-05"),
        (123_456_789_012_345.0, "123456789012345"),
        (1e15, "1000000000000000"),
        (1e16, "10000000000000000"),
        (1.5e16, "15000000000000000"),
        (1e17, "1E+17"),
        (1e-5, "1E-05"),
        (12_345_678_901_234_567.0, "12345678901234568"),
        (1.25e17, "1.25E+17"),
        (123_456_789_012_345_680.0, "1.2345678901234568E+17"),
        (-2.5e-300, "-2.5E-300"),
        (f64::MAX, "1.7976931348623157E+308"),
    ];
    for (value, expected) in cases {
        assert_eq!(dotnet_double(value), expected, "{value:e}");
    }
}

#[test]
fn credentials_are_removed_wherever_dotnet_removes_them() {
    let cases = [
        ("Bearer abc", "Bearer [REDACTED]"),
        ("\u{e9}Bearer Bearer tok", "\u{e9}Bearer Bearer [REDACTED]"),
        ("\u{903}Bearer tok", "\u{903}Bearer [REDACTED]"),
        ("\u{200d}Bearer tok", "\u{200d}Bearer tok"),
        ("xBearer abc", "xBearer abc"),
        ("_Bearer abc", "_Bearer abc"),
        ("(Bearer abc) next", "(Bearer [REDACTED]) next"),
        ("Bearer\u{3000}a.b~c+d/e=f-g!rest", "Bearer [REDACTED]!rest"),
        ("Bearer ", "Bearer "),
        ("Bearer !", "Bearer !"),
        ("password = s3cr3t, next", "password[REDACTED], next"),
        ("PASSWORD:", "PASSWORD:"),
        (
            "apikey=1 api-key=2 api_key=3 api--key=4",
            "apikey[REDACTED] api-key[REDACTED] api_key[REDACTED] api--key=4",
        ),
        ("access_token=abc", "access_token=abc"),
        ("access-token=abc", "access-token[REDACTED]"),
        ("tokens=abc", "tokens=abc"),
        ("token: : x", "token[REDACTED] x"),
        ("secret=a&b;c,d e", "secret[REDACTED]&b;c,d e"),
        (
            "Authorization: Bearer token=abc",
            "Authorization: Bearer [REDACTED]",
        ),
        // `(?i)` folds the Kelvin sign to `k`, and no other non-ASCII letter.
        ("api\u{212A}ey=1", "api\u{212A}ey[REDACTED]"),
        ("Bearer \u{212A}", "Bearer [REDACTED]"),
        (
            "\u{212A}assword=1 \u{17f}ecret=2",
            "\u{212A}assword=1 \u{17f}ecret=2",
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(redact(text), expected, "{text}");
    }
}

#[test]
fn long_text_is_cut_at_2048_utf16_units() {
    assert_eq!(redact(&"a".repeat(2_048)), "a".repeat(2_048));
    assert_eq!(redact(&"a".repeat(2_049)), "a".repeat(2_048) + "…");
    assert_eq!(
        redact(&("a".repeat(2_047) + "😀")),
        "a".repeat(2_047) + "\u{FFFD}…"
    );
    assert_eq!(
        redact(&("a".repeat(2_046) + "😀")),
        "a".repeat(2_046) + "😀"
    );
    assert_eq!(redact(&"é".repeat(2_049)), "é".repeat(2_048) + "…");
}

#[test]
fn text_and_timestamps_are_written_as_dotnet_writes_them() {
    let text = "a\"b\\c&'+<>`\u{e9}\u{2028}\n\t\u{1}\u{1f600}";
    assert_eq!(
        JsonObject::write(|object| object.string("T", text)),
        r#"{"T":"a\u0022b\\c\u0026\u0027\u002B\u003C\u003E\u0060\u00E9\u2028\n\t\u0001\uD83D\uDE00"}"#
    );
    let noon: jiff::Timestamp = "2026-10-13T07:00:00Z".parse().unwrap();
    let cases = [
        (0, "2026-10-13T07:00:00+00:00"),
        (123_450_000, "2026-10-13T07:00:00.12345+00:00"),
        (100, "2026-10-13T07:00:00.0000001+00:00"),
    ];
    for (nanoseconds, expected) in cases {
        let now = noon + jiff::SignedDuration::from_nanos(nanoseconds);
        assert_eq!(json_timestamp(now), expected);
    }
}
