//! How the week and « Corriger le temps… » write clock times, ranges, durations and dates.

use bientot_domain::intervals::UtcInterval;
use jiff::SignedDuration;
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;

use crate::strings::{Key, Language, keep_together};
use crate::workday::flyout_duration;

/// The midnight that closes `date`: 00:00 of the next day, which a range ending there writes
/// 24:00 and where Fin typed on 00:00 ends.
pub fn day_close(date: Date) -> Option<DateTime> {
    date.tomorrow()
        .ok()
        .map(|next| next.to_datetime(Time::midnight()))
}

/// `10:20–11:05`, `09:40:12–09:45:42`, as the week and « Corriger le temps… » write a range:
/// seconds on an end that has some, and 24:00 on an end at the midnight closing its start's day,
/// so a whole day reads `00:00–24:00` rather than an empty `00:00–00:00`.
pub fn range_text(start: DateTime, end: DateTime, language: Language) -> String {
    let [start, end] = range_ends(start, end);
    language.format(Key::HistoryIntervalFormat, &[&start, &end])
}

/// The two ends of a range as `range_text` writes them, for a sentence that names each.
pub fn range_ends(start: DateTime, end: DateTime) -> [String; 2] {
    let end = if day_close(start.date()) == Some(end) {
        "24:00".to_owned()
    } else {
        clock_time(end)
    };
    [clock_time(start), end]
}

/// `10:20–11:05` and its length, both to the nearest minute: what a slot to settle and the
/// range selected on the week show, so that their lengths add up. A slot under a minute keeps
/// its seconds rather than reading `09:40–09:40`.
pub fn minute_range(start: DateTime, end: DateTime, language: Language) -> (String, String) {
    let on_minute = |local: DateTime| local.round(jiff::Unit::Minute).unwrap_or(local);
    let (rounded_start, rounded_end) = (on_minute(start), on_minute(end));
    if rounded_start == rounded_end {
        return (
            range_text(start, end, language),
            duration_text(end.duration_since(start), language),
        );
    }
    let (start, end) = (rounded_start, rounded_end);
    (
        range_text(start, end, language),
        minutes_text(end.duration_since(start), language),
    )
}

/// `interval` in local times of `zone`, to the second: `09:40:12–09:45:42`.
pub fn local_range_text(interval: UtcInterval, zone: &TimeZone, language: Language) -> String {
    let [start, end] =
        [interval.start(), interval.end()].map(|instant| instant.to_zoned(zone.clone()).datetime());
    range_text(start, end, language)
}

fn clock_time(local: DateTime) -> String {
    let pattern = if local.second() == 0 && local.subsec_nanosecond() == 0 {
        "%H:%M"
    } else {
        "%H:%M:%S"
    };
    local.time().strftime(pattern).to_string()
}

/// A slot's length: hours and minutes, or minutes under an hour; seconds only when the
/// duration has some, as a slot whose bounds have seconds does.
pub fn duration_text(duration: SignedDuration, language: Language) -> String {
    if is_subsecond(duration) {
        return language.text(Key::HistoryDurationSubsecond).to_owned();
    }
    let seconds = whole_seconds(duration);
    match (seconds / 3600, seconds % 3600 / 60, seconds % 60) {
        (_, _, 0) => flyout_duration(SignedDuration::from_secs(seconds), language).to_string(),
        (0, minutes, seconds) => keep_together(
            &language.format(Key::HistoryDurationMinutesSeconds, &[&minutes, &seconds]),
        ),
        (hours, minutes, seconds) => keep_together(&language.format(
            Key::HistoryDurationHoursMinutesSeconds,
            &[&hours, &minutes, &seconds],
        )),
    }
}

/// The same duration as a screen reader reads it, in words: « 9 minutes 50 secondes », where
/// « 9m 50s » would be read as letters.
pub fn spoken_duration(duration: SignedDuration, language: Language) -> String {
    if is_subsecond(duration) {
        return language.text(Key::HistoryDurationSubsecond).to_owned();
    }
    let seconds = whole_seconds(duration);
    let parts = [
        (seconds / 3600, Key::SpokenHour, Key::SpokenHours),
        (seconds % 3600 / 60, Key::SpokenMinute, Key::SpokenMinutes),
        (seconds % 60, Key::SpokenSecond, Key::SpokenSeconds),
    ];
    let spoken: Vec<String> = parts
        .into_iter()
        .filter(|&(value, ..)| value != 0)
        .map(|(value, one, many)| language.format(if value == 1 { one } else { many }, &[&value]))
        .collect();
    if spoken.is_empty() {
        language.format(Key::SpokenSeconds, &[&0])
    } else {
        spoken.join(" ")
    }
}

fn is_subsecond(duration: SignedDuration) -> bool {
    !duration.is_zero() && duration.abs() < SignedDuration::from_secs(1)
}

/// To the nearest second, as a slot's length is shown.
fn whole_seconds(duration: SignedDuration) -> i64 {
    duration.abs().as_secs() + i64::from(duration.subsec_millis().abs() >= 500)
}

/// A total or a balance, to the nearest minute: « 6 h 25 min », « 45 min ».
pub fn minutes_text(duration: SignedDuration, language: Language) -> String {
    flyout_duration(nearest_minute(duration), language).to_string()
}

fn nearest_minute(duration: SignedDuration) -> SignedDuration {
    SignedDuration::from_mins((duration.as_millis() + 30_000).div_euclid(60_000) as i64)
}

/// « Mardi 6 octobre ».
pub fn long_date(date: Date, language: Language) -> String {
    capitalize(&language.format(
        Key::HistoryDayLongFormat,
        &[
            &language.weekday_name(date.weekday()),
            &date.day(),
            &language.month_name(date.month()),
        ],
    ))
}

fn capitalize(text: &str) -> String {
    let mut characters = text.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    fn at(day: i8, hour: i8, minute: i8, second: i8) -> DateTime {
        date(2026, 9, day).at(hour, minute, second, 0)
    }

    #[test]
    fn the_day_closes_at_the_next_midnight() {
        for (day, close) in [
            (date(2026, 9, 2), Some("2026-09-03T00:00:00")),
            (date(2026, 9, 30), Some("2026-10-01T00:00:00")),
            (date(2026, 12, 31), Some("2027-01-01T00:00:00")),
            (Date::MAX, None),
        ] {
            assert_eq!(
                day_close(day).map(|close| close.to_string()).as_deref(),
                close,
                "{day}"
            );
        }
    }

    #[test]
    fn ranges_show_seconds_when_a_bound_has_some_and_24_00_at_the_day_close() {
        for (start, end, text) in [
            (at(2, 10, 20, 0), at(2, 11, 5, 0), "10:20–11:05"),
            (at(2, 9, 40, 12), at(2, 9, 45, 42), "09:40:12–09:45:42"),
            (at(2, 9, 40, 0), at(2, 9, 45, 42), "09:40–09:45:42"),
            (at(2, 9, 40, 12), at(2, 9, 45, 0), "09:40:12–09:45"),
            (at(2, 0, 0, 0), at(3, 0, 0, 0), "00:00–24:00"),
            (at(2, 22, 0, 0), at(3, 0, 0, 0), "22:00–24:00"),
            (at(2, 23, 59, 30), at(3, 0, 0, 0), "23:59:30–24:00"),
            (at(2, 22, 0, 0), at(3, 0, 0, 1), "22:00–00:00:01"),
            (at(2, 22, 0, 0), at(3, 7, 0, 0), "22:00–07:00"),
            (at(2, 0, 0, 0), at(2, 9, 0, 0), "00:00–09:00"),
            (at(2, 0, 0, 0), at(2, 0, 0, 0), "00:00–00:00"),
        ] {
            for language in [Language::French, Language::English] {
                assert_eq!(range_text(start, end, language), text, "{start} {end}");
            }
        }
    }

    #[test]
    fn a_minute_range_rounds_its_ends_and_its_length_adds_up() {
        let french = Language::French;
        for (start, end, text, length) in [
            (
                at(2, 9, 40, 12),
                at(2, 9, 45, 42),
                "09:40–09:46",
                "6\u{a0}min",
            ),
            (
                at(2, 9, 40, 30),
                at(2, 9, 45, 29),
                "09:41–09:45",
                "4\u{a0}min",
            ),
            (
                at(2, 23, 59, 30),
                at(3, 0, 0, 0),
                "23:59:30–24:00",
                "0\u{a0}min\u{a0}30\u{a0}s",
            ),
            (
                at(2, 22, 0, 10),
                at(3, 0, 0, 0),
                "22:00–24:00",
                "2\u{a0}h\u{a0}00\u{a0}min",
            ),
        ] {
            let (range, duration) = minute_range(start, end, french);
            assert_eq!(
                (range.as_str(), duration.as_str()),
                (text, length),
                "{start} {end}"
            );
        }
    }

    #[test]
    fn slot_durations_show_seconds_only_when_they_have_some() {
        let french = Language::French;
        let text = |seconds| duration_text(SignedDuration::from_secs(seconds), french);
        assert_eq!(text(330), "5\u{a0}min\u{a0}30\u{a0}s");
        assert_eq!(text(3930), "1\u{a0}h\u{a0}05\u{a0}min\u{a0}30\u{a0}s");
        assert_eq!(text(45 * 60), "45\u{a0}min");
        assert_eq!(text(150 * 60), "2\u{a0}h\u{a0}30\u{a0}min");
        assert_eq!(
            duration_text(SignedDuration::from_secs(330), Language::English),
            "5m\u{a0}30s"
        );
        assert_eq!(
            duration_text(SignedDuration::from_millis(400), french),
            "Moins d’une seconde"
        );
        assert_eq!(
            duration_text(SignedDuration::from_millis(400), Language::English),
            "Less than one second"
        );
    }

    #[test]
    fn spoken_durations_round_as_the_shown_ones_and_say_every_word() {
        let almost = SignedDuration::from_millis(9 * 60_000 + 49_600);
        assert_eq!(
            duration_text(almost, Language::French),
            "9\u{a0}min\u{a0}50\u{a0}s"
        );
        assert_eq!(
            spoken_duration(almost, Language::French),
            "9 minutes 50 secondes"
        );
        assert_eq!(
            spoken_duration(almost, Language::English),
            "9 minutes 50 seconds"
        );
        let spoken =
            |seconds| spoken_duration(SignedDuration::from_secs(seconds), Language::English);
        assert_eq!(spoken(3601), "1 hour 1 second");
        assert_eq!(spoken(7260), "2 hours 1 minute");
        assert_eq!(spoken(0), "0 seconds");
    }

    #[test]
    fn totals_and_balances_round_to_the_minute() {
        let minutes = |seconds| minutes_text(SignedDuration::from_secs(seconds), Language::French);
        assert_eq!(minutes(329), "5\u{a0}min");
        assert_eq!(minutes(330), "6\u{a0}min");
        assert_eq!(minutes(29), "0\u{a0}min");
        assert_eq!(minutes(3570), "1\u{a0}h\u{a0}00\u{a0}min");
    }

    #[test]
    fn long_dates_name_the_weekday_capitalized_and_the_month() {
        let monday = date(2026, 8, 31);
        assert_eq!(long_date(monday, Language::French), "Lundi 31\u{a0}août");
        assert_eq!(
            long_date(monday, Language::English),
            "Monday, August\u{a0}31"
        );
    }
}
