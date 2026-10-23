//! Ported from `WorkdayAccountingServiceTests.cs` and the accounting cases of
//! `TrackingControlIntegrationTests.cs`, against a migrated SQLite ledger.

use std::cell::{Cell, RefCell};

use jiff::civil::date;
use worktime_domain::accounting::{AccountingLabel, ActivitySegment, ActivityState};
use worktime_domain::forecasting::ForecastAvailability;
use worktime_domain::sources::{SourceDiagnosticInterval, SourceIntervalKind};
use worktime_domain::summaries::IndicatorState;
use worktime_domain::time::FrozenClock;

use super::*;
use crate::test_support::{CountingClock, Database, utc, utc_zones};

/// A source that answers each read with `answer` and keeps the intervals it was asked for.
struct RecordingSource<F> {
    answer: F,
    requests: RefCell<Vec<UtcInterval>>,
}

impl<F: Fn(UtcInterval) -> ActivityReadResult> RecordingSource<F> {
    fn new(answer: F) -> Self {
        Self {
            answer,
            requests: RefCell::new(Vec::new()),
        }
    }
}

impl<F: Fn(UtcInterval) -> ActivityReadResult> ActivitySource for RecordingSource<F> {
    fn read(&self, interval: UtcInterval) -> Result<ActivityReadResult, ActivityError> {
        self.requests.borrow_mut().push(interval);
        Ok((self.answer)(interval))
    }
}

/// A time zone provider that counts how often it is asked for the local zone and for a named
/// one.
struct CountingZones {
    zones: FixedTimeZoneProvider,
    local_reads: Cell<usize>,
    named_reads: Cell<usize>,
}

impl TimeZoneProvider for CountingZones {
    fn local(&self) -> (String, TimeZone) {
        self.local_reads.set(self.local_reads.get() + 1);
        self.zones.local()
    }

    fn get(&self, id: &str) -> Result<TimeZone, worktime_domain::time::UnknownTimeZone> {
        self.named_reads.set(self.named_reads.get() + 1);
        self.zones.get(id)
    }
}

fn interval(start: Timestamp, end: Timestamp) -> UtcInterval {
    UtcInterval::new(start, end).unwrap()
}

/// Active throughout the read, as a healthy ActivityWatch bucket reported it.
fn fully_active(interval: UtcInterval) -> ActivityReadResult {
    fully_active_with(
        interval,
        SourceHealth::healthy("activitywatch", interval.end()),
    )
}

fn fully_active_with(interval: UtcInterval, health: SourceHealth) -> ActivityReadResult {
    ActivityReadResult::Snapshot {
        coverage: ActivityCoverage::new(
            interval,
            [ActivitySegment::new(
                interval,
                ActivityState::Active,
                AutomaticOrigin::new("activitywatch:test").unwrap(),
            )],
        ),
        health,
    }
}

fn utc_context(settings: WorkTimeSettings, observed_at: Timestamp) -> CalculationContext {
    CalculationContext {
        settings,
        time_zone_id: "UTC".into(),
        time_zone: TimeZone::UTC,
        observed_at,
    }
}

/// Calculates `day` in `context` on an empty ledger, reading `source`.
fn calculate_with(
    source: &dyn ActivitySource,
    context: &CalculationContext,
    day: Date,
) -> Result<WorkdayCalculation, AccountingError> {
    let database = Database::new();
    let clock = FrozenClock::new(context.observed_at);
    let zones = FixedTimeZoneProvider::new(context.time_zone_id.clone(), context.time_zone.clone());
    let settings = || context.settings.clone();
    let accounting = WorkdayAccounting::new(
        source,
        SessionEventRepository::new(&database.connection, &clock),
        ManualAdjustmentRepository::new(&database.connection),
        &clock,
        &zones,
        &settings,
    );
    accounting.calculate(day, context)
}

/// Today's nominal summary on an empty ledger, in UTC, with the default settings.
fn current_summary(source: &dyn ActivitySource, clock: &dyn Clock) -> DailySummary {
    let database = Database::new();
    let zones = utc_zones();
    let settings = WorkTimeSettings::default;
    WorkdayAccounting::new(
        source,
        SessionEventRepository::new(&database.connection, clock),
        ManualAdjustmentRepository::new(&database.connection),
        clock,
        &zones,
        &settings,
    )
    .calculate_current()
    .unwrap()
}

#[test]
fn current_calculation_captures_settings_time_zone_and_clock_once() {
    let now = utc(2026, 9, 8, 11);
    let clock = CountingClock::new(now);
    let zones = CountingZones {
        zones: utc_zones(),
        local_reads: Cell::new(0),
        named_reads: Cell::new(0),
    };
    let settings_reads = Cell::new(0);
    let settings = || {
        settings_reads.set(settings_reads.get() + 1);
        WorkTimeSettings {
            nominal_daily_target: SignedDuration::from_hours(8),
            ..WorkTimeSettings::default()
        }
    };
    let source = RecordingSource::new(fully_active);
    let database = Database::new();
    let ledger_clock = FrozenClock::new(now);
    let accounting = WorkdayAccounting::new(
        &source,
        SessionEventRepository::new(&database.connection, &ledger_clock),
        ManualAdjustmentRepository::new(&database.connection),
        &clock,
        &zones,
        &settings,
    );

    let summary = accounting.calculate_current().unwrap();

    assert_eq!(clock.reads.get(), 1);
    assert_eq!(zones.local_reads.get(), 1);
    assert_eq!(zones.named_reads.get(), 0);
    assert_eq!(settings_reads.get(), 1);
    assert_eq!(summary.computed_at(), now);
    assert_eq!(summary.nominal_target(), SignedDuration::from_hours(8));
    assert_eq!(summary.effective_target(), SignedDuration::from_hours(8));
    assert_eq!(
        *source.requests.borrow(),
        [interval(utc(2026, 9, 8, 0), now)]
    );
}

#[test]
fn historical_calculation_uses_the_full_dst_window_and_real_observation_time() {
    let observed_at = utc(2026, 10, 26, 12);
    let context = CalculationContext {
        settings: WorkTimeSettings::default(),
        time_zone_id: "Europe/Paris".into(),
        time_zone: TimeZone::get("Europe/Paris").unwrap(),
        observed_at,
    };
    let source = RecordingSource::new(fully_active);

    let result = calculate_with(&source, &context, date(2026, 10, 25)).unwrap();

    let requests = source.requests.borrow();
    assert_eq!(requests.len(), 1);
    let request = requests[0];
    assert_eq!(request.duration(), SignedDuration::from_hours(25));
    assert_eq!(result.summary_input.projection_anchor(), request.end());
    assert_eq!(
        result.summary_input.accounting().timeline().through(),
        request.end()
    );
    assert_eq!(result.summary_input.observed_at(), observed_at);
    assert_eq!(
        result.summary_input.source_diagnostics().observed_at(),
        observed_at
    );
    assert_eq!(result.activity_read, ActivityReadDisposition::Snapshot);
    assert!(result.has_usable_source_snapshot());
}

#[test]
fn future_date_is_rejected_before_any_source_or_persistence_read() {
    let observed_at = utc(2026, 9, 8, 11);
    let context = utc_context(WorkTimeSettings::default(), observed_at);
    // Without tables, any read of the ledger fails, as the current day shows.
    let database = Database::unmigrated();
    let clock = FrozenClock::new(observed_at);
    let zones = utc_zones();
    let settings = WorkTimeSettings::default;
    let source = RecordingSource::new(fully_active);
    let accounting = WorkdayAccounting::new(
        &source,
        SessionEventRepository::new(&database.connection, &clock),
        ManualAdjustmentRepository::new(&database.connection),
        &clock,
        &zones,
        &settings,
    );

    let future = accounting.calculate(date(2026, 9, 9), &context);
    let today = accounting.calculate(date(2026, 9, 8), &context);

    assert!(matches!(future, Err(AccountingError::FutureWorkday)));
    assert!(source.requests.borrow().is_empty());
    assert!(matches!(today, Err(AccountingError::Events(_))));
}

#[test]
fn exact_current_midnight_tags_the_skipped_source_read() {
    let midnight = utc(2026, 9, 8, 0);
    let context = utc_context(WorkTimeSettings::default(), midnight);
    let source = RecordingSource::new(fully_active);

    let result = calculate_with(&source, &context, date(2026, 9, 8)).unwrap();

    assert_eq!(result.activity_read, ActivityReadDisposition::NotRequested);
    assert!(!result.has_usable_source_snapshot());
    assert!(source.requests.borrow().is_empty());
    assert_eq!(result.summary_input.projection_anchor(), midnight);
}

#[test]
fn raw_activity_disposition_survives_normalization() {
    let context = utc_context(WorkTimeSettings::default(), utc(2026, 9, 8, 11));
    let day = date(2026, 9, 8);
    let stale_ambiguous = RecordingSource::new(|interval| {
        fully_active_with(
            interval,
            SourceHealth::degraded(
                "activitywatch",
                SourceFreshness::Stale,
                SourceQuality::Ambiguous,
                "retained-snapshot",
                None,
            )
            .unwrap(),
        )
    });
    let unavailable = RecordingSource::new(|_| ActivityReadResult::Unavailable {
        health: SourceHealth::degraded(
            "activitywatch",
            SourceFreshness::Unavailable,
            SourceQuality::Incomplete,
            "unavailable",
            None,
        )
        .unwrap(),
    });

    let stale_ambiguous = calculate_with(&stale_ambiguous, &context, day).unwrap();
    let unavailable = calculate_with(&unavailable, &context, day).unwrap();

    assert_eq!(
        stale_ambiguous.activity_read,
        ActivityReadDisposition::Snapshot
    );
    assert!(stale_ambiguous.has_usable_source_snapshot());
    assert_eq!(
        unavailable.activity_read,
        ActivityReadDisposition::Unavailable
    );
    assert!(!unavailable.has_usable_source_snapshot());
}

#[test]
fn the_uncovered_live_tail_is_pending_within_the_selected_sources_own_horizon() {
    use ActivitySourceKind::{ActivityWatch, BuiltIn};
    use SourceIntervalKind::{PendingSettlement, UnresolvedCoverage};
    let observed_at = utc(2026, 9, 8, 11);
    let cases = [
        (BuiltIn, 600, 300, PendingSettlement),
        (BuiltIn, 30, 120, UnresolvedCoverage),
        (ActivityWatch, 600, 300, UnresolvedCoverage),
        (ActivityWatch, 30, 120, PendingSettlement),
    ];

    for (source, idle_threshold_seconds, tail_seconds, expected) in cases {
        let mut settings = WorkTimeSettings {
            activity_source: Some(source),
            ..WorkTimeSettings::default()
        };
        settings.built_in_activity.idle_threshold =
            SignedDuration::from_secs(idle_threshold_seconds);
        let covered_through = observed_at - SignedDuration::from_secs(tail_seconds);
        let reader = RecordingSource::new(|read: UtcInterval| ActivityReadResult::Snapshot {
            coverage: ActivityCoverage::new(
                read,
                [ActivitySegment::new(
                    interval(read.start(), covered_through),
                    ActivityState::Active,
                    AutomaticOrigin::new("test").unwrap(),
                )],
            ),
            health: SourceHealth::healthy("test", observed_at),
        });

        let result = calculate_with(
            &reader,
            &utc_context(settings, observed_at),
            date(2026, 9, 8),
        )
        .unwrap();

        assert_eq!(
            result.summary_input.source_diagnostics().intervals(),
            [SourceDiagnosticInterval::new(
                interval(covered_through, observed_at),
                expected,
                "test"
            )
            .unwrap()],
            "{source:?} with a {idle_threshold_seconds} s threshold and a {tail_seconds} s tail"
        );
    }
}

#[test]
fn healthy_active_weekend_is_schedule_excluded_and_never_credited() {
    let source = RecordingSource::new(fully_active);
    let clock = FrozenClock::new(utc(2026, 8, 29, 11));

    let summary = current_summary(&source, &clock);

    assert!(!summary.workday().is_working_day());
    assert_eq!(summary.credited_work(), SignedDuration::ZERO);
    assert!(!summary.has_source_warning());
    assert_eq!(summary.indicator_state(), IndicatorState::Green);
    let segments = summary.accounting().timeline().segments();
    assert!(!segments.is_empty());
    assert!(
        segments
            .iter()
            .all(|segment| segment.label() == AccountingLabel::DayFinished)
    );
}

#[test]
fn healthy_but_incomplete_activity_is_unknown_and_degraded() {
    let monday_start = utc(2026, 8, 24, 0);
    let eleven = utc(2026, 8, 24, 11);
    let empty = RecordingSource::new(|read| ActivityReadResult::Snapshot {
        coverage: ActivityCoverage::new(read, []),
        health: SourceHealth::healthy("activitywatch", eleven),
    });
    let gapped = RecordingSource::new(|read: UtcInterval| {
        let first_half_hour = interval(read.start(), read.start() + SignedDuration::from_mins(30));
        fully_active_with(
            first_half_hour,
            SourceHealth::healthy("activitywatch", eleven),
        )
    });
    let half_past = monday_start + SignedDuration::from_mins(30);
    let cases: [(&dyn ActivitySource, SignedDuration, Timestamp); 2] = [
        (&empty, SignedDuration::ZERO, monday_start),
        (&gapped, SignedDuration::from_mins(30), half_past),
    ];

    for (source, credited, unknown_from) in cases {
        let clock = FrozenClock::new(eleven);

        let summary = current_summary(source, &clock);

        assert_eq!(summary.credited_work(), credited);
        assert_eq!(
            summary.accounting().timeline().unknown_work(),
            eleven.duration_since(unknown_from)
        );
        assert!(summary.has_source_warning());
        assert_eq!(summary.source_health().quality, SourceQuality::Complete);
        assert_eq!(
            summary.assessment().forecast(),
            ForecastAvailability::Unavailable
        );
        assert_eq!(
            summary.source_diagnostics().intervals(),
            [SourceDiagnosticInterval::new(
                interval(unknown_from, eleven),
                SourceIntervalKind::UnresolvedCoverage,
                "activitywatch"
            )
            .unwrap()]
        );
    }
}

#[test]
fn unavailable_activity_becomes_unknown_without_credited_work() {
    let health = SourceHealth::degraded(
        "activitywatch",
        SourceFreshness::Unavailable,
        SourceQuality::Incomplete,
        "activitywatch-unavailable",
        None,
    )
    .unwrap();
    let source = RecordingSource::new(|_| ActivityReadResult::Unavailable {
        health: health.clone(),
    });
    let clock = FrozenClock::new(utc(2026, 8, 24, 11));

    let summary = current_summary(&source, &clock);

    assert_eq!(summary.credited_work(), SignedDuration::ZERO);
    assert_eq!(
        summary.accounting().timeline().unknown_work(),
        SignedDuration::from_hours(11)
    );
    assert_eq!(*summary.source_health(), health);
}

#[test]
fn accounting_captures_current_time_once_per_calculation() {
    let eleven = utc(2026, 8, 24, 11);
    let clock = CountingClock::new(eleven);
    let source = RecordingSource::new(fully_active);

    let summary = current_summary(&source, &clock);

    assert_eq!(summary.computed_at(), eleven);
    assert_eq!(clock.reads.get(), 1);
}

#[test]
fn an_absence_during_an_enabled_appointment_of_the_weekday_awaits_review() {
    use worktime_domain::accounting::{LongLockReviewId, LongLockReviewReason};
    use worktime_domain::configuration::RecurringAppointment;
    // Thursday 22 October 2026, at the PC from 09:00, away from 14:00 to 15:30.
    let day = date(2026, 10, 22);
    let observed_at = utc(2026, 10, 22, 18);
    let reader = RecordingSource::new(|read: UtcInterval| {
        let away = interval(
            utc(2026, 10, 22, 14),
            utc(2026, 10, 22, 14) + SignedDuration::from_mins(90),
        );
        let segment = |interval, state| {
            ActivitySegment::new(interval, state, AutomaticOrigin::new("test").unwrap())
        };
        ActivityReadResult::Snapshot {
            coverage: ActivityCoverage::new(
                read,
                [
                    segment(
                        interval(utc(2026, 10, 22, 9), away.start()),
                        ActivityState::Active,
                    ),
                    segment(away, ActivityState::Afk),
                    segment(interval(away.end(), observed_at), ActivityState::Active),
                ],
            ),
            health: SourceHealth::healthy("test", observed_at),
        }
    });
    let appointment = |weekday, enabled| RecurringAppointment {
        id: "meeting".to_owned(),
        name: String::new(),
        weekday,
        start: jiff::civil::Time::constant(14, 0, 0, 0),
        end: jiff::civil::Time::constant(16, 0, 0, 0),
        enabled,
        since: None,
        alternate_weeks_from: None,
        automatic: false,
    };
    let reviews = |appointment| {
        let settings = WorkTimeSettings {
            recurring_appointments: vec![appointment],
            ..WorkTimeSettings::default()
        };
        calculate_with(&reader, &utc_context(settings, observed_at), day)
            .unwrap()
            .summary_input
            .accounting()
            .pending_reviews()
            .to_vec()
    };

    let [review] = reviews(appointment(jiff::civil::Weekday::Thursday, true))
        .try_into()
        .expect("one review");
    assert_eq!(
        review.id,
        LongLockReviewId::for_appointment("meeting", utc(2026, 10, 22, 14))
    );
    assert_eq!(review.reason, LongLockReviewReason::RecurringAppointment);
    assert!(reviews(appointment(jiff::civil::Weekday::Thursday, false)).is_empty());
    assert!(reviews(appointment(jiff::civil::Weekday::Wednesday, true)).is_empty());
    let added_later = RecurringAppointment {
        since: Some(date(2026, 10, 23)),
        ..appointment(jiff::civil::Weekday::Thursday, true)
    };
    assert!(reviews(added_later).is_empty());
    // One Thursday in two, counted from a Thursday before or after this one.
    for (from, counts) in [
        (date(2026, 10, 8), true),
        (date(2026, 10, 15), false),
        (date(2026, 11, 5), true),
        (date(2026, 10, 29), false),
    ] {
        let alternate = RecurringAppointment {
            alternate_weeks_from: Some(from),
            ..appointment(jiff::civil::Weekday::Thursday, true)
        };
        assert_eq!(reviews(alternate).len(), usize::from(counts), "{from}");
    }
}
