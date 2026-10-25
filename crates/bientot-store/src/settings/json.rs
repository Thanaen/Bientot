//! `settings.json` as System.Text.Json reads and writes it with the web defaults and indentation
//! the .NET store uses. Reading: property names match in any ASCII case, the last duplicate wins,
//! unknown properties are skipped, integers may be quoted, a missing property keeps its default
//! (`false` and zero inside a notification rule), and `null` is accepted only where the setting is
//! optional (.NET reads a `null` group and its validator rejects it; here the read rejects it).
//! Writing: camelCase names in declaration order, two-space indentation, `null` for
//! absent values, and the .NET escaping of HTML-sensitive and non-ASCII characters.
//!
//! `usagePing`, `recurringAppointments` and `appointmentProposals` are the members the .NET
//! settings lack. They are written after `updates`; .NET skips them on read.
//!
//! Unlike .NET, a write keeps the members of the file it replaces that these settings do not
//! know, after the known members of the same object, so that the settings of a newer WorkTime
//! survive an older one. Inside an array, such as `recurringAppointments`, they are not kept.

use std::borrow::Cow;
use std::fmt::Write as _;

use bientot_domain::configuration::{
    ActivityWatchSettings, ActivityWatchSettlementContract, AppointmentProposalSettings,
    BuiltInActivitySettings, DismissedProposal, LoggingSettings, NotificationRule,
    NotificationSettings, RecurringAppointment, Settings, UpdateSettings,
};
use bientot_domain::sources::ActivitySourceKind;
use jiff::civil::{Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde_json::value::RawValue;

use crate::codec::{Cursor, civil_datetime, digits, dotnet_instant, from_ticks};

const TICKS_PER_SECOND: u128 = 10_000_000;
const TICKS_PER_DAY: u128 = 86_400 * TICKS_PER_SECOND;

/// Reads settings the way `JsonSerializer.Deserialize<Settings>` does. The error names
/// what was rejected. Values are not validated here.
pub(crate) fn read(text: &str) -> Result<Settings, String> {
    let mut settings = Settings::default();
    for (name, value) in object(text)? {
        match name.to_ascii_lowercase().as_str() {
            "schemaversion" => settings.schema_version = integer(value)?,
            "nominaldailytarget" => settings.nominal_daily_target = duration(value)?,
            "minimumeffectivetarget" => settings.minimum_effective_target = duration(value)?,
            "lunchstart" => settings.lunch_start = time(value)?,
            "lunchend" => settings.lunch_end = time(value)?,
            "preferredlatestdeparture" => settings.preferred_latest_departure = time(value)?,
            "orangewarningmargin" => settings.orange_warning_margin = duration(value)?,
            "longlockreviewthreshold" => settings.long_lock_review_threshold = duration(value)?,
            "weeklycreditenabled" => settings.weekly_credit_enabled = boolean(value)?,
            "weeklydebtenabled" => settings.weekly_debt_enabled = boolean(value)?,
            "maximumweeklydebt" => settings.maximum_weekly_debt = optional(value, duration)?,
            "startwithwindows" => settings.start_with_windows = boolean(value)?,
            "notifications" => settings.notifications = notifications(value)?,
            "activitysource" => settings.activity_source = optional(value, activity_source)?,
            "builtinactivity" => settings.built_in_activity = built_in_activity(value)?,
            "activitywatch" => settings.activity_watch = activity_watch(value)?,
            "updates" => settings.updates = updates(value)?,
            "usageping" => settings.usage_ping = boolean(value)?,
            "recurringappointments" => {
                settings.recurring_appointments = array(value, recurring_appointment)?;
            }
            "appointmentproposals" => settings.appointment_proposals = proposals(value)?,
            "logging" => settings.logging = logging(value)?,
            "languageoverride" => settings.language_override = optional(value, string)?,
            _ => {}
        }
    }
    Ok(settings)
}

fn notifications(value: &RawValue) -> Result<NotificationSettings, String> {
    let mut notifications = NotificationSettings::default();
    for (name, value) in object(value.get())? {
        match name.to_ascii_lowercase().as_str() {
            "completionapproach" => notifications.completion_approach = rule(value)?,
            "latestdepartureapproach" => notifications.latest_departure_approach = rule(value)?,
            "daycomplete" => notifications.day_complete = rule(value)?,
            _ => {}
        }
    }
    Ok(notifications)
}

/// A positional record in .NET: a missing member takes the default of its type.
fn rule(value: &RawValue) -> Result<NotificationRule, String> {
    let mut rule = NotificationRule::new(false, SignedDuration::ZERO);
    for (name, value) in object(value.get())? {
        match name.to_ascii_lowercase().as_str() {
            "enabled" => rule.enabled = boolean(value)?,
            "leadtime" => rule.lead_time = duration(value)?,
            _ => {}
        }
    }
    Ok(rule)
}

fn built_in_activity(value: &RawValue) -> Result<BuiltInActivitySettings, String> {
    let mut built_in = BuiltInActivitySettings::default();
    for (name, value) in object(value.get())? {
        if name.eq_ignore_ascii_case("idleThreshold") {
            built_in.idle_threshold = duration(value)?;
        }
    }
    Ok(built_in)
}

fn activity_watch(value: &RawValue) -> Result<ActivityWatchSettings, String> {
    let mut source = ActivityWatchSettings::default();
    for (name, value) in object(value.get())? {
        match name.to_ascii_lowercase().as_str() {
            "baseurl" => source.base_url = string(value)?,
            "selectedbucketid" => source.selected_bucket_id = optional(value, string)?,
            "apikeyreference" => source.api_key_reference = optional(value, string)?,
            "requesttimeout" => source.request_timeout = duration(value)?,
            "settlement" => source.settlement = settlement(value)?,
            _ => {}
        }
    }
    Ok(source)
}

/// `horizon` is computed, so .NET writes it and skips it on read.
fn settlement(value: &RawValue) -> Result<ActivityWatchSettlementContract, String> {
    let mut settlement = ActivityWatchSettlementContract::default();
    for (name, value) in object(value.get())? {
        match name.to_ascii_lowercase().as_str() {
            "afktimeout" => settlement.afk_timeout = duration(value)?,
            "pollinterval" => settlement.poll_interval = duration(value)?,
            "maximumjitter" => settlement.maximum_jitter = duration(value)?,
            _ => {}
        }
    }
    Ok(settlement)
}

fn updates(value: &RawValue) -> Result<UpdateSettings, String> {
    let mut updates = UpdateSettings::default();
    for (name, value) in object(value.get())? {
        match name.to_ascii_lowercase().as_str() {
            "checkforupdates" => updates.check_for_updates = boolean(value)?,
            "lastcheckedatutc" => updates.last_checked_at = optional(value, instant)?,
            _ => {}
        }
    }
    Ok(updates)
}

fn recurring_appointment(value: &RawValue) -> Result<RecurringAppointment, String> {
    let mut appointment = RecurringAppointment {
        id: String::new(),
        name: String::new(),
        weekday: Weekday::Monday,
        start: Time::midnight(),
        end: Time::midnight(),
        enabled: true,
        since: None,
        alternate_weeks_from: None,
        automatic: false,
    };
    for (name, value) in object(value.get())? {
        match name.to_ascii_lowercase().as_str() {
            "id" => appointment.id = string(value)?,
            "name" => appointment.name = string(value)?,
            "weekday" => appointment.weekday = weekday(value)?,
            "start" => appointment.start = time(value)?,
            "end" => appointment.end = time(value)?,
            "enabled" => appointment.enabled = boolean(value)?,
            "since" => appointment.since = optional(value, civil_date)?,
            "alternateweeksfrom" => appointment.alternate_weeks_from = optional(value, civil_date)?,
            "automatic" => appointment.automatic = boolean(value)?,
            _ => {}
        }
    }
    Ok(appointment)
}

fn proposals(value: &RawValue) -> Result<AppointmentProposalSettings, String> {
    let mut proposals = AppointmentProposalSettings::default();
    for (name, value) in object(value.get())? {
        match name.to_ascii_lowercase().as_str() {
            "enabled" => proposals.enabled = boolean(value)?,
            "dismissed" => proposals.dismissed = array(value, dismissed_proposal)?,
            _ => {}
        }
    }
    Ok(proposals)
}

fn dismissed_proposal(value: &RawValue) -> Result<DismissedProposal, String> {
    let mut dismissed = DismissedProposal {
        weekday: Weekday::Monday,
        start: Time::midnight(),
        end: Time::midnight(),
        until: None,
    };
    for (name, value) in object(value.get())? {
        match name.to_ascii_lowercase().as_str() {
            "weekday" => dismissed.weekday = weekday(value)?,
            "start" => dismissed.start = time(value)?,
            "end" => dismissed.end = time(value)?,
            "until" => dismissed.until = optional(value, civil_date)?,
            _ => {}
        }
    }
    Ok(dismissed)
}

/// A `DateOnly`: `yyyy-MM-dd`.
fn civil_date(value: &RawValue) -> Result<jiff::civil::Date, String> {
    quoted(value, 10..=10)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| format!("{} is not a DateOnly", value.get()))
}

const WEEKDAYS: [(Weekday, &str); 7] = [
    (Weekday::Monday, "monday"),
    (Weekday::Tuesday, "tuesday"),
    (Weekday::Wednesday, "wednesday"),
    (Weekday::Thursday, "thursday"),
    (Weekday::Friday, "friday"),
    (Weekday::Saturday, "saturday"),
    (Weekday::Sunday, "sunday"),
];

fn weekday(value: &RawValue) -> Result<Weekday, String> {
    let name = string(value)?;
    WEEKDAYS
        .iter()
        .find(|(_, known)| known.eq_ignore_ascii_case(&name))
        .map(|&(weekday, _)| weekday)
        .ok_or_else(|| format!("weekday '{name}' is not recognized"))
}

fn array<T>(value: &RawValue, read: fn(&RawValue) -> Result<T, String>) -> Result<Vec<T>, String> {
    serde_json::from_str::<Vec<&RawValue>>(value.get())
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(read)
        .collect()
}

fn logging(value: &RawValue) -> Result<LoggingSettings, String> {
    let mut logging = LoggingSettings::default();
    for (name, value) in object(value.get())? {
        match name.to_ascii_lowercase().as_str() {
            "maximumfilecount" => logging.maximum_file_count = integer(value)?,
            "maximumfilebytes" => logging.maximum_file_bytes = integer(value)?,
            _ => {}
        }
    }
    Ok(logging)
}

/// The members of a JSON object in document order, duplicates included, with their raw values.
fn object(text: &str) -> Result<Vec<(String, &RawValue)>, String> {
    serde_json::from_str::<Members>(text)
        .map(|members| members.0)
        .map_err(|error| error.to_string())
}

struct Members<'a>(Vec<(String, &'a RawValue)>);

impl<'de> Deserialize<'de> for Members<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MembersVisitor;

        impl<'de> Visitor<'de> for MembersVisitor {
            type Value = Members<'de>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut members = Vec::new();
                while let Some(member) = map.next_entry()? {
                    members.push(member);
                }
                Ok(Members(members))
            }
        }

        deserializer.deserialize_map(MembersVisitor)
    }
}

fn optional<T>(
    value: &RawValue,
    read: fn(&RawValue) -> Result<T, String>,
) -> Result<Option<T>, String> {
    if value.get() == "null" {
        Ok(None)
    } else {
        read(value).map(Some)
    }
}

fn boolean(value: &RawValue) -> Result<bool, String> {
    match value.get() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("{other} is not a boolean")),
    }
}

fn string(value: &RawValue) -> Result<String, String> {
    if value.get().starts_with('"') {
        serde_json::from_str(value.get()).map_err(|error| error.to_string())
    } else {
        Err(format!("{} is not a string", value.get()))
    }
}

/// A JSON integer, or a string holding `[+-]digits`: the web defaults read numbers from strings.
/// A fraction or an exponent is rejected even when the value is whole.
fn integer<T: std::str::FromStr>(value: &RawValue) -> Result<T, String> {
    let raw = value.get();
    let text = if raw.starts_with('"') {
        string(value)?
    } else {
        raw.to_owned()
    };
    text.parse()
        .map_err(|_| format!("{raw} is not an integer in range"))
}

fn activity_source(value: &RawValue) -> Result<ActivitySourceKind, String> {
    match string(value)?.as_str() {
        "built-in" => Ok(ActivitySourceKind::BuiltIn),
        "activitywatch" => Ok(ActivitySourceKind::ActivityWatch),
        other => Err(format!("activity source '{other}' is not recognized")),
    }
}

/// A `TimeSpan`: a string in the constant format, `[-][d.]hh:mm:ss[.fffffff]`, which .NET also
/// reads with fewer components (`d`, `h:m`, `d.h:m`, `h:m:s`, `d.h:m:s`, `h:m:s.f`).
fn duration(value: &RawValue) -> Result<SignedDuration, String> {
    let ticks = quoted(value, 1..=156)
        .filter(|text| text.starts_with(|first: char| first.is_ascii_digit() || first == '-'))
        .and_then(|text| constant_time_span(&text));
    let ticks = ticks.ok_or_else(|| format!("{} is not a TimeSpan", value.get()))?;
    Ok(from_ticks(ticks))
}

/// A `TimeOnly`: `h:m`, `h:m:s` or `h:m:s.f` of the same constant format.
fn time(value: &RawValue) -> Result<Time, String> {
    let ticks = quoted(value, 3..=96)
        .filter(|text| {
            text.starts_with(|first: char| first.is_ascii_digit())
                && text
                    .find(['.', ':'])
                    .is_some_and(|at| text.as_bytes()[at] == b':')
        })
        .and_then(|text| constant_time_span(&text))
        .and_then(|ticks| u128::try_from(ticks).ok());
    let time = ticks.and_then(|ticks| {
        let (seconds, fraction) = (ticks / TICKS_PER_SECOND, ticks % TICKS_PER_SECOND);
        Time::new(
            i8::try_from(seconds / 3600).ok()?,
            (seconds / 60 % 60) as i8,
            (seconds % 60) as i8,
            fraction as i32 * 100,
        )
        .ok()
    });
    time.ok_or_else(|| format!("{} is not a TimeOnly", value.get()))
}

/// A `DateTimeOffset` in the ISO 8601 profile System.Text.Json reads: `yyyy-MM-dd`, then
/// optionally `THH:mm`, `:ss` and up to 16 fraction digits (7 kept), then `Z`, `+hh` or `+hh:mm`.
/// Without an offset the time is local, as .NET reads it.
fn instant(value: &RawValue) -> Result<Timestamp, String> {
    quoted(value, 10..=252)
        .and_then(|text| iso_instant(&text))
        .ok_or_else(|| format!("{} is not a DateTimeOffset", value.get()))
}

/// The unescaped content of a JSON string whose escaped content has a length in `lengths`, as the
/// .NET converters check it before parsing.
fn quoted(value: &RawValue, lengths: std::ops::RangeInclusive<usize>) -> Option<String> {
    let raw = value.get();
    let escaped_length = raw.len().checked_sub(2)?;
    if !raw.starts_with('"') || !lengths.contains(&escaped_length) {
        return None;
    }
    serde_json::from_str(raw).ok()
}

/// Ticks of a `TimeSpan` in the constant format, read as `Utf8Parser` does with `'c'`.
fn constant_time_span(text: &str) -> Option<i64> {
    let mut cursor = Cursor::new(text);
    let negative = cursor.byte(b'-').is_some();
    let unsigned = |cursor: &mut Cursor| digits(cursor.leading_digits(usize::MAX));
    let mut values = vec![unsigned(&mut cursor)?];
    let mut separators = Vec::new();
    while values.len() < 5 {
        let Some(&separator) = cursor.rest().first() else {
            break;
        };
        cursor.byte(separator);
        let value = match separator {
            b':' => unsigned(&mut cursor)?,
            // A period after the first component separates days; after the third, a fraction.
            b'.' if values.len() == 1 => unsigned(&mut cursor)?,
            b'.' => {
                let fraction = cursor.leading_digits(8);
                if !(1..=7).contains(&fraction.len()) {
                    return None;
                }
                digits(fraction)? * 10u32.pow(7 - fraction.len() as u32)
            }
            _ => return None,
        };
        values.push(value);
        separators.push(separator);
    }
    if !cursor.rest().is_empty() {
        return None;
    }
    let (days, hours, minutes, seconds, fraction) = match (separators.as_slice(), values.as_slice())
    {
        ([], &[days]) => (days, 0, 0, 0, 0),
        (b":", &[hours, minutes]) => (0, hours, minutes, 0, 0),
        (b".:", &[days, hours, minutes]) => (days, hours, minutes, 0, 0),
        (b"::", &[hours, minutes, seconds]) => (0, hours, minutes, seconds, 0),
        (b".::", &[days, hours, minutes, seconds]) => (days, hours, minutes, seconds, 0),
        (b"::.", &[hours, minutes, seconds, fraction]) => (0, hours, minutes, seconds, fraction),
        (b".::.", &[days, hours, minutes, seconds, fraction]) => {
            (days, hours, minutes, seconds, fraction)
        }
        _ => return None,
    };
    if hours > 23 || minutes > 59 || seconds > 59 {
        return None;
    }
    let seconds = i64::from(days) * 86_400
        + i64::from(hours) * 3_600
        + i64::from(minutes) * 60
        + i64::from(seconds);
    // .NET bounds the whole milliseconds first, then the ticks.
    let milliseconds = seconds * 1_000;
    if milliseconds > i64::MAX / 10_000 {
        return None;
    }
    let whole = milliseconds * 10_000;
    if negative {
        (-whole).checked_sub(i64::from(fraction))
    } else {
        whole.checked_add(i64::from(fraction))
    }
}

fn iso_instant(text: &str) -> Option<Timestamp> {
    let mut cursor = Cursor::new(text);
    let date = cursor.date()?;
    if cursor.rest().is_empty() {
        return dotnet_instant(date.to_datetime(Time::midnight()), None);
    }
    let hour = cursor.byte(b'T')?.number(2)?;
    let minute = cursor.byte(b':')?.number(2)?;
    let (mut second, mut nanosecond) = (0, 0);
    if cursor.byte(b':').is_some() {
        second = cursor.number(2)?;
        if cursor.byte(b'.').is_some() {
            // At least one character follows the period, though it need not be a digit.
            if cursor.rest().is_empty() {
                return None;
            }
            let fraction = cursor.leading_digits(16);
            let kept = &fraction[..fraction.len().min(7)];
            if let Some(value) = digits(kept) {
                nanosecond = value * 10u32.pow(9 - kept.len() as u32);
            }
        }
    }
    let datetime = civil_datetime(date, hour, minute, second, nanosecond)?;
    let offset_minutes = match cursor.rest() {
        b"" => None,
        b"Z" => Some(0),
        [sign @ (b'+' | b'-'), offset @ ..] => {
            let (hours, minutes) = match offset {
                [h1, h2] => (digits(&[*h1, *h2])?, 0),
                [h1, h2, b':', m1, m2] => (digits(&[*h1, *h2])?, digits(&[*m1, *m2])?),
                _ => return None,
            };
            if minutes > 59 || hours * 60 + minutes > 14 * 60 {
                return None;
            }
            let minutes = (hours * 60 + minutes) as i32;
            Some(if *sign == b'-' { -minutes } else { minutes })
        }
        _ => return None,
    };
    dotnet_instant(datetime, offset_minutes)
}

/// Writes settings as the .NET store does, with `newline` between lines and none at the end,
/// then the members of `previous`, the text of the file replaced, that the settings do not know.
pub(crate) fn write(settings: &Settings, newline: &str, previous: Option<&str>) -> String {
    let notifications = &settings.notifications;
    let source = &settings.activity_watch;
    let settlement = &source.settlement;
    let document = Json::object(vec![
        ("schemaVersion", Json::number(settings.schema_version)),
        (
            "nominalDailyTarget",
            Json::duration(settings.nominal_daily_target),
        ),
        (
            "minimumEffectiveTarget",
            Json::duration(settings.minimum_effective_target),
        ),
        ("lunchStart", Json::time(settings.lunch_start)),
        ("lunchEnd", Json::time(settings.lunch_end)),
        (
            "preferredLatestDeparture",
            Json::time(settings.preferred_latest_departure),
        ),
        (
            "orangeWarningMargin",
            Json::duration(settings.orange_warning_margin),
        ),
        (
            "longLockReviewThreshold",
            Json::duration(settings.long_lock_review_threshold),
        ),
        (
            "weeklyCreditEnabled",
            Json::number(settings.weekly_credit_enabled),
        ),
        (
            "weeklyDebtEnabled",
            Json::number(settings.weekly_debt_enabled),
        ),
        (
            "maximumWeeklyDebt",
            Json::optional(settings.maximum_weekly_debt, Json::duration),
        ),
        (
            "startWithWindows",
            Json::number(settings.start_with_windows),
        ),
        (
            "notifications",
            Json::object(vec![
                (
                    "completionApproach",
                    Json::rule(notifications.completion_approach),
                ),
                (
                    "latestDepartureApproach",
                    Json::rule(notifications.latest_departure_approach),
                ),
                ("dayComplete", Json::rule(notifications.day_complete)),
            ]),
        ),
        (
            "activitySource",
            Json::optional(settings.activity_source, Json::activity_source),
        ),
        (
            "builtInActivity",
            Json::object(vec![(
                "idleThreshold",
                Json::duration(settings.built_in_activity.idle_threshold),
            )]),
        ),
        (
            "activityWatch",
            Json::object(vec![
                ("baseUrl", Json::Text(source.base_url.clone())),
                (
                    "selectedBucketId",
                    Json::optional(source.selected_bucket_id.clone(), Json::Text),
                ),
                (
                    "apiKeyReference",
                    Json::optional(source.api_key_reference.clone(), Json::Text),
                ),
                ("requestTimeout", Json::duration(source.request_timeout)),
                (
                    "settlement",
                    Json::object(vec![
                        ("afkTimeout", Json::duration(settlement.afk_timeout)),
                        ("pollInterval", Json::duration(settlement.poll_interval)),
                        ("maximumJitter", Json::duration(settlement.maximum_jitter)),
                        ("horizon", Json::duration(settlement.horizon())),
                    ]),
                ),
            ]),
        ),
        (
            "updates",
            Json::object(vec![
                (
                    "checkForUpdates",
                    Json::number(settings.updates.check_for_updates),
                ),
                (
                    "lastCheckedAtUtc",
                    Json::optional(settings.updates.last_checked_at, Json::instant),
                ),
            ]),
        ),
        ("usagePing", Json::number(settings.usage_ping)),
        (
            "recurringAppointments",
            Json::Array(
                settings
                    .recurring_appointments
                    .iter()
                    .map(Json::recurring_appointment)
                    .collect(),
            ),
        ),
        (
            "appointmentProposals",
            Json::object(vec![
                (
                    "enabled",
                    Json::number(settings.appointment_proposals.enabled),
                ),
                (
                    "dismissed",
                    Json::Array(
                        settings
                            .appointment_proposals
                            .dismissed
                            .iter()
                            .map(Json::dismissed_proposal)
                            .collect(),
                    ),
                ),
            ]),
        ),
        (
            "logging",
            Json::object(vec![
                (
                    "maximumFileCount",
                    Json::number(settings.logging.maximum_file_count),
                ),
                (
                    "maximumFileBytes",
                    Json::number(settings.logging.maximum_file_bytes),
                ),
            ]),
        ),
        (
            "languageOverride",
            Json::optional(settings.language_override.clone(), Json::Text),
        ),
    ]);
    let mut document = document;
    if let (Some(previous), Json::Object(members)) = (previous, &mut document) {
        keep_unknown(members, previous);
    }
    let mut text = String::new();
    document.render(0, newline, &mut text);
    text
}

/// Appends to `members` those of the JSON object `previous` whose name matches none of them in
/// any ASCII case, as the read matches them, and does the same inside each known object. The
/// last of duplicate unknown members wins, as on read; what cannot be read is dropped.
fn keep_unknown(members: &mut Vec<(Cow<'static, str>, Json)>, previous: &str) {
    let Ok(previous) = object(previous) else {
        return;
    };
    let known = members.len();
    for (name, value) in previous {
        if let Some(index) = members[..known]
            .iter()
            .position(|(member, _)| member.eq_ignore_ascii_case(&name))
        {
            if let Json::Object(children) = &mut members[index].1 {
                keep_unknown(children, value.get());
            }
            continue;
        }
        let Some(value) = unknown(value) else {
            continue;
        };
        match members[known..]
            .iter()
            .position(|(member, _)| *member == name)
        {
            Some(index) => members[known + index].1 = value,
            None => members.push((Cow::Owned(name), value)),
        }
    }
}

/// A member the settings do not know, as read: its scalars keep their text and escapes.
fn unknown(value: &RawValue) -> Option<Json> {
    let raw = value.get();
    if raw.starts_with('{') {
        object(raw)
            .ok()?
            .into_iter()
            .map(|(name, value)| Some((Cow::Owned(name), unknown(value)?)))
            .collect::<Option<_>>()
            .map(Json::Object)
    } else if raw.starts_with('[') {
        serde_json::from_str::<Vec<&RawValue>>(raw)
            .ok()?
            .into_iter()
            .map(unknown)
            .collect::<Option<_>>()
            .map(Json::Array)
    } else {
        Some(Json::Raw(raw.to_owned()))
    }
}

enum Json {
    /// Written as is.
    Raw(String),
    Text(String),
    Object(Vec<(Cow<'static, str>, Json)>),
    Array(Vec<Json>),
}

impl Json {
    fn object(members: Vec<(&'static str, Json)>) -> Self {
        Self::Object(
            members
                .into_iter()
                .map(|(name, member)| (Cow::Borrowed(name), member))
                .collect(),
        )
    }

    fn number(value: impl ToString) -> Self {
        Self::Raw(value.to_string())
    }

    fn optional<T>(value: Option<T>, write: fn(T) -> Self) -> Self {
        value.map_or_else(|| Self::Raw("null".to_owned()), write)
    }

    fn rule(rule: NotificationRule) -> Self {
        Self::object(vec![
            ("enabled", Self::number(rule.enabled)),
            ("leadTime", Self::duration(rule.lead_time)),
        ])
    }

    fn recurring_appointment(appointment: &RecurringAppointment) -> Self {
        let (_, weekday) = WEEKDAYS[appointment.weekday.to_monday_zero_offset() as usize];
        Self::object(vec![
            ("id", Self::Text(appointment.id.clone())),
            ("name", Self::Text(appointment.name.clone())),
            ("weekday", Self::Text(weekday.to_owned())),
            ("start", Self::time(appointment.start)),
            ("end", Self::time(appointment.end)),
            ("enabled", Self::number(appointment.enabled)),
            (
                "since",
                Self::optional(appointment.since, |date| Self::Text(date.to_string())),
            ),
            (
                "alternateWeeksFrom",
                Self::optional(appointment.alternate_weeks_from, |date| {
                    Self::Text(date.to_string())
                }),
            ),
            ("automatic", Self::number(appointment.automatic)),
        ])
    }

    fn dismissed_proposal(dismissed: &DismissedProposal) -> Self {
        let (_, weekday) = WEEKDAYS[dismissed.weekday.to_monday_zero_offset() as usize];
        Self::object(vec![
            ("weekday", Self::Text(weekday.to_owned())),
            ("start", Self::time(dismissed.start)),
            ("end", Self::time(dismissed.end)),
            (
                "until",
                Self::optional(dismissed.until, |date| Self::Text(date.to_string())),
            ),
        ])
    }

    fn activity_source(source: ActivitySourceKind) -> Self {
        Self::Text(
            match source {
                ActivitySourceKind::BuiltIn => "built-in",
                ActivitySourceKind::ActivityWatch => "activitywatch",
            }
            .to_owned(),
        )
    }

    /// `[-][d.]hh:mm:ss[.fffffff]`, truncated to ticks.
    fn duration(duration: SignedDuration) -> Self {
        let ticks = duration.as_nanos() / 100;
        let sign = if ticks < 0 { "-" } else { "" };
        let ticks = ticks.unsigned_abs();
        let days = ticks / TICKS_PER_DAY;
        let days = if days > 0 {
            format!("{days}.")
        } else {
            String::new()
        };
        Self::Text(format!("{sign}{days}{}", clock(ticks % TICKS_PER_DAY)))
    }

    /// `HH:mm:ss[.fffffff]`, truncated to ticks.
    fn time(time: Time) -> Self {
        let ticks = time.duration_since(Time::midnight()).as_nanos() / 100;
        Self::Text(clock(ticks.unsigned_abs()))
    }

    /// The round-trip format in UTC without trailing fraction zeros. .NET formats it without the
    /// encoder, so the `+` is not escaped.
    fn instant(instant: Timestamp) -> Self {
        let utc = TimeZone::UTC.to_datetime(instant);
        let fraction = format!("{:07}", utc.subsec_nanosecond() / 100);
        let fraction = fraction.trim_end_matches('0');
        let separator = if fraction.is_empty() { "" } else { "." };
        Self::Raw(format!(
            "\"{}{separator}{fraction}+00:00\"",
            utc.strftime("%Y-%m-%dT%H:%M:%S")
        ))
    }

    fn render(&self, depth: usize, newline: &str, out: &mut String) {
        match self {
            Self::Raw(raw) => out.push_str(raw),
            Self::Text(text) => escape(text, out),
            Self::Object(members) if members.is_empty() => out.push_str("{}"),
            Self::Object(members) => {
                out.push('{');
                for (index, (name, member)) in members.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(newline);
                    out.push_str(&"  ".repeat(depth + 1));
                    escape(name, out);
                    out.push_str(": ");
                    member.render(depth + 1, newline, out);
                }
                out.push_str(newline);
                out.push_str(&"  ".repeat(depth));
                out.push('}');
            }
            Self::Array(items) if items.is_empty() => out.push_str("[]"),
            Self::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(newline);
                    out.push_str(&"  ".repeat(depth + 1));
                    item.render(depth + 1, newline, out);
                }
                out.push_str(newline);
                out.push_str(&"  ".repeat(depth));
                out.push(']');
            }
        }
    }
}

/// `hh:mm:ss[.fffffff]` of ticks within a day.
fn clock(ticks: u128) -> String {
    let (seconds, fraction) = (ticks / TICKS_PER_SECOND, ticks % TICKS_PER_SECOND);
    let clock = format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    );
    if fraction == 0 {
        clock
    } else {
        format!("{clock}.{fraction:07}")
    }
}

/// A JSON string as the default .NET encoder writes it: printable ASCII stays, except the
/// HTML-sensitive `"&'+<>` and backtick, which become `\u` escapes like every other character.
fn escape(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' if !matches!(character, '"' | '&' | '\'' | '+' | '<' | '>' | '`') => {
                out.push(character);
            }
            _ => {
                for unit in character.encode_utf16(&mut [0; 2]) {
                    let _ = write!(out, "\\u{unit:04X}");
                }
            }
        }
    }
    out.push('"');
}
