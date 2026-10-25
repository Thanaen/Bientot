//! The `BreakdownJson` column, ported from `History/DailySummaryBreakdownCodec.cs`.
//!
//! An append compares the stored text with the text of the candidate, so the writer reproduces
//! `Utf8JsonWriter` byte for byte: compact, keys in a fixed order, explicit nulls, and the
//! escaping of its default encoder.

use std::fmt::Write as _;

use bientot_domain::accounting::{AccountingLabel, AdjustmentId, LongLockReviewId};
use bientot_domain::history::{
    DailySummaryBreakdown, DailySummaryBreakdownEntry, DailySummaryDisposition,
};
use bientot_domain::intervals::UtcInterval;
use serde_json::{Map, Value};

use super::{named, stored_name};
use crate::codec::{
    DurationOutOfRange, format_instant, from_ticks, parse_instant, to_ticks, write_json_string,
};

const CURRENT_VERSION: i64 = 1;

const DISPOSITIONS: [(DailySummaryDisposition, &str); 5] = [
    (DailySummaryDisposition::ConfirmedCredit, "confirmed-credit"),
    (
        DailySummaryDisposition::ProvisionalCredit,
        "provisional-credit",
    ),
    (DailySummaryDisposition::Excluded, "excluded"),
    (DailySummaryDisposition::Unknown, "unknown"),
    (DailySummaryDisposition::ManualCredit, "manual-credit"),
];

const LABELS: [(AccountingLabel, &str); 11] = [
    (AccountingLabel::ActiveWork, "active-work"),
    (AccountingLabel::Afk, "afk"),
    (AccountingLabel::LunchLock, "lunch-lock"),
    (AccountingLabel::LockedWork, "locked-work"),
    (AccountingLabel::Suspended, "suspended"),
    (AccountingLabel::ManualPause, "manual-pause"),
    (AccountingLabel::DayFinished, "day-finished"),
    (AccountingLabel::Manual, "manual"),
    (AccountingLabel::Unknown, "unknown"),
    (AccountingLabel::OvernightLock, "overnight-lock"),
    (AccountingLabel::ScheduledAway, "scheduled-away"),
];

pub(crate) fn serialize(breakdown: &DailySummaryBreakdown) -> Result<String, DurationOutOfRange> {
    let mut json = format!("{{\"version\":{CURRENT_VERSION},\"entries\":[");
    for (index, entry) in breakdown.entries().iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str("{\"disposition\":");
        write_json_string(&mut json, stored_name(&DISPOSITIONS, entry.disposition()));
        json.push_str(",\"label\":");
        write_json_string(&mut json, stored_name(&LABELS, entry.label()));
        write!(json, ",\"durationTicks\":{}", to_ticks(entry.duration())?)
            .expect("writing to a String cannot fail");
        let interval = entry.interval();
        json.push_str(",\"startUtc\":");
        write_optional(
            &mut json,
            interval.map(|interval| format_instant(interval.start())),
        );
        json.push_str(",\"endUtc\":");
        write_optional(
            &mut json,
            interval.map(|interval| format_instant(interval.end())),
        );
        json.push_str(",\"adjustmentId\":");
        write_optional(&mut json, entry.adjustment_id().map(AdjustmentId::as_str));
        json.push_str(",\"reviewId\":");
        write_optional(&mut json, entry.review_id().map(LongLockReviewId::as_str));
        json.push('}');
    }
    json.push_str("]}");
    Ok(json)
}

fn write_optional(json: &mut String, value: Option<impl AsRef<str>>) {
    match value {
        Some(value) => write_json_string(json, value.as_ref()),
        None => json.push_str("null"),
    }
}

/// Reads the breakdown the way the .NET codec does; the error says what it rejects.
pub(crate) fn deserialize(json: &str) -> Result<DailySummaryBreakdown, String> {
    let root: Value = serde_json::from_str(json)
        .map_err(|_| "Daily summary breakdown is malformed.".to_owned())?;
    let version = root
        .as_object()
        .and_then(|root| root.get("version"))
        .and_then(Value::as_i64)
        .filter(|version| i32::try_from(*version).is_ok())
        .ok_or("Daily summary breakdown must contain an integer version.")?;
    if version != CURRENT_VERSION {
        return Err(format!(
            "Unsupported daily summary breakdown version {version}."
        ));
    }
    let entries = required(&root, "entries")?
        .as_array()
        .ok_or("Daily summary breakdown entries must be an array.")?;
    let entries = entries
        .iter()
        .map(entry)
        .collect::<Result<Vec<_>, String>>()?;
    Ok(DailySummaryBreakdown::new(entries))
}

fn entry(element: &Value) -> Result<DailySummaryBreakdownEntry, String> {
    let disposition = required_string(element, "disposition")?;
    let disposition = named(&DISPOSITIONS, disposition)
        .ok_or_else(|| format!("Unknown daily summary disposition '{disposition}'."))?;
    let label = required_string(element, "label")?;
    let label =
        named(&LABELS, label).ok_or_else(|| format!("Unknown accounting label '{label}'."))?;
    let duration_ticks = required(element, "durationTicks")?
        .as_i64()
        .ok_or("Daily summary breakdown 'durationTicks' must be an integer.")?;
    let start = optional_instant(element, "startUtc")?;
    let end = optional_instant(element, "endUtc")?;
    let interval = match (start, end) {
        (Some(start), Some(end)) => Some(UtcInterval::new(start, end).map_err(|_| malformed())?),
        (None, None) => None,
        _ => return Err("Breakdown start and end instants must be provided together.".to_owned()),
    };
    let adjustment_id = optional_string(element, "adjustmentId")?
        .map(|id| AdjustmentId::new(id).map_err(|_| malformed()))
        .transpose()?;
    let review_id = optional_string(element, "reviewId")?
        .map(|id| LongLockReviewId::new(id).map_err(|_| malformed()))
        .transpose()?;
    DailySummaryBreakdownEntry::new(
        disposition,
        label,
        from_ticks(duration_ticks),
        interval,
        adjustment_id,
        review_id,
    )
    .map_err(|_| malformed())
}

fn malformed() -> String {
    "Daily summary breakdown is malformed.".to_owned()
}

fn required<'v>(element: &'v Value, name: &str) -> Result<&'v Value, String> {
    element
        .as_object()
        .and_then(|object: &Map<String, Value>| object.get(name))
        .ok_or_else(|| format!("Daily summary breakdown is missing '{name}'."))
}

fn required_string<'v>(element: &'v Value, name: &str) -> Result<&'v str, String> {
    required(element, name)?
        .as_str()
        .ok_or_else(|| format!("Daily summary breakdown '{name}' must be a string."))
}

fn optional_string<'v>(element: &'v Value, name: &str) -> Result<Option<&'v str>, String> {
    match required(element, name)? {
        Value::Null => Ok(None),
        Value::String(value) => Ok(Some(value)),
        _ => Err(format!(
            "Daily summary breakdown '{name}' must be a string or null."
        )),
    }
}

fn optional_instant(element: &Value, name: &str) -> Result<Option<jiff::Timestamp>, String> {
    optional_string(element, name)?
        .map(|text| {
            parse_instant(text)
                .map_err(|_| format!("Stored {name} is not a canonical UTC round-trip timestamp."))
        })
        .transpose()
}

#[cfg(test)]
mod tests;
