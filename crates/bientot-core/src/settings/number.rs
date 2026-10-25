//! What the settings window's fields accept, ported from `Services/SettingsNumberParser.cs`.

use jiff::SignedDuration;

/// Why a field's text is not a value, named after the `.resw` key that says so.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputError {
    TargetDurationRequired,
    TargetDurationOutOfRange,
    WholeNumberRequired,
    WholeSecondsRequired,
    NumberOutOfRange,
}

impl InputError {
    pub fn resource_key(self) -> &'static str {
        match self {
            Self::TargetDurationRequired => "SettingsTargetDurationRequired",
            Self::TargetDurationOutOfRange => "SettingsTargetDurationOutOfRange",
            Self::WholeNumberRequired => "SettingsWholeNumberRequired",
            Self::WholeSecondsRequired => "SettingsWholeSecondsRequired",
            Self::NumberOutOfRange => "SettingsNumberOutOfRange",
        }
    }
}

/// `H:mm` or `HH:mm` from 0:01 to 24:00, spaces around it ignored.
pub fn parse_target_duration(raw: &str) -> Result<SignedDuration, InputError> {
    let raw = raw.trim().as_bytes();
    let separator = raw.iter().position(|&byte| byte == b':');
    // A character beyond ASCII fails the digit checks, so counting bytes rejects what .NET's
    // UTF-16 lengths reject.
    let Some(separator) = separator.filter(|&separator| {
        (4..=5).contains(&raw.len())
            && (1..=2).contains(&separator)
            && separator == raw.len() - 3
            && raw[..separator].iter().all(u8::is_ascii_digit)
            && raw[separator + 1..].iter().all(u8::is_ascii_digit)
    }) else {
        return Err(InputError::TargetDurationRequired);
    };
    let number = |digits: &[u8]| {
        digits
            .iter()
            .fold(0_i64, |value, digit| value * 10 + i64::from(digit - b'0'))
    };
    let (hours, minutes) = (number(&raw[..separator]), number(&raw[separator + 1..]));
    if minutes > 59 || hours > 24 || hours == 24 && minutes != 0 || hours == 0 && minutes == 0 {
        return Err(InputError::TargetDurationOutOfRange);
    }
    Ok(SignedDuration::from_mins(hours * 60 + minutes))
}

/// « 7:24 »: whole hours, then two digits of minutes.
pub fn format_target_duration(value: SignedDuration) -> String {
    let seconds = value.as_secs();
    format!("{}:{:02}", seconds / 3600, seconds / 60 % 60)
}

/// An optional [`parse_target_duration`]: blank text is valid and means no value.
pub fn parse_optional_target_duration(raw: &str) -> Result<Option<SignedDuration>, InputError> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    parse_target_duration(raw).map(Some)
}

pub fn format_optional_target_duration(value: Option<SignedDuration>) -> String {
    value.map(format_target_duration).unwrap_or_default()
}

/// A whole number of minutes within `minimum..=maximum`.
pub fn parse_minutes(raw: &str, minimum: f64, maximum: f64) -> Result<SignedDuration, InputError> {
    let minutes = parse_whole(raw, minimum, maximum, InputError::WholeNumberRequired)?;
    Ok(SignedDuration::from_secs(minutes * 60))
}

/// A whole number of seconds within `minimum..=maximum`.
pub fn parse_seconds(raw: &str, minimum: f64, maximum: f64) -> Result<SignedDuration, InputError> {
    let seconds = parse_whole(raw, minimum, maximum, InputError::WholeSecondsRequired)?;
    Ok(SignedDuration::from_secs(seconds))
}

/// A number as `double.Parse` reads it with `NumberStyles.Float`, which must be whole and in
/// range. Only the invariant decimal point is read.
fn parse_whole(
    raw: &str,
    minimum: f64,
    maximum: f64,
    malformed: InputError,
) -> Result<i64, InputError> {
    let value = raw
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && value.trunc() == *value)
        .ok_or(malformed)?;
    if value < minimum || value > maximum {
        return Err(InputError::NumberOutOfRange);
    }
    Ok(value as i64)
}
