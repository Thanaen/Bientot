//! Ported from the demo cases of `HistoryIntegrationTests.cs` and `SettingsIntegrationTests.cs`,
//! against a migrated SQLite ledger in UTC. The .NET theories over `immediate` collapse into one
//! test: Rust has no source backoff to bypass.
//!
//! Not ported: the timeline text of `DemoHistoryWeekDrawsEveryKindOfSegmentForTheScreenshots`
//! (`HistoryFormatter` is R6's); its breakdown underneath is asserted instead. The settings
//! service and its JSON store of `SettingsRecalculateImmediatelyAndSurviveRestart` (R6); its
//! accounting is ported. `DemoStoreUpdatePort` in `StoreUpdateServiceTests.cs` (R6 wiring over
//! `bientot-windows::updates`).

use bientot_domain::accounting::AdjustmentId;
use bientot_domain::adjustments::ManualAdjustment;
use bientot_domain::configuration::Settings;
use bientot_domain::history::{CurrentWeek, DailySummaryDisposition};
use bientot_domain::summaries::IndicatorState;
use bientot_domain::time::FixedTimeZoneProvider;
use bientot_store::adjustments::ManualAdjustmentRepository;
use bientot_store::codec::{from_ticks, to_ticks};
use bientot_store::credit::WeeklyCreditRepository;
use bientot_store::history::DailySummaryRepository;
use jiff::civil::{Date, date};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use rusqlite::Connection;

use super::*;
use crate::history::HistoryService;
use crate::test_support::{Database, count, utc_accounting, utc_zones};

const TICK: SignedDuration = SignedDuration::from_nanos(100);

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

fn hours(hours: i64) -> SignedDuration {
    SignedDuration::from_hours(hours)
}

/// `duration` as the ledger stores it, in 100 ns ticks.
fn stored(duration: SignedDuration) -> SignedDuration {
    from_ticks(to_ticks(duration).unwrap())
}

fn utc_at(day: Date, hour: i8, minute: i8) -> Timestamp {
    day.at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn seed(
    connection: &Connection,
    demo: &DemoEnvironment<&FixedTimeZoneProvider>,
    zones: &FixedTimeZoneProvider,
) {
    let accounting = utc_accounting(connection, demo, demo, zones);
    let corrections =
        ManualAdjustmentCommands::new(&accounting, ManualAdjustmentRepository::new(connection));
    demo.seed(&SessionEventRepository::new(connection, demo), &corrections)
        .unwrap();
}

fn current_week(
    connection: &Connection,
    demo: &DemoEnvironment<&FixedTimeZoneProvider>,
    zones: &FixedTimeZoneProvider,
) -> CurrentWeek {
    HistoryService::new(DailySummaryRepository::new(connection), demo, zones)
        .current_week()
        .unwrap()
}

#[test]
fn demo_exercises_real_accounting() {
    let zones = utc_zones();
    for (scenario, credited, indicator) in [
        (DemoScenario::OnTrack, 360, IndicatorState::Green),
        (DemoScenario::Warning, 280, IndicatorState::Orange),
        (DemoScenario::Late, 240, IndicatorState::Red),
        (DemoScenario::Complete, 480, IndicatorState::Blue),
        (DemoScenario::Morning, 120, IndicatorState::Green),
        (DemoScenario::MorningWarning, 100, IndicatorState::Orange),
        (DemoScenario::MorningLate, 80, IndicatorState::Red),
    ] {
        let database = Database::new();
        let demo = DemoEnvironment::new(&zones, scenario);
        let accounting = utc_accounting(&database.connection, &demo, &demo, &zones);

        let summary = demo.summary(&accounting).unwrap();

        assert_eq!(
            (
                summary.credited_work(),
                summary.indicator_state(),
                summary.has_source_warning(),
                summary.credit_result().is_none(),
            ),
            (minutes(credited), indicator, false, true),
            "{scenario:?}"
        );
    }
}

#[test]
fn credit_demo_uses_real_tuesday_accounting_and_clears_when_switching_back_to_monday() {
    let zones = utc_zones();
    let database = Database::new();
    let demo = DemoEnvironment::new(&zones, DemoScenario::Credit);
    let accounting = utc_accounting(&database.connection, &demo, &demo, &zones);

    let credit = demo.summary(&accounting).unwrap();

    assert_eq!(credit.workday().date, date(2026, 9, 1));
    assert_eq!(credit.credited_work(), minutes(12));
    assert_eq!(credit.nominal_target(), hours(7));
    assert_eq!(credit.applied_carry_credit(), hours(1));
    assert_eq!(credit.effective_target(), hours(6));
    assert_eq!(
        credit.credit_result().unwrap().closing_balance(),
        SignedDuration::ZERO
    );
    assert!(!credit.has_source_warning());

    demo.set_scenario(DemoScenario::OnTrack);
    let on_track = demo.summary(&accounting).unwrap();

    assert_eq!(on_track.workday().date, date(2026, 8, 31));
    assert_eq!(on_track.credited_work(), hours(6));
    assert_eq!(on_track.effective_target(), hours(7));
    assert!(on_track.credit_result().is_none());
}

/// The accounting half of `SettingsRecalculateImmediatelyAndSurviveRestart`: a 7:24 target
/// moves the departure of the demo Monday to 17:24.
#[test]
fn changed_target_applies_to_the_demo_day() {
    let zones = utc_zones();
    let database = Database::new();
    let connection = &database.connection;
    let demo = DemoEnvironment::new(&zones, DemoScenario::OnTrack);
    let settings = || Settings {
        nominal_daily_target: minutes(444),
        ..Settings::default()
    };
    let accounting = WorkdayAccounting::new(
        &demo,
        SessionEventRepository::new(connection, &demo),
        ManualAdjustmentRepository::new(connection),
        &demo,
        &zones,
        &settings,
    );

    let published = accounting.calculate_current().unwrap();

    assert_eq!(published.credited_work(), hours(6));
    assert_eq!(published.effective_target(), minutes(444));
    assert_eq!(
        published
            .departure_projection()
            .unwrap()
            .projected_departure,
        utc_at(date(2026, 8, 31), 17, 24)
    );
}

#[test]
fn source_outage_and_recovery_do_not_invent_work() {
    let zones = utc_zones();
    let database = Database::new();
    let demo = DemoEnvironment::new(&zones, DemoScenario::Unavailable);
    let accounting = utc_accounting(&database.connection, &demo, &demo, &zones);

    let unavailable = accounting.calculate_current().unwrap();
    assert!(unavailable.has_source_warning());
    assert_eq!(unavailable.credited_work(), SignedDuration::ZERO);
    assert_eq!(
        unavailable.source_health().degraded_reason.as_deref(),
        Some("demo-offline")
    );

    demo.set_scenario(DemoScenario::OnTrack);
    let healthy = accounting.calculate_current().unwrap();
    assert!(!healthy.has_source_warning());
    assert_eq!(healthy.credited_work(), hours(6));
}

#[test]
fn demo_history_week_uses_real_persistence_and_does_not_overwrite_heads_on_reopen() {
    let zones = utc_zones();
    let database = Database::new();
    let connection = &database.connection;
    let demo = DemoEnvironment::new(&zones, DemoScenario::HistoryWeek);
    let accounting = utc_accounting(connection, &demo, &demo, &zones);
    let corrections =
        ManualAdjustmentCommands::new(&accounting, ManualAdjustmentRepository::new(connection));
    demo.seed(
        &SessionEventRepository::new(connection, &demo),
        &corrections,
    )
    .unwrap();
    let store = WeeklyCreditRepository::new(connection);
    let weekly = WeeklyRecalculation::new(&accounting, &store);

    let overview = demo.refresh(&accounting, &weekly, None).unwrap();
    let first = current_week(connection, &demo, &zones);
    let friday = first.days()[4].latest_revision.clone().unwrap();
    let friday = friday.snapshot();
    let summary = overview.summary.as_ref().unwrap();
    assert!(overview.source_failure.is_none());
    assert_eq!(
        stored(summary.effective_target()),
        friday.effective_target()
    );
    assert_eq!(summary.effective_target(), hours(7));
    assert_eq!(
        stored(summary.credit_result().unwrap().closing_balance()),
        friday.carry_available()
    );
    assert_eq!(stored(summary.credited_work()), friday.credited_work());

    let reopened_connection = database.reopen();
    seed(&reopened_connection, &demo, &zones);
    let reopened_accounting = utc_accounting(&reopened_connection, &demo, &demo, &zones);
    WeeklyRecalculation::new(
        &reopened_accounting,
        &WeeklyCreditRepository::new(&reopened_connection),
    )
    .calculate_current()
    .unwrap();
    let reopened = current_week(&reopened_connection, &demo, &zones);

    let snapshots: Vec<_> = first
        .days()
        .iter()
        .map(|day| day.latest_revision.as_ref().unwrap().snapshot())
        .collect();
    assert_eq!(snapshots.len(), 5);
    assert!(
        snapshots[0]
            .breakdown()
            .entries()
            .iter()
            .any(|entry| entry.disposition() == DailySummaryDisposition::ManualCredit)
    );
    for disposition in [
        DailySummaryDisposition::Unknown,
        DailySummaryDisposition::ProvisionalCredit,
    ] {
        assert!(snapshots.iter().any(|snapshot| {
            snapshot
                .breakdown()
                .entries()
                .iter()
                .any(|entry| entry.disposition() == disposition)
        }));
    }
    assert!(
        snapshots
            .iter()
            .any(|snapshot| snapshot.has_pending_review())
    );
    let heads = |week: &CurrentWeek| -> Vec<(i64, String)> {
        week.days()
            .iter()
            .map(|day| {
                let revision = day.latest_revision.as_ref().unwrap();
                (
                    revision.revision(),
                    revision.snapshot().input_fingerprint().to_owned(),
                )
            })
            .collect()
    };
    assert_eq!(heads(&reopened), heads(&first));

    demo.set_scenario(DemoScenario::OnTrack);
    let on_track = demo.refresh(&accounting, &weekly, Some(&overview)).unwrap();
    let on_track = on_track.summary.unwrap();
    assert!(on_track.credit_result().is_none());
    assert_eq!(on_track.effective_target(), hours(7));

    demo.set_scenario(DemoScenario::HistoryWeek);
    let restored = demo.refresh(&accounting, &weekly, None).unwrap();
    let restored = restored.summary.unwrap();
    assert_eq!(
        stored(restored.effective_target()),
        friday.effective_target()
    );
    assert_eq!(
        stored(restored.credit_result().unwrap().closing_balance()),
        friday.carry_available()
    );

    let edit = ManualAdjustment::duration_work(
        AdjustmentId::new("demo-history-edited").unwrap(),
        LocalWorkday::new(date(2026, 8, 31), "UTC"),
        hours(1),
        demo.now() + TICK,
    )
    .unwrap()
    .superseding(AdjustmentId::new("demo-history-week-manual").unwrap());
    corrections.save(&edit).unwrap();
    let corrected = demo.refresh(&accounting, &weekly, None).unwrap();
    let corrected = corrected.summary.unwrap();
    let corrected_week = current_week(connection, &demo, &zones);
    let corrected_friday = corrected_week.days()[4].latest_revision.as_ref().unwrap();
    assert_eq!(
        stored(corrected.effective_target()),
        friday.effective_target() - minutes(30)
    );
    assert_eq!(
        stored(corrected.credit_result().unwrap().closing_balance()),
        corrected_friday.snapshot().carry_available()
    );
    assert_eq!(
        stored(corrected.effective_target()),
        corrected_friday.snapshot().effective_target()
    );
}

#[test]
fn credit_scenario_calculates_after_the_history_week_was_seeded() {
    let zones = utc_zones();
    let database = Database::new();
    let connection = &database.connection;
    let demo = DemoEnvironment::new(&zones, DemoScenario::HistoryWeek);
    seed(connection, &demo, &zones);
    let accounting = utc_accounting(connection, &demo, &demo, &zones);
    let store = WeeklyCreditRepository::new(connection);
    let weekly = WeeklyRecalculation::new(&accounting, &store);
    weekly.calculate_current().unwrap();

    demo.set_scenario(DemoScenario::Credit);
    let credit = demo.refresh(&accounting, &weekly, None).unwrap();

    assert!(credit.source_failure.is_none());
    let summary = credit.summary.unwrap();
    assert_eq!(summary.workday().date, date(2026, 9, 1));
    assert_eq!(summary.applied_carry_credit(), hours(1));
    assert_eq!(summary.credited_work(), minutes(12 + 45));
}

/// The breakdown under the screenshot timeline of the .NET test. Each day's timeline is its
/// entries with an interval, without the leading and trailing pauses, the end at midnight shown
/// as 00:00 here: Monday "Travail
/// 09:00–17:30"; Tuesday "Travail 08:15–12:15, Pause 12:15–13:00, Travail 13:00–15:15,
/// Correction manuelle 15:15–16:00"; Wednesday "Travail 09:00–10:00, À vérifier 10:00–12:00,
/// Travail 12:00–14:00, Trou 14:00–14:15, Travail 14:15–16:45"; Thursday "Travail 09:00–11:00,
/// Veille 11:00–11:40, Travail 11:40–15:30"; Friday "Travail 09:00–14:00, À vérifier
/// 14:00–18:00".
#[test]
fn demo_history_week_draws_every_kind_of_segment_for_the_screenshots() {
    let zones = utc_zones();
    let database = Database::new();
    let connection = &database.connection;
    let demo = DemoEnvironment::new(&zones, DemoScenario::HistoryWeek);
    seed(connection, &demo, &zones);
    let accounting = utc_accounting(connection, &demo, &demo, &zones);
    WeeklyRecalculation::new(&accounting, &WeeklyCreditRepository::new(connection))
        .calculate_current()
        .unwrap();

    let week = current_week(connection, &demo, &zones);

    let time = |instant: Timestamp| {
        let local = instant.to_zoned(TimeZone::UTC);
        format!("{:02}:{:02}", local.hour(), local.minute())
    };
    let days: Vec<Vec<String>> = week
        .days()
        .iter()
        .map(|day| {
            let snapshot = day.latest_revision.as_ref().unwrap().snapshot();
            snapshot
                .breakdown()
                .entries()
                .iter()
                .map(|entry| {
                    let span = match entry.interval() {
                        Some(interval) => {
                            format!("{}-{}", time(interval.start()), time(interval.end()))
                        }
                        None => format!("{}m", entry.duration().as_mins()),
                    };
                    let mark = match (entry.adjustment_id(), entry.review_id()) {
                        (Some(id), _) => format!(" {id}"),
                        (None, Some(_)) => " review".to_owned(),
                        (None, None) => String::new(),
                    };
                    format!("{:?} {:?} {span}{mark}", entry.disposition(), entry.label())
                })
                .collect()
        })
        .collect();

    assert_eq!(
        days,
        [
            &[
                "Excluded Afk 00:00-09:00",
                "ConfirmedCredit ActiveWork 09:00-17:30",
                "Excluded Afk 17:30-00:00",
                "ManualCredit Manual 30m demo-history-week-manual",
            ][..],
            &[
                "Excluded Afk 00:00-08:15",
                "ConfirmedCredit ActiveWork 08:15-12:15",
                "Excluded LunchLock 12:15-13:00",
                "ConfirmedCredit ActiveWork 13:00-15:15",
                "ConfirmedCredit Manual 15:15-16:00 demo-history-week-meeting",
                "Excluded Afk 16:00-00:00",
            ],
            &[
                "Excluded Afk 00:00-09:00",
                "ConfirmedCredit ActiveWork 09:00-10:00",
                "ProvisionalCredit LockedWork 10:00-12:00 review",
                "ConfirmedCredit ActiveWork 12:00-14:00",
                "Unknown Unknown 14:00-14:15",
                "ConfirmedCredit ActiveWork 14:15-16:45",
                "Excluded Afk 16:45-00:00",
            ],
            &[
                "Excluded Afk 00:00-09:00",
                "ConfirmedCredit ActiveWork 09:00-11:00",
                "Excluded Suspended 11:00-11:40",
                "ConfirmedCredit ActiveWork 11:40-15:30",
                "Excluded Afk 15:30-00:00",
            ],
            &[
                "Excluded Afk 00:00-09:00",
                "ConfirmedCredit ActiveWork 09:00-14:00",
                "ProvisionalCredit LockedWork 14:00-18:00 review",
            ],
        ]
    );
    assert_eq!(
        week.days()
            .iter()
            .map(|day| day
                .latest_revision
                .as_ref()
                .unwrap()
                .snapshot()
                .has_pending_review())
            .collect::<Vec<_>>(),
        [false, false, true, false, true]
    );
    assert_eq!(count(connection, "DailySummaryRevision"), 5);
}

/// The scenarios with stored days refresh through the weekly recalculation, which stores today
/// with the seeded sleep in it.
#[test]
fn problem_periods_and_fragmented_day_store_today_with_their_seeded_sleep() {
    let zones = utc_zones();
    let monday = date(2026, 8, 31);
    let at = |hour, minute, second| {
        monday
            .at(hour, minute, second, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp()
    };
    for (scenario, sleeps) in [
        (
            DemoScenario::ProblemPeriods,
            vec![(at(10, 0, 0), at(10, 30, 0))],
        ),
        (
            DemoScenario::FragmentedDay,
            vec![(at(0, 0, 0), at(8, 27, 8)), (at(10, 45, 0), at(10, 45, 20))],
        ),
    ] {
        let database = Database::new();
        let connection = &database.connection;
        let demo = DemoEnvironment::new(&zones, scenario);
        seed(connection, &demo, &zones);
        let accounting = utc_accounting(connection, &demo, &demo, &zones);
        let store = WeeklyCreditRepository::new(connection);
        let weekly = WeeklyRecalculation::new(&accounting, &store);

        let refreshed = demo.refresh(&accounting, &weekly, None).unwrap();

        let week = current_week(connection, &demo, &zones);
        let today = week.days()[0].latest_revision.as_ref().unwrap().snapshot();
        assert_eq!(
            stored(refreshed.summary.unwrap().credited_work()),
            today.credited_work(),
            "{scenario:?}"
        );
        let suspended: Vec<_> = today
            .breakdown()
            .entries()
            .iter()
            .filter(|entry| entry.label() == bientot_domain::accounting::AccountingLabel::Suspended)
            .filter_map(|entry| entry.interval())
            .map(|interval| (interval.start(), interval.end()))
            .collect();
        assert_eq!(suspended, sleeps, "{scenario:?}");
    }
}

#[test]
fn seeding_again_records_nothing_more() {
    let zones = utc_zones();
    for scenario in [
        DemoScenario::HistoryWeek,
        DemoScenario::ProblemPeriods,
        DemoScenario::FragmentedDay,
        DemoScenario::OnTrack,
    ] {
        let database = Database::new();
        let connection = &database.connection;
        let demo = DemoEnvironment::new(&zones, scenario);

        seed(connection, &demo, &zones);
        let seeded = (
            count(connection, "SessionEvent"),
            count(connection, "ManualAdjustment"),
        );
        seed(connection, &demo, &zones);

        assert_eq!(
            (
                count(connection, "SessionEvent"),
                count(connection, "ManualAdjustment"),
            ),
            seeded,
            "{scenario:?}"
        );
        let expected = match scenario {
            DemoScenario::HistoryWeek => (7, 2),
            DemoScenario::ProblemPeriods => (2, 0),
            DemoScenario::FragmentedDay => (4, 0),
            _ => (0, 0),
        };
        assert_eq!(seeded, expected, "{scenario:?}");
    }
}

#[test]
fn scenarios_keep_the_dotnet_picker_order_and_resource_keys_before_the_rust_only_one() {
    assert_eq!(
        DemoScenario::ALL.map(DemoScenario::resource_key),
        [
            "DemoOnTrack",
            "DemoWarning",
            "DemoLate",
            "DemoComplete",
            "DemoMorning",
            "DemoUnavailable",
            "DemoCredit",
            "DemoHistoryWeek",
            "DemoMorningWarning",
            "DemoMorningLate",
            "DemoProblemPeriods",
            "DemoFragmentedDay",
            "DemoUnavailable",
        ]
    );
}
