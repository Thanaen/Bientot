use jiff::civil::date;
use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;
use serde_json::Value;

use super::*;

fn cases(fixture: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(fixture)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn text(case: &Value, field: &str) -> Option<String> {
    case[field].as_str().map(str::to_owned)
}

/// `tests/fixtures/instants.json` holds what `PersistedUtcInstantParser`, the session event reader
/// and the credit ledger reader parse from each stored text, written back with `ToString("O")` in
/// UTC.
#[test]
fn instants_parse_like_dotnet() {
    for case in cases(include_str!("../../tests/fixtures/instants.json")) {
        let input = case["input"].as_str().unwrap();
        let expected = |field| text(&case, field).filter(|_| !beyond_timestamp_range(input));
        let strict = parse_instant(input).ok().map(format_instant);
        let loose = parse_instant_loose(input).ok().map(format_instant);
        let lenient = parse_instant_lenient(input).ok().map(format_instant);
        assert_eq!(strict, expected("strict"), "strict {input:?}");
        assert_eq!(loose, expected("loose"), "loose {input:?}");
        assert_eq!(lenient, expected("lenient"), "lenient {input:?}");
    }
}

/// A jiff `Timestamp` ends at 9999-12-30T22:00:00.999999999Z, so the last 26 hours a .NET
/// `DateTimeOffset` holds are rejected.
pub(crate) fn beyond_timestamp_range(text: &str) -> bool {
    text.contains("9999-12-31T")
}

#[test]
fn the_last_instant_jiff_holds_round_trips() {
    let text = "9999-12-30T22:00:00.9999999+00:00";
    assert_eq!(parse_instant(text).map(format_instant), Ok(text.to_owned()));
    assert!(parse_instant("9999-12-30T22:00:01.0000000+00:00").is_err());
}

#[test]
fn dates_parse_like_dotnet() {
    for case in cases(include_str!("../../tests/fixtures/dates.json")) {
        let input = case["input"].as_str().unwrap();
        let parsed = parse_date(input).ok().map(format_date);
        assert_eq!(parsed, text(&case, "date"), "{input:?}");
    }
}

#[test]
fn an_instant_is_written_with_seven_fraction_digits_and_a_zero_offset() {
    let instant: Timestamp = "2026-08-30T07:00:00.123456789Z".parse().unwrap();
    assert_eq!(format_instant(instant), "2026-08-30T07:00:00.1234567+00:00");
    let before_epoch: Timestamp = "1969-12-31T23:59:59.99999999Z".parse().unwrap();
    assert_eq!(
        format_instant(before_epoch),
        "1969-12-31T23:59:59.9999999+00:00"
    );
}

#[test]
fn a_rejected_instant_names_the_text() {
    let error = parse_instant("2026-08-30T07:00:00Z").unwrap_err();
    assert_eq!(
        error.to_string(),
        "'2026-08-30T07:00:00Z' is not a .NET round-trip UTC instant"
    );
}

#[test]
fn dates_are_written_with_four_year_digits() {
    assert_eq!(format_date(date(26, 8, 3)), "0026-08-03");
}

#[test]
fn ticks_truncate_toward_zero_and_round_trip() {
    assert_eq!(to_ticks(SignedDuration::from_nanos(-199)), Ok(-1));
    assert_eq!(to_ticks(SignedDuration::from_mins(1)), Ok(600_000_000));
    assert_eq!(from_ticks(-15), SignedDuration::from_nanos(-1_500));
    assert_eq!(
        from_ticks(i64::MIN),
        SignedDuration::from_nanos_i128(i128::from(i64::MIN) * 100)
    );
    assert_eq!(
        to_ticks(SignedDuration::MAX),
        Err(DurationOutOfRange {
            duration: SignedDuration::MAX
        })
    );
}

proptest! {
    #[test]
    fn every_dotnet_instant_round_trips_through_the_strict_parser(
        ticks in 0i64..3_155_378_040_010_000_000,
    ) {
        // Ticks since 0001-01-01, up to the last instant jiff holds.
        let origin: Timestamp = "0001-01-01T00:00:00Z".parse().unwrap();
        let instant = origin + from_ticks(ticks);
        prop_assert_eq!(parse_instant(&format_instant(instant)), Ok(instant));
    }

    #[test]
    fn every_duration_in_ticks_round_trips(ticks: i64) {
        prop_assert_eq!(to_ticks(from_ticks(ticks)), Ok(ticks));
    }
}

/// Expected values come from .NET 10 `DateTimeOffset.ParseExact("O")` and System.Text.Json run
/// with `TZ` set to each zone; both pick the standard-time offset in a gap or a fold.
#[test]
fn an_offset_free_time_skipped_or_repeated_by_a_clock_change_takes_standard_time() {
    for (zone, local, utc) in [
        (
            "America/New_York",
            "2024-11-03T01:30",
            "2024-11-03T06:30:00Z",
        ),
        (
            "America/New_York",
            "2024-03-10T02:30",
            "2024-03-10T07:30:00Z",
        ),
        (
            "Australia/Sydney",
            "2024-04-07T02:30",
            "2024-04-06T16:30:00Z",
        ),
        (
            "Australia/Sydney",
            "2024-10-06T02:30",
            "2024-10-05T16:30:00Z",
        ),
        ("Europe/Dublin", "2024-10-27T01:30", "2024-10-27T01:30:00Z"),
        ("Europe/Dublin", "2024-03-31T01:30", "2024-03-31T01:30:00Z"),
        ("Europe/Paris", "2024-07-01T12:00", "2024-07-01T10:00:00Z"),
    ] {
        let zone_id = zone;
        let zone = TimeZone::get(zone).unwrap();
        let instant = local_instant(&zone, local.parse().unwrap());
        assert_eq!(instant, Some(utc.parse().unwrap()), "{zone_id} {local}");
    }
}
