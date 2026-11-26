use jiff::Timestamp;
use jiff::tz::Offset;

use super::*;

fn offset(zone: &TimeZone, instant: &str) -> Offset {
    zone.to_offset(instant.parse::<Timestamp>().unwrap())
}

#[test]
fn every_windows_key_resolves_to_the_zone_of_its_iana_name() {
    for (key, iana) in windows::WINDOWS_ZONES {
        let zone = resolve(key).unwrap_or_else(|_| panic!("{key} ({iana}) does not resolve"));
        assert_eq!(Ok(zone), jiff::tz::db().get(iana).map_err(|_| ()), "{key}");
    }
}

#[test]
fn a_windows_key_resolves_to_the_zone_an_iana_name_gives() {
    let windows = resolve("Romance Standard Time").unwrap();
    let iana = resolve("Europe/Paris").unwrap();

    assert_eq!(windows, iana);
    assert_eq!(
        offset(&windows, "2026-01-15T12:00:00Z"),
        Offset::constant(1)
    );
    assert_eq!(
        offset(&windows, "2026-07-15T12:00:00Z"),
        Offset::constant(2)
    );
}

#[test]
fn keys_whose_cldr_zone_has_an_old_name_resolve() {
    let india = resolve("India Standard Time").unwrap();
    let ukraine = resolve("FLE Standard Time").unwrap();

    assert_eq!(
        offset(&india, "2026-07-15T12:00:00Z"),
        Offset::from_seconds(5 * 3600 + 1800).unwrap()
    );
    assert_eq!(
        offset(&ukraine, "2026-07-15T12:00:00Z"),
        Offset::constant(3)
    );
}

#[test]
fn a_name_of_neither_platform_is_unknown() {
    assert_eq!(
        resolve("Mars Standard Time").unwrap_err(),
        UnknownTimeZone("Mars Standard Time".into())
    );
}
