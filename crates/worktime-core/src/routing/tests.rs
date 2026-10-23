//! Ported from `ActivitySourceRouterTests.cs` and `Settings/ActivitySourceSwitchTests.cs`, against
//! a migrated SQLite ledger and a `settings.json` in a temporary directory.
//!
//! ActivityWatch is not ported, so where .NET read the ActivityWatch server the time selected for
//! it reads as unavailable, and the tests on its bucket choice or its retained snapshots check the
//! same routing rule with the built-in source. Left out: `OnlyAChangeToActivityWatchWaitsForA
//! ConnectionTest` (the connection test is ActivityWatch's) and `ActivityWatchPickedWhileASwitch
//! ToBuiltInIsStillSavingWaitsForAConnectionTest` (the settings workspace is R6's). The
//! recalculation a saved switch triggers is the settings service's, also R6.

use std::cell::RefCell;

use jiff::civil::{Date, date};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span};
use tempfile::TempDir;
use worktime_domain::accounting::{ActivitySegment, ActivityState, AutomaticOrigin};
use worktime_domain::configuration::BuiltInActivitySettings;
use worktime_domain::sources::SourceIntervalKind;
use worktime_domain::time::FrozenClock;

use super::*;
use crate::sources::normalize;
use crate::test_support::Database;

const MONDAY: Date = date(2026, 10, 12);
const BUILT_IN: &str = "built-in";

fn at(day: i64, hour: i8, minute: i8) -> Timestamp {
    MONDAY
        .checked_add(Span::new().days(day))
        .unwrap()
        .at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn interval(start: Timestamp, end: Timestamp) -> UtcInterval {
    UtcInterval::new(start, end).unwrap()
}

fn selection(effective_from: Timestamp, source: ActivitySourceKind) -> ActivitySourceSelection {
    ActivitySourceSelection {
        effective_from,
        source,
    }
}

fn built_in_settings() -> WorkTimeSettings {
    WorkTimeSettings {
        activity_source: Some(ActivitySourceKind::BuiltIn),
        ..WorkTimeSettings::default()
    }
}

/// Active throughout each read, unless `result` says otherwise, and records what it was asked.
struct RecordingSource {
    requests: RefCell<Vec<UtcInterval>>,
    result: RefCell<Option<ActivityReadResult>>,
}

impl ActivitySource for RecordingSource {
    fn read(&self, interval: UtcInterval) -> Result<ActivityReadResult, ActivityError> {
        self.requests.borrow_mut().push(interval);
        Ok(self
            .result
            .borrow()
            .clone()
            .unwrap_or_else(|| ActivityReadResult::Snapshot {
                coverage: ActivityCoverage::new(
                    interval,
                    [ActivitySegment::new(
                        interval,
                        ActivityState::Active,
                        AutomaticOrigin::new(BUILT_IN).unwrap(),
                    )],
                ),
                health: SourceHealth::healthy(BUILT_IN, interval.end()),
            }))
    }
}

struct Harness {
    database: Database,
    settings_directory: TempDir,
    clock: FrozenClock,
    built_in: RecordingSource,
}

impl Harness {
    fn new() -> Self {
        Self {
            database: Database::new(),
            settings_directory: TempDir::new().unwrap(),
            clock: FrozenClock::new(at(0, 9, 0)),
            built_in: RecordingSource {
                requests: RefCell::default(),
                result: RefCell::default(),
            },
        }
    }

    fn selections(&self) -> ActivitySourceSelectionRepository<'_> {
        ActivitySourceSelectionRepository::new(&self.database.connection, &self.clock)
    }

    fn settings_store(&self) -> SettingsStore {
        SettingsStore::new(self.settings_directory.path().join("settings.json")).unwrap()
    }

    /// The settings saved last, or `None` when none were.
    fn saved_settings(&self) -> Option<WorkTimeSettings> {
        let loaded = self.settings_store().load().unwrap();
        (loaded.source == worktime_store::settings::SettingsSource::File).then_some(loaded.settings)
    }

    fn forget_saved_settings(&self) {
        std::fs::remove_file(self.settings_store().path()).unwrap();
    }

    /// Makes every save fail to write, as a locked file would.
    fn fail_settings_saves(&self) {
        std::fs::create_dir(self.settings_store().path()).unwrap();
    }

    fn start(&self, saved: WorkTimeSettings, now: Timestamp) -> WorkTimeSettings {
        self.clock.set(now);
        record_startup_selection(
            saved,
            &self.selections(),
            &self.settings_store(),
            &self.clock,
        )
        .unwrap()
    }

    fn timeline(&self) -> Vec<ActivitySourceSelection> {
        self.selections()
            .read_timeline(ActivitySourceSelection::LEGACY_EPOCH, None)
            .unwrap()
    }

    fn read(&self, start: Timestamp, end: Timestamp) -> ActivityReadResult {
        ActivitySourceRouter::new(self.selections(), &self.built_in)
            .read(interval(start, end))
            .unwrap()
    }

    fn requests(&self) -> Vec<(Timestamp, Timestamp)> {
        self.built_in
            .requests
            .borrow()
            .iter()
            .map(|request| (request.start(), request.end()))
            .collect()
    }

    fn switch(
        &self,
        current: &WorkTimeSettings,
        source: ActivitySourceKind,
        also_apply: impl FnOnce(WorkTimeSettings) -> WorkTimeSettings,
    ) -> Result<(), SwitchError<SaveSettingsError>> {
        let store = self.settings_store();
        ActivitySourceSwitch::new(self.selections(), &self.clock).select(
            current,
            source,
            also_apply,
            |settings| store.save(&settings),
        )
    }
}

fn coverage(result: &ActivityReadResult) -> &ActivityCoverage {
    match result {
        ActivityReadResult::Snapshot { coverage, .. } => coverage,
        ActivityReadResult::Unavailable { .. } => panic!("expected a snapshot, got {result:?}"),
    }
}

fn segments(result: &ActivityReadResult) -> Vec<(Timestamp, Timestamp, &str)> {
    coverage(result)
        .segments()
        .iter()
        .map(|segment| {
            (
                segment.interval.start(),
                segment.interval.end(),
                segment.origin.source_id(),
            )
        })
        .collect()
}

fn states(coverage: &ActivityCoverage) -> Vec<(Timestamp, Timestamp, ActivityState, &str)> {
    coverage
        .segments()
        .iter()
        .map(|segment| {
            (
                segment.interval.start(),
                segment.interval.end(),
                segment.state,
                segment.origin.source_id(),
            )
        })
        .collect()
}

fn unavailable(source_id: &str, reason: &str) -> ActivityReadResult {
    ActivityReadResult::Unavailable {
        health: SourceHealth::degraded(
            source_id,
            SourceFreshness::Unavailable,
            SourceQuality::Incomplete,
            reason,
            None,
        )
        .unwrap(),
    }
}

#[test]
fn a_fresh_built_in_install_reads_nothing_before_onboarding_confirmation() {
    let harness = Harness::new();
    harness
        .selections()
        .append(selection(at(0, 10, 30), ActivitySourceKind::BuiltIn))
        .unwrap();
    let confirmed = built_in_settings();

    assert_eq!(harness.start(confirmed.clone(), at(0, 9, 0)), confirmed);
    let day = harness.read(at(0, 8, 0), at(0, 18, 0));
    let week = harness.read(at(0, 0, 0), at(5, 0, 0));
    let before_confirmation = harness.read(at(0, 8, 0), at(0, 10, 30));

    assert_eq!(segments(&day), [(at(0, 10, 30), at(0, 18, 0), BUILT_IN)]);
    assert_eq!(segments(&week), [(at(0, 10, 30), at(5, 0, 0), BUILT_IN)]);
    assert_eq!(
        before_confirmation,
        unavailable(NO_SOURCE_ID, "no-activity-source-selected")
    );
    assert_eq!(
        harness.requests(),
        [(at(0, 10, 30), at(0, 18, 0)), (at(0, 10, 30), at(5, 0, 0))]
    );
    assert_eq!(harness.saved_settings(), None);
}

/// .NET reads ActivityWatch before the upgrade; here that time reads as nothing, and the read
/// reports the built-in source's health.
#[test]
fn a_legacy_install_reads_built_in_after_the_upgrade_and_the_seed_is_a_no_op_on_a_second_start() {
    let harness = Harness::new();
    let first_start = harness.start(WorkTimeSettings::default(), at(0, 9, 0));
    let saved_by_first_start = harness.saved_settings();
    harness
        .selections()
        .append(selection(at(0, 14, 0), ActivitySourceKind::BuiltIn))
        .unwrap();
    harness.forget_saved_settings();

    let upgraded = WorkTimeSettings {
        activity_source: Some(ActivitySourceKind::BuiltIn),
        ..first_start.clone()
    };
    let second_start = harness.start(upgraded.clone(), at(0, 15, 0));
    let day = harness.read(at(0, 8, 0), at(0, 18, 0));
    let week = harness.read(at(0, 0, 0), at(5, 0, 0));

    assert_eq!(segments(&day), [(at(0, 14, 0), at(0, 18, 0), BUILT_IN)]);
    assert_eq!(day.health(), &SourceHealth::healthy(BUILT_IN, at(0, 18, 0)));
    assert_eq!(segments(&week), [(at(0, 14, 0), at(5, 0, 0), BUILT_IN)]);
    assert_eq!(
        harness.requests(),
        [(at(0, 14, 0), at(0, 18, 0)), (at(0, 14, 0), at(5, 0, 0))]
    );
    assert_eq!(
        first_start.activity_source,
        Some(ActivitySourceKind::ActivityWatch)
    );
    assert_eq!(saved_by_first_start, Some(first_start));
    assert_eq!(second_start, upgraded);
    assert_eq!(harness.saved_settings(), None);
    assert_eq!(
        harness.timeline(),
        [
            selection(
                ActivitySourceSelection::LEGACY_EPOCH,
                ActivitySourceKind::ActivityWatch
            ),
            selection(at(0, 14, 0), ActivitySourceKind::BuiltIn),
        ]
    );
}

/// .NET then reads the day from ActivityWatch; here the whole day is unavailable, so its stored
/// summary is kept.
#[test]
fn a_legacy_start_whose_settings_save_failed_seeds_once_and_still_selects_activity_watch() {
    let harness = Harness::new();
    harness.fail_settings_saves();

    let first_start = harness.start(WorkTimeSettings::default(), at(0, 9, 0));
    let second_start = harness.start(WorkTimeSettings::default(), at(0, 9, 0));
    let day = harness.read(at(0, 8, 0), at(0, 18, 0));

    assert_eq!(
        first_start.activity_source,
        Some(ActivitySourceKind::ActivityWatch)
    );
    assert_eq!(
        second_start.activity_source,
        Some(ActivitySourceKind::ActivityWatch)
    );
    assert_eq!(
        harness.timeline(),
        [selection(
            ActivitySourceSelection::LEGACY_EPOCH,
            ActivitySourceKind::ActivityWatch
        )]
    );
    assert_eq!(
        day,
        unavailable("activitywatch", "activitywatch-not-supported")
    );
    assert_eq!(harness.requests(), []);
}

#[test]
fn settings_without_a_source_after_an_interrupted_onboarding_mirror_the_recorded_selection() {
    let harness = Harness::new();
    harness
        .selections()
        .append(selection(at(0, 10, 30), ActivitySourceKind::BuiltIn))
        .unwrap();

    let started = harness.start(WorkTimeSettings::default(), at(0, 9, 0));
    harness.read(at(0, 8, 0), at(0, 18, 0));

    assert_eq!(started, built_in_settings());
    assert_eq!(harness.saved_settings(), Some(started));
    assert_eq!(
        harness.timeline(),
        [selection(at(0, 10, 30), ActivitySourceKind::BuiltIn)]
    );
    assert_eq!(harness.requests(), [(at(0, 10, 30), at(0, 18, 0))]);
}

#[test]
fn settings_that_name_a_source_over_an_empty_ledger_record_it_from_that_start() {
    let harness = Harness::new();
    let saved = built_in_settings();

    let first_start = harness.start(saved.clone(), at(0, 11, 0));
    let second_start = harness.start(saved.clone(), at(0, 15, 0));
    let day = harness.read(at(0, 11, 0), at(0, 18, 0));

    assert_eq!(first_start, saved);
    assert_eq!(second_start, saved);
    assert_eq!(
        harness.timeline(),
        [selection(at(0, 11, 0), ActivitySourceKind::BuiltIn)]
    );
    assert_eq!(segments(&day), [(at(0, 11, 0), at(0, 18, 0), BUILT_IN)]);
    assert_eq!(harness.saved_settings(), None);
}

/// Not in .NET, whose clock has no precision below the tick the ledger keeps: a selection
/// recorded at a sub-tick instant is stored truncated, and still answers a read from that instant.
#[test]
fn a_selection_recorded_between_ticks_answers_a_read_from_that_instant() {
    let harness = Harness::new();
    let started_at = at(0, 11, 0) + SignedDuration::from_nanos(50);

    harness.start(built_in_settings(), started_at);
    let day = harness.read(started_at, at(0, 18, 0));

    assert_eq!(
        harness.timeline(),
        [selection(at(0, 11, 0), ActivitySourceKind::BuiltIn)]
    );
    assert_eq!(segments(&day), [(started_at, at(0, 18, 0), BUILT_IN)]);
    assert_eq!(harness.requests(), [(started_at, at(0, 18, 0))]);
}

/// .NET passes an ActivityWatch bucket choice through; the built-in source's degraded read is
/// passed through the same way.
#[test]
fn a_read_within_one_selection_is_passed_through_unchanged() {
    let harness = Harness::new();
    harness
        .selections()
        .append(selection(at(0, 8, 0), ActivitySourceKind::BuiltIn))
        .unwrap();
    let outage = unavailable(BUILT_IN, "test outage");
    *harness.built_in.result.borrow_mut() = Some(outage.clone());

    let result = harness.read(at(0, 8, 0), at(0, 9, 0));

    assert_eq!(result, outage);
    assert_eq!(harness.requests(), [(at(0, 8, 0), at(0, 9, 0))]);
}

#[test]
fn reselecting_the_same_source_does_not_split_the_read() {
    let harness = Harness::new();
    let selections = harness.selections();
    selections
        .append(selection(at(0, 8, 0), ActivitySourceKind::BuiltIn))
        .unwrap();
    selections
        .append(selection(at(0, 12, 0), ActivitySourceKind::BuiltIn))
        .unwrap();

    harness.read(at(0, 9, 0), at(0, 18, 0));

    assert_eq!(harness.requests(), [(at(0, 9, 0), at(0, 18, 0))]);
}

/// .NET checks a retained ActivityWatch snapshot; here the built-in source answers for more than
/// its part, and only its part is kept.
#[test]
fn a_snapshot_wider_than_its_part_stays_inside_the_period_its_source_was_selected() {
    let harness = Harness::new();
    harness.selections().append_legacy_selection().unwrap();
    harness
        .selections()
        .append(selection(at(0, 14, 0), ActivitySourceKind::BuiltIn))
        .unwrap();
    let whole_day = interval(at(0, 8, 0), at(0, 18, 0));
    *harness.built_in.result.borrow_mut() = Some(ActivityReadResult::Snapshot {
        coverage: ActivityCoverage::new(
            whole_day,
            [ActivitySegment::new(
                whole_day,
                ActivityState::Afk,
                AutomaticOrigin::new(BUILT_IN).unwrap(),
            )],
        ),
        health: SourceHealth::healthy(BUILT_IN, at(0, 18, 0)),
    });

    let day = harness.read(at(0, 8, 0), at(0, 18, 0));

    assert_eq!(
        states(coverage(&day)),
        [(at(0, 14, 0), at(0, 18, 0), ActivityState::Afk, BUILT_IN)]
    );
}

/// .NET checks an ActivityWatch bucket choice for the later part; here ActivityWatch is
/// unavailable for it, which answers for the read's health and leaves its part unresolved.
#[test]
fn a_later_activity_watch_selection_keeps_the_earlier_sources_evidence() {
    let harness = Harness::new();
    let selections = harness.selections();
    selections
        .append(selection(at(0, 8, 0), ActivitySourceKind::BuiltIn))
        .unwrap();
    selections
        .append(selection(at(0, 14, 0), ActivitySourceKind::ActivityWatch))
        .unwrap();
    let bounds = interval(at(0, 8, 0), at(0, 18, 0));

    let result = harness.read(bounds.start(), bounds.end());
    let normalized = normalize(
        bounds,
        at(0, 18, 0),
        &result,
        BuiltInActivitySettings::default().pending_horizon(),
    );

    assert_eq!(
        result.health(),
        unavailable("activitywatch", "activitywatch-not-supported").health()
    );
    assert_eq!(
        states(&normalized.coverage),
        [
            (at(0, 8, 0), at(0, 14, 0), ActivityState::Active, BUILT_IN),
            (
                at(0, 14, 0),
                at(0, 18, 0),
                ActivityState::Unknown,
                "activitywatch:coverage-gap"
            ),
        ]
    );
    let unresolved: Vec<_> = normalized
        .diagnostics
        .intervals()
        .iter()
        .map(|finding| (finding.interval(), finding.kind()))
        .collect();
    assert_eq!(
        unresolved,
        [(
            interval(at(0, 14, 0), at(0, 18, 0)),
            SourceIntervalKind::UnresolvedCoverage
        )]
    );
}

fn on_activity_watch() -> WorkTimeSettings {
    WorkTimeSettings {
        activity_source: Some(ActivitySourceKind::ActivityWatch),
        ..WorkTimeSettings::default()
    }
}

#[test]
fn switching_records_the_new_source_from_now_so_the_day_is_split_there() {
    let harness = Harness::new();
    harness.selections().append_legacy_selection().unwrap();
    harness.clock.set(at(0, 14, 0));

    harness
        .switch(
            &on_activity_watch(),
            ActivitySourceKind::BuiltIn,
            |current| current,
        )
        .unwrap();
    let day = harness.read(at(0, 8, 0), at(0, 18, 0));

    assert_eq!(
        harness.saved_settings().unwrap().activity_source,
        Some(ActivitySourceKind::BuiltIn)
    );
    assert_eq!(
        harness.timeline(),
        [
            selection(
                ActivitySourceSelection::LEGACY_EPOCH,
                ActivitySourceKind::ActivityWatch
            ),
            selection(at(0, 14, 0), ActivitySourceKind::BuiltIn),
        ]
    );
    assert_eq!(segments(&day), [(at(0, 14, 0), at(0, 18, 0), BUILT_IN)]);
}

#[test]
fn the_source_already_selected_records_nothing_and_saves_only_the_rest_of_the_change() {
    let harness = Harness::new();
    let current = on_activity_watch();

    harness
        .switch(
            &current,
            ActivitySourceKind::ActivityWatch,
            |mut current| {
                current.activity_watch.base_url = "http://127.0.0.1:5700".to_owned();
                current
            },
        )
        .unwrap();

    assert_eq!(harness.timeline(), []);
    let saved = harness.saved_settings().unwrap();
    assert_eq!(saved.activity_watch.base_url, "http://127.0.0.1:5700");
    assert_eq!(
        saved.activity_source,
        Some(ActivitySourceKind::ActivityWatch)
    );
}

#[test]
fn a_failed_settings_write_puts_the_ledger_back_on_the_source_the_settings_still_name() {
    let harness = Harness::new();
    harness.fail_settings_saves();
    harness.clock.set(at(0, 14, 0));

    let error = harness
        .switch(
            &on_activity_watch(),
            ActivitySourceKind::BuiltIn,
            |current| current,
        )
        .unwrap_err();

    assert!(
        matches!(error, SwitchError::Save(SaveSettingsError::Io(_))),
        "{error:?}"
    );
    assert_eq!(
        harness.timeline(),
        [
            selection(at(0, 14, 0), ActivitySourceKind::BuiltIn),
            selection(at(0, 14, 0), ActivitySourceKind::ActivityWatch),
        ]
    );
}

#[test]
fn an_invalid_change_is_rejected_before_the_ledger_changes() {
    let harness = Harness::new();

    let error = harness
        .switch(
            &on_activity_watch(),
            ActivitySourceKind::BuiltIn,
            |current| WorkTimeSettings {
                built_in_activity: BuiltInActivitySettings {
                    idle_threshold: SignedDuration::from_secs(10),
                },
                ..current
            },
        )
        .unwrap_err();

    assert!(
        matches!(&error, SwitchError::InvalidSettings(errors) if errors == &[SettingsError::IdleThreshold]),
        "{error:?}"
    );
    assert_eq!(error.to_string(), "Invalid workday settings.");
    assert_eq!(harness.timeline(), []);
    assert_eq!(harness.saved_settings(), None);
}

#[test]
fn a_settings_file_without_a_source_reads_activity_watch() {
    assert_eq!(
        selected(&WorkTimeSettings::default()),
        ActivitySourceKind::ActivityWatch
    );
    assert_eq!(selected(&built_in_settings()), ActivitySourceKind::BuiltIn);
}
