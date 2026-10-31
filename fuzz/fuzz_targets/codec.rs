//! The text formats of the ledger's dates and instants: no input panics, and what reads writes
//! back to text that reads as the same value.

#![no_main]

use bientot_store::codec::{
    format_date, format_instant, parse_date, parse_instant, parse_instant_lenient,
    parse_instant_loose,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    if let Ok(instant) = parse_instant(text) {
        assert_eq!(format_instant(instant), text);
    }
    if let Ok(instant) = parse_instant_loose(text) {
        assert_eq!(parse_instant(&format_instant(instant)).ok(), Some(instant));
    }
    if let Ok(instant) = parse_instant_lenient(text) {
        assert_eq!(
            parse_instant_loose(&format_instant(instant)).ok(),
            Some(instant)
        );
    }
    if let Ok(date) = parse_date(text) {
        assert_eq!(parse_date(&format_date(date)).ok(), Some(date));
    }
});
