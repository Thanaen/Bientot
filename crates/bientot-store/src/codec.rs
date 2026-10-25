//! The text and integer encodings the .NET ledger stores: instants, dates and durations.
//!
//! A jiff `Timestamp` ends at 9999-12-30T22:00:00.999999999Z, 26 hours before a .NET
//! `DateTimeOffset`, so the parsers reject the instants .NET accepts in between.

use std::fmt::Write as _;

use jiff::civil::{Date, DateTime};
use jiff::tz::{AmbiguousOffset, Offset, TimeZone};
use jiff::{SignedDuration, Timestamp};

const NANOS_PER_TICK: u32 = 100;
const TICKS_PER_SECOND: i64 = 10_000_000;

/// `DateTimeOffset.ToString("O")` of an instant in UTC, `yyyy-MM-ddTHH:mm:ss.fffffff+00:00`.
/// The text compares in time order, which the ledger queries rely on. .NET keeps 100 ns ticks, so
/// a finer instant is truncated.
pub fn format_instant(instant: Timestamp) -> String {
    let utc = TimeZone::UTC.to_datetime(instant);
    format!(
        "{}.{:07}+00:00",
        utc.strftime("%Y-%m-%dT%H:%M:%S"),
        utc.subsec_nanosecond() / NANOS_PER_TICK as i32
    )
}

/// The instant [`format_instant`] stores: `instant` truncated to a 100 ns tick.
pub fn truncate_to_tick(instant: Timestamp) -> Timestamp {
    let nanoseconds = instant.as_nanosecond();
    Timestamp::from_nanosecond(nanoseconds - nanoseconds.rem_euclid(i128::from(NANOS_PER_TICK)))
        .expect("a truncated instant stays within the range of its tick")
}

/// Reads an instant written by [`format_instant`], and nothing else: the .NET
/// `PersistedUtcInstantParser`, which rejects any text that would not be written back unchanged.
pub fn parse_instant(text: &str) -> Result<Timestamp, InvalidInstant> {
    parse_instant_loose(text)
        .ok()
        .filter(|&instant| format_instant(instant) == text)
        .ok_or_else(|| InvalidInstant::new(text))
}

/// Reads an instant in the .NET round-trip format with any offset, as `DateTimeOffset.ParseExact`
/// with `"O"` does for the session event, manual adjustment and long-lock review columns:
/// `yyyy-MM-ddTHH:mm:ss.fffffff` followed by `Z`, `+h:mm` or `+hh:mm` (at most 14 hours), or by
/// nothing for the local time of this machine.
pub fn parse_instant_loose(text: &str) -> Result<Timestamp, InvalidInstant> {
    round_trip_instant(text).ok_or_else(|| InvalidInstant::new(text))
}

fn round_trip_instant(text: &str) -> Option<Timestamp> {
    let mut cursor = Cursor::new(text);
    let date = cursor.date()?;
    let hour = cursor.byte(b'T')?.number(2)?;
    let minute = cursor.byte(b':')?.number(2)?;
    let second = cursor.byte(b':')?.number(2)?;
    let ticks = cursor.byte(b'.')?.number(7)?;
    let datetime = civil_datetime(date, hour, minute, second, ticks * NANOS_PER_TICK)?;
    let offset_minutes = match cursor.rest() {
        b"" => None,
        b"Z" => Some(0),
        [sign @ (b'+' | b'-'), offset @ ..] => {
            let (hours, minutes) = offset.split_at(offset.iter().position(|&byte| byte == b':')?);
            if !(1..=2).contains(&hours.len()) || minutes.len() != 3 {
                return None;
            }
            // .NET accepts minutes past 59 here, such as +00:99, within the 14 hour limit.
            let minutes = i32::try_from(digits(hours)? * 60 + digits(&minutes[1..])?).ok()?;
            if minutes > 14 * 60 {
                return None;
            }
            Some(if *sign == b'-' { -minutes } else { minutes })
        }
        _ => return None,
    };
    dotnet_instant(datetime, offset_minutes)
}

/// Reads an instant as `DateTimeOffset.Parse(text, InvariantCulture, RoundtripKind)` does for the
/// credit ledger's `CreatedAtUtc`, limited to its ISO 8601 shapes: `yyyy-M-d`, then optionally a
/// `T` or white space and `HH:mm[:ss[.fraction]]`, then optionally `Z` or an offset of at most
/// 14 hours (`+h`, `+hh`, `+hmm`, `+hhmm` or `+h:m` with one or two digits each). White space
/// around the text is ignored, and an instant without an offset is local time on this machine.
/// .NET also reads the culture formats (`08/30/2026`, `Sun, 30 Aug 2026 ...`), which this rejects.
pub fn parse_instant_lenient(text: &str) -> Result<Timestamp, InvalidInstant> {
    lenient_instant(text.trim()).ok_or_else(|| InvalidInstant::new(text))
}

fn lenient_instant(text: &str) -> Option<Timestamp> {
    let mut cursor = Cursor::new(text);
    let year = cursor.number(4)?;
    let month = digits(cursor.byte(b'-')?.leading_digits(2))?;
    let day = digits(cursor.byte(b'-')?.leading_digits(2))?;
    let date = Date::new(
        i16::try_from(year).ok().filter(|&year| year >= 1)?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()?;

    let date_end = cursor.rest();
    let separated = cursor.skip_white_space();
    let designated = cursor.byte(b'T').is_some() || cursor.byte(b't').is_some();
    if designated {
        cursor.skip_white_space();
    }
    let (time, ticks) = if (separated || designated) && cursor.rest().first()?.is_ascii_digit() {
        let hour = cursor.number(2)?;
        let minute = cursor.byte(b':')?.number(2)?;
        let (second, ticks) = match cursor.byte(b':') {
            None => (0, 0),
            Some(cursor) => {
                let second = cursor.number(2)?;
                let ticks = if cursor.byte(b'.').is_some() || cursor.byte(b',').is_some() {
                    rounded_fraction_ticks(cursor.leading_digits(usize::MAX))?
                } else {
                    0
                };
                (second, ticks)
            }
        };
        (civil_datetime(date, hour, minute, second, 0)?, ticks)
    } else if designated {
        return None;
    } else {
        cursor = Cursor { bytes: date_end };
        (date.to_datetime(jiff::civil::Time::midnight()), 0)
    };
    let datetime = time
        .checked_add(SignedDuration::from_nanos(
            ticks * i64::from(NANOS_PER_TICK),
        ))
        .ok()?;

    cursor.skip_white_space();
    let offset_minutes = match cursor.rest() {
        b"" => None,
        b"Z" | b"z" => Some(0),
        [sign @ (b'+' | b'-'), offset @ ..] => {
            let (hours, minutes) = match offset.iter().position(|&byte| byte == b':') {
                Some(colon) => {
                    let (hours, minutes) = (&offset[..colon], &offset[colon + 1..]);
                    if !(1..=2).contains(&hours.len()) || !(1..=2).contains(&minutes.len()) {
                        return None;
                    }
                    (digits(hours)?, digits(minutes)?)
                }
                None => match offset.len() {
                    1 | 2 => (digits(offset)?, 0),
                    3 | 4 => {
                        let (hours, minutes) = offset.split_at(offset.len() - 2);
                        (digits(hours)?, digits(minutes)?)
                    }
                    _ => return None,
                },
            };
            let total = i32::try_from(hours * 60 + minutes).ok()?;
            if minutes > 59 || total > 14 * 60 {
                return None;
            }
            Some(if *sign == b'-' { -total } else { total })
        }
        _ => return None,
    };
    dotnet_instant(datetime, offset_minutes)
}

/// The ticks of a decimal fraction of a second, as .NET computes them: the digits summed into a
/// `double`, then scaled and rounded half to even.
fn rounded_fraction_ticks(fraction: &[u8]) -> Option<i64> {
    if fraction.is_empty() {
        return None;
    }
    let mut value = 0.0f64;
    let mut weight = 0.1f64;
    for &digit in fraction {
        value += f64::from(digit - b'0') * weight;
        weight *= 0.1;
    }
    Some((value * TICKS_PER_SECOND as f64).round_ties_even() as i64)
}

/// `yyyy-MM-dd`, the workday dates of the ledger.
pub fn format_date(date: Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day())
}

/// Reads a date written by [`format_date`], as `DateOnly.ParseExact(text, "yyyy-MM-dd")` does.
pub fn parse_date(text: &str) -> Result<Date, InvalidDate> {
    let mut cursor = Cursor::new(text);
    cursor
        .date()
        .filter(|_| cursor.rest().is_empty())
        .ok_or_else(|| InvalidDate {
            text: text.to_owned(),
        })
}

/// A duration in .NET `TimeSpan` ticks of 100 ns, truncated toward zero.
pub fn to_ticks(duration: SignedDuration) -> Result<i64, DurationOutOfRange> {
    i64::try_from(duration.as_nanos() / i128::from(NANOS_PER_TICK))
        .map_err(|_| DurationOutOfRange { duration })
}

/// The duration of a number of .NET `TimeSpan` ticks.
pub fn from_ticks(ticks: i64) -> SignedDuration {
    SignedDuration::new(
        ticks / TICKS_PER_SECOND,
        (ticks % TICKS_PER_SECOND) as i32 * NANOS_PER_TICK as i32,
    )
}

/// Appends `value` as a JSON string escaped the way `Utf8JsonWriter` and its default
/// `JavaScriptEncoder` escape it: everything outside printable ASCII, and the HTML-sensitive `"&'+<>` and backtick, as `\uXXXX` with upper-case digits (UTF-16
/// surrogate pairs beyond the BMP); the backslash and five control characters use short escapes.
pub fn write_json_string(json: &mut String, value: &str) {
    json.push('"');
    for character in value.chars() {
        match character {
            '\\' => json.push_str("\\\\"),
            '\n' => json.push_str("\\n"),
            '\r' => json.push_str("\\r"),
            '\t' => json.push_str("\\t"),
            '\u{8}' => json.push_str("\\b"),
            '\u{c}' => json.push_str("\\f"),
            ' ' | '!' | '#'..='%' | '('..='*' | ','..=';' | '=' | '?'..='_' | 'a'..='~' => {
                json.push(character);
            }
            _ => {
                let mut units = [0u16; 2];
                for unit in character.encode_utf16(&mut units) {
                    write!(json, "\\u{unit:04X}").expect("writing to a String cannot fail");
                }
            }
        }
    }
    json.push('"');
}

/// The value of a run of ASCII digits; `None` when empty, too large, or not all digits.
pub(crate) fn digits(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    bytes.iter().try_fold(0u32, |value, &byte| {
        let digit = byte.is_ascii_digit().then(|| u32::from(byte - b'0'))?;
        value.checked_mul(10)?.checked_add(digit)
    })
}

/// Reads fixed-width fields from the front of a text.
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
        }
    }

    pub(crate) fn rest(&self) -> &'a [u8] {
        self.bytes
    }

    /// Takes `expected` when it comes next.
    pub(crate) fn byte(&mut self, expected: u8) -> Option<&mut Self> {
        let (&first, rest) = self.bytes.split_first()?;
        (first == expected).then(|| {
            self.bytes = rest;
            self
        })
    }

    /// Takes the value of the next `width` ASCII digits.
    pub(crate) fn number(&mut self, width: usize) -> Option<u32> {
        let (field, rest) = self.bytes.split_at_checked(width)?;
        let value = digits(field)?;
        self.bytes = rest;
        Some(value)
    }

    /// Takes any white space that comes next, and tells whether there was some.
    pub(crate) fn skip_white_space(&mut self) -> bool {
        let text = std::str::from_utf8(self.bytes)
            .expect("the cursor only takes ASCII, so it stops between characters");
        let rest = text.trim_start();
        self.bytes = rest.as_bytes();
        rest.len() != text.len()
    }

    /// Takes up to `limit` ASCII digits.
    pub(crate) fn leading_digits(&mut self, limit: usize) -> &'a [u8] {
        let count = self
            .bytes
            .iter()
            .take(limit)
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        let (digits, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        digits
    }

    /// Takes a `yyyy-MM-dd` date in the .NET range of years 1 to 9999.
    pub(crate) fn date(&mut self) -> Option<Date> {
        let year = self.number(4)?;
        let month = self.byte(b'-')?.number(2)?;
        let day = self.byte(b'-')?.number(2)?;
        let year = i16::try_from(year).ok().filter(|&year| year >= 1)?;
        Date::new(year, i8::try_from(month).ok()?, i8::try_from(day).ok()?).ok()
    }
}

/// A .NET `DateTime`, which has no leap second and no 24:00.
pub(crate) fn civil_datetime(
    date: Date,
    hour: u32,
    minute: u32,
    second: u32,
    nanosecond: u32,
) -> Option<DateTime> {
    let time = jiff::civil::Time::new(
        i8::try_from(hour).ok()?,
        i8::try_from(minute).ok()?,
        i8::try_from(second).ok()?,
        i32::try_from(nanosecond).ok()?,
    );
    Some(date.to_datetime(time.ok()?))
}

/// The instant of a local date and time `offset_minutes` ahead of UTC, or in the local time zone
/// of this machine when no offset is given, as `new DateTimeOffset(dateTime)` reads it. `None`
/// outside the range a `DateTimeOffset` can hold.
pub(crate) fn dotnet_instant(datetime: DateTime, offset_minutes: Option<i32>) -> Option<Timestamp> {
    let instant = match offset_minutes {
        Some(minutes) => Offset::from_seconds(minutes * 60)
            .ok()?
            .to_timestamp(datetime)
            .ok()?,
        None => local_instant(&TimeZone::system(), datetime)?,
    };
    let utc = TimeZone::UTC.to_datetime(instant);
    (1..=9999).contains(&utc.year()).then_some(instant)
}

/// The instant of a local date and time in `zone`. A time skipped or repeated by a clock change
/// takes the smaller of the two offsets, the standard time, as .NET does; jiff would take the
/// earlier instant.
fn local_instant(zone: &TimeZone, datetime: DateTime) -> Option<Timestamp> {
    let offset = match zone.to_ambiguous_timestamp(datetime).offset() {
        AmbiguousOffset::Unambiguous { offset } => offset,
        AmbiguousOffset::Gap { before, after } | AmbiguousOffset::Fold { before, after } => {
            before.min(after)
        }
    };
    offset.to_timestamp(datetime).ok()
}

/// Stored text that is not an instant in the expected .NET format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidInstant {
    pub text: String,
}

impl InvalidInstant {
    fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
        }
    }
}

impl std::fmt::Display for InvalidInstant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{}' is not a .NET round-trip UTC instant", self.text)
    }
}

impl std::error::Error for InvalidInstant {}

/// Stored text that is not a `yyyy-MM-dd` date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidDate {
    pub text: String,
}

impl std::fmt::Display for InvalidDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{}' is not a yyyy-MM-dd date", self.text)
    }
}

impl std::error::Error for InvalidDate {}

/// A duration longer than a .NET `TimeSpan` can hold, about 29,000 years.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DurationOutOfRange {
    pub duration: SignedDuration,
}

impl std::fmt::Display for DurationOutOfRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} does not fit in .NET ticks", self.duration)
    }
}

impl std::error::Error for DurationOutOfRange {}

#[cfg(test)]
pub(crate) mod tests;
