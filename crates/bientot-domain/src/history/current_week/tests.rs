use jiff::SignedDuration;
use jiff::Timestamp;

use super::*;
use crate::accounting::AccountingLabel;
use crate::history::{
    DailySummaryBreakdown, DailySummaryBreakdownEntry, DailySummaryDisposition,
    DailySummarySnapshot, DailySummarySnapshotParts, InvalidSnapshot,
};
use crate::intervals::UtcInterval;
use crate::sources::SourceQuality;
use crate::summaries::IndicatorState;
use crate::time::LocalWorkday;

const TICK: SignedDuration = SignedDuration::from_nanos(100);

fn instant(date: Date, hour: i8) -> Timestamp {
    date.at(hour, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn snapshot(
    date: Date,
    credited_ticks: i32,
    effective_target_ticks: i32,
    indicator_state: IndicatorState,
    projected_departure: Option<Timestamp>,
) -> Result<DailySummarySnapshot, InvalidSnapshot> {
    let credited_work = TICK * credited_ticks;
    let effective_target = TICK * effective_target_ticks;
    let start = instant(date, 8);
    DailySummarySnapshot::new(DailySummarySnapshotParts {
        workday: LocalWorkday::new(date, "Europe/Paris"),
        computed_at: instant(date, 12),
        input_fingerprint: format!("fingerprint-{}", date.strftime("%Y%m%d")),
        credited_work,
        excluded_work: SignedDuration::ZERO,
        unknown_work: SignedDuration::ZERO,
        nominal_target: effective_target,
        effective_target,
        carry_available: SignedDuration::ZERO,
        carry_applied: SignedDuration::ZERO,
        carry_earned: SignedDuration::ZERO,
        carry_expired: SignedDuration::ZERO,
        carry_debt: SignedDuration::ZERO,
        debt_pending: false,
        projected_departure,
        indicator_state,
        source_quality: SourceQuality::Complete,
        has_pending_review: false,
        breakdown: DailySummaryBreakdown::new([DailySummaryBreakdownEntry::new(
            DailySummaryDisposition::ConfirmedCredit,
            AccountingLabel::ActiveWork,
            credited_work,
            Some(UtcInterval::new(start, start + credited_work).unwrap()),
            None,
            None,
        )
        .unwrap()]),
    })
}

fn revision(date: Date, revision: i64) -> DailySummaryRevision {
    let departure = instant(jiff::civil::date(2026, 9, 7), 18);
    DailySummaryRevision::new(
        revision,
        snapshot(date, 1, 2, IndicatorState::Green, Some(departure)).unwrap(),
    )
    .unwrap()
}

#[test]
fn weekend_at_year_boundary_produces_five_ordered_slots_with_missing_days() {
    let monday = revision(jiff::civil::date(2026, 12, 28), 2);
    let friday = revision(jiff::civil::date(2027, 1, 1), 1);

    let week = CurrentWeek::new(
        jiff::civil::date(2027, 1, 3),
        [friday.clone(), monday.clone()],
    )
    .unwrap();

    assert_eq!(week.week_start_date(), jiff::civil::date(2026, 12, 28));
    assert_eq!(
        week.days().iter().map(|day| day.date).collect::<Vec<_>>(),
        (0..5)
            .map(|offset| days_after(week.week_start_date(), offset))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        week.days()
            .iter()
            .map(|day| day.latest_revision.as_ref())
            .collect::<Vec<_>>(),
        [Some(&monday), None, None, None, Some(&friday)]
    );
}

#[test]
fn snapshot_rejects_headline_that_contradicts_completion_state() {
    let result = snapshot(
        jiff::civil::date(2026, 9, 7),
        2,
        2,
        IndicatorState::Green,
        Some(instant(jiff::civil::date(2026, 9, 7), 18)),
    );

    assert_eq!(result, Err(InvalidSnapshot::IndicatorContradictsCompletion));
}

#[test]
fn revisions_outside_the_week_or_on_the_same_day_are_rejected() {
    let monday = jiff::civil::date(2026, 9, 7);

    assert_eq!(
        CurrentWeek::new(monday, [revision(days_after(monday, 7), 1)]),
        Err(InvalidCurrentWeek::RevisionOutsideWeek)
    );
    assert_eq!(
        CurrentWeek::new(monday, [revision(monday, 1), revision(monday, 2)]),
        Err(InvalidCurrentWeek::DuplicateDay)
    );
    assert_eq!(
        DailySummaryRevision::new(0, revision(monday, 1).snapshot().clone()),
        Err(crate::history::NonPositiveRevision)
    );
}
