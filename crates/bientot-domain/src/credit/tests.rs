mod credit_recalculation;
mod weekly_credit;
mod weekly_credit_ledger;
mod weekly_debt;

use jiff::civil::{Date, date};
use jiff::{SignedDuration, Timestamp};

use super::*;
use crate::configuration::Settings;
use crate::time::{LocalWorkday, WorkdayWindow, days_after};

const MONDAY: Date = date(2026, 9, 7);
const TICK: SignedDuration = SignedDuration::from_nanos(100);

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

fn day_of_week(offset: i64) -> Date {
    days_after(MONDAY, offset)
}

fn day(offset: i64, gross_minutes: i64) -> DailyCreditInput {
    DailyCreditInput::new(day_of_week(offset), minutes(gross_minutes))
}

fn settings() -> Settings {
    Settings {
        nominal_daily_target: minutes(420),
        minimum_effective_target: minutes(360),
        weekly_credit_enabled: true,
        ..Settings::default()
    }
}

fn utc(day: Date, hour: i8) -> Timestamp {
    day.at(hour, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn friday_window(monday: Date) -> WorkdayWindow {
    let friday = days_after(monday, 4);
    WorkdayWindow {
        workday: LocalWorkday::new(friday, "Europe/Paris"),
        start: utc(friday, 6),
        end: utc(friday, 20),
    }
}

fn revisions_of(
    results: &[DailyCreditResult],
    revisions: &[i64],
) -> std::collections::BTreeMap<Date, i64> {
    results
        .iter()
        .zip(revisions)
        .map(|(result, &revision)| (result.date(), revision))
        .collect()
}

fn sum(entries: &[CreditLedgerEntry], kind: CreditLedgerEntryKind) -> SignedDuration {
    entries
        .iter()
        .filter(|entry| entry.kind() == kind)
        .map(CreditLedgerEntry::amount)
        .sum()
}

/// Movement date, kind, amount in minutes, origin date and origin revision.
fn movement(entry: &CreditLedgerEntry) -> (Date, CreditLedgerEntryKind, SignedDuration, Date, i64) {
    (
        entry.workday_date(),
        entry.kind(),
        entry.amount(),
        entry.origin().workday_date,
        entry.origin().revision,
    )
}
