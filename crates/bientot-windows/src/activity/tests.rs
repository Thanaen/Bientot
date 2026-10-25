use std::cell::Cell;

use bientot_domain::time::FrozenClock;
use jiff::civil::date;
use rusqlite::Connection;
use tempfile::TempDir;

use super::*;

fn day() -> Timestamp {
    date(2026, 10, 12)
        .at(0, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn at(hour: i64, minute: i64, second: i64) -> Timestamp {
    day() + SignedDuration::from_secs(hour * 3600 + minute * 60 + second)
}

fn seconds(seconds: i64) -> SignedDuration {
    SignedDuration::from_secs(seconds)
}

/// A `GetLastInputInfo` that wraps its 32-bit tick count during the test day.
struct FakeLastInput<'a> {
    clock: &'a FrozenClock,
    last_input_tick: Cell<u32>,
    fails: Cell<bool>,
}

impl<'a> FakeLastInput<'a> {
    const TICK_AT_MIDNIGHT: u32 = u32::MAX - 30_000_000;

    fn new(clock: &'a FrozenClock) -> Self {
        Self {
            clock,
            last_input_tick: Cell::new(0),
            fails: Cell::new(false),
        }
    }

    fn input(&self, at: Timestamp) {
        self.last_input_tick.set(Self::tick_at(at));
    }

    fn tick_at(at: Timestamp) -> u32 {
        let milliseconds = at.duration_since(day()).as_millis() as u32;
        Self::TICK_AT_MIDNIGHT.wrapping_add(milliseconds)
    }
}

impl LastInputProbe for FakeLastInput<'_> {
    fn read(&self) -> Option<LastInputReading> {
        (!self.fails.get()).then(|| LastInputReading {
            tick_count: Self::tick_at(self.clock.now()),
            last_input_tick: self.last_input_tick.get(),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Segment(Timestamp, Timestamp, ActivityState);

fn segments(snapshot: &ActivitySnapshot) -> Vec<Segment> {
    snapshot
        .coverage
        .segments()
        .iter()
        .map(|segment| {
            Segment(
                segment.interval.start(),
                segment.interval.end(),
                segment.state,
            )
        })
        .collect()
}

fn active(start: Timestamp, end: Timestamp) -> Segment {
    Segment(start, end, ActivityState::Active)
}

fn built_in() -> Settings {
    Settings {
        activity_source: Some(ActivitySourceKind::BuiltIn),
        ..Settings::default()
    }
}

struct Database {
    _directory: TempDir,
    connection: Connection,
}

impl Database {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let connection =
            bientot_store::database::open(&directory.path().join("worktime.db")).unwrap();
        bientot_store::database::migrate(&connection).unwrap();
        Self {
            _directory: directory,
            connection,
        }
    }
}

/// The source with its ledger, fake input and clock. [`Harness::restart`] replaces the source
/// without ending observation, as a crash or a new process would.
struct Harness<'a> {
    clock: &'a FrozenClock,
    input: &'a FakeLastInput<'a>,
    database: &'a Database,
    source: BuiltInActivitySource<'a>,
}

impl<'a> Harness<'a> {
    fn new(clock: &'a FrozenClock, input: &'a FakeLastInput<'a>, database: &'a Database) -> Self {
        Self {
            clock,
            input,
            database,
            source: Self::source(clock, input, database),
        }
    }

    fn source(
        clock: &'a FrozenClock,
        input: &'a FakeLastInput<'a>,
        database: &'a Database,
    ) -> BuiltInActivitySource<'a> {
        BuiltInActivitySource::new(
            input,
            ActivitySpanRepository::new(&database.connection, clock),
            clock,
        )
    }

    fn restart(&mut self) {
        self.source = Self::source(self.clock, self.input, self.database);
    }

    fn input(&self, at: Timestamp) {
        self.input.input(at);
    }

    fn sample_every_5_seconds(&mut self, from: Timestamp, through: Timestamp) {
        let mut at = from;
        while at <= through {
            self.clock.set(at);
            self.source.sample().unwrap();
            at += seconds(5);
        }
    }

    fn poll_every_5_seconds(&mut self, settings: &Settings, from: Timestamp, through: Timestamp) {
        let mut at = from;
        while at <= through {
            self.clock.set(at);
            self.source.poll(settings).unwrap();
            at += seconds(5);
        }
    }

    fn read_result(&self, start: Timestamp, end: Timestamp) -> ActivitySnapshot {
        self.source
            .read(UtcInterval::new(start, end).unwrap())
            .unwrap()
    }

    fn read(&self, start: Timestamp, end: Timestamp) -> Vec<Segment> {
        segments(&self.read_result(start, end))
    }
}

macro_rules! harness {
    ($harness:ident) => {
        let clock = FrozenClock::new(day());
        let input = FakeLastInput::new(&clock);
        let database = Database::new();
        #[allow(unused_mut)]
        let mut $harness = Harness::new(&clock, &input, &database);
    };
}

#[test]
fn clean_stop_with_a_pending_tail_credits_activity_only_through_the_last_input_hours_later_and_after_a_threshold_change()
 {
    harness!(harness);
    harness.input(at(9, 57, 0));
    harness.sample_every_5_seconds(at(9, 57, 0), at(9, 59, 25));
    harness.input(at(9, 59, 30));
    harness.sample_every_5_seconds(at(9, 59, 30), at(9, 59, 55));
    harness.input(at(10, 0, 0));
    harness.sample_every_5_seconds(at(10, 0, 0), at(10, 1, 0));

    harness.source.end_observation().unwrap();
    harness.restart();
    harness.clock.set(at(14, 0, 0));
    let hours_later = harness.read(at(9, 0, 0), at(12, 0, 0));
    harness.source.set_idle_threshold(seconds(30)).unwrap();
    let after_threshold_change = harness.read(at(9, 0, 0), at(12, 0, 0));

    let expected = vec![active(at(9, 57, 0), at(10, 0, 0))];
    assert_eq!(hours_later, expected);
    assert_eq!(after_threshold_change, expected);
}

#[test]
fn crash_with_a_pending_tail_then_restart_credits_through_the_last_stored_boundary_and_leaves_the_gap_unknown()
 {
    harness!(harness);
    harness.input(at(9, 55, 0));
    harness.sample_every_5_seconds(at(9, 55, 0), at(9, 57, 55));
    harness.input(at(9, 58, 0));
    harness.sample_every_5_seconds(at(9, 58, 0), at(9, 58, 45));
    harness.input(at(9, 58, 50));
    harness.sample_every_5_seconds(at(9, 58, 50), at(10, 0, 0));
    let before_crash = harness.read(at(9, 0, 0), at(10, 0, 0));

    harness.restart();
    harness.input(at(11, 0, 0));
    harness.sample_every_5_seconds(at(11, 0, 0), at(11, 0, 10));
    harness.input(at(11, 0, 12));
    harness.sample_every_5_seconds(at(11, 0, 15), at(11, 0, 15));
    let after_restart = harness.read(at(9, 0, 0), at(12, 0, 0));

    assert_eq!(before_crash, vec![active(at(9, 55, 0), at(9, 58, 50))]);
    assert_eq!(
        after_restart,
        vec![
            active(at(9, 55, 0), at(9, 58, 0)),
            active(at(11, 0, 0), at(11, 0, 12)),
        ]
    );
}

#[test]
fn a_settled_sub_threshold_pause_longer_than_a_minute_survives_a_crash_before_any_other_write() {
    harness!(harness);
    harness.input(at(10, 0, 0));
    harness.sample_every_5_seconds(at(10, 0, 0), at(10, 2, 25));
    harness.input(at(10, 2, 30));
    harness.sample_every_5_seconds(at(10, 2, 30), at(10, 2, 30));

    harness.restart();

    assert_eq!(
        harness.read(at(9, 0, 0), at(11, 0, 0)),
        vec![active(at(10, 0, 0), at(10, 2, 30))]
    );
}

#[test]
fn idle_time_is_read_as_afk_from_the_last_input_through_the_latest_sample() {
    harness!(harness);
    harness.input(at(10, 0, 0));
    harness.sample_every_5_seconds(at(10, 0, 0), at(10, 0, 5));
    harness.input(at(10, 0, 7));
    harness.sample_every_5_seconds(at(10, 0, 10), at(10, 5, 0));

    assert_eq!(
        harness.read(at(9, 0, 0), at(11, 0, 0)),
        vec![
            active(at(10, 0, 0), at(10, 0, 7)),
            Segment(at(10, 0, 7), at(10, 5, 0), ActivityState::Afk),
        ]
    );
}

#[test]
fn suspend_during_an_active_period_then_resume_leaves_the_sleep_unknown() {
    harness!(harness);
    harness.input(at(10, 0, 0));
    harness.sample_every_5_seconds(at(10, 0, 0), at(10, 0, 25));
    harness.input(at(10, 0, 28));
    harness.sample_every_5_seconds(at(10, 0, 30), at(10, 0, 35));

    harness.source.end_observation().unwrap();
    harness.input(at(11, 0, 0));
    harness.sample_every_5_seconds(at(11, 0, 0), at(11, 0, 5));
    harness.input(at(11, 0, 9));
    harness.sample_every_5_seconds(at(11, 0, 10), at(11, 0, 10));

    assert_eq!(
        harness.read(at(9, 0, 0), at(12, 0, 0)),
        vec![
            active(at(10, 0, 0), at(10, 0, 28)),
            active(at(11, 0, 0), at(11, 0, 9)),
        ]
    );
}

#[test]
fn a_suspend_notification_stores_the_settled_boundary_and_resume_starts_a_new_span_at_once() {
    harness!(harness);
    let built_in = built_in();
    harness.input(at(10, 0, 0));
    harness.poll_every_5_seconds(&built_in, at(10, 0, 0), at(10, 0, 25));
    harness.input(at(10, 0, 28));
    harness.poll_every_5_seconds(&built_in, at(10, 0, 30), at(10, 0, 35));

    harness.clock.set(at(10, 0, 37));
    harness
        .source
        .observe_power(PowerState::Suspended, &built_in)
        .unwrap();
    harness.restart();
    let while_asleep = harness.read(at(9, 0, 0), at(12, 0, 0));
    harness.input(at(11, 0, 0));
    harness.clock.set(at(11, 0, 1));
    harness
        .source
        .observe_power(PowerState::Active, &built_in)
        .unwrap();
    harness.input(at(11, 0, 4));
    harness.poll_every_5_seconds(&built_in, at(11, 0, 6), at(11, 0, 6));

    assert_eq!(while_asleep, vec![active(at(10, 0, 0), at(10, 0, 28))]);
    assert_eq!(
        harness.read(at(9, 0, 0), at(12, 0, 0)),
        vec![
            active(at(10, 0, 0), at(10, 0, 28)),
            active(at(11, 0, 1), at(11, 0, 4)),
        ]
    );
}

#[test]
fn input_while_suspended_is_not_sampled_until_resume() {
    harness!(harness);
    let built_in = built_in();
    harness.input(at(10, 0, 0));
    harness.poll_every_5_seconds(&built_in, at(10, 0, 0), at(10, 0, 25));
    harness.input(at(10, 0, 28));
    harness.poll_every_5_seconds(&built_in, at(10, 0, 30), at(10, 0, 35));

    harness.clock.set(at(10, 0, 37));
    harness
        .source
        .observe_power(PowerState::Suspended, &built_in)
        .unwrap();
    harness.input(at(10, 0, 41));
    harness.poll_every_5_seconds(&built_in, at(10, 0, 40), at(10, 0, 50));

    assert_eq!(
        harness.read(at(9, 0, 0), at(11, 0, 0)),
        vec![active(at(10, 0, 0), at(10, 0, 28))]
    );
}

#[test]
fn missed_samples_then_recovery_in_the_same_state_start_a_new_span_and_keep_the_gap_unknown() {
    harness!(harness);
    harness.input(at(10, 0, 0));
    harness.sample_every_5_seconds(at(10, 0, 0), at(10, 0, 0));
    harness.input(at(10, 0, 4));
    harness.sample_every_5_seconds(at(10, 0, 5), at(10, 0, 5));
    harness.input(at(10, 0, 39));
    harness.sample_every_5_seconds(at(10, 0, 40), at(10, 0, 40));
    harness.input(at(10, 0, 43));
    harness.sample_every_5_seconds(at(10, 0, 45), at(10, 0, 45));

    assert_eq!(
        harness.read(at(9, 0, 0), at(11, 0, 0)),
        vec![
            active(at(10, 0, 0), at(10, 0, 4)),
            active(at(10, 0, 40), at(10, 0, 43)),
        ]
    );
}

#[test]
fn a_failed_input_query_degrades_the_source_and_the_next_successful_sample_recovers_with_a_new_span()
 {
    harness!(harness);
    harness.input(at(10, 0, 0));
    harness.sample_every_5_seconds(at(10, 0, 0), at(10, 0, 5));
    harness.input(at(10, 0, 8));
    harness.sample_every_5_seconds(at(10, 0, 10), at(10, 0, 10));

    harness.input.fails.set(true);
    harness.sample_every_5_seconds(at(10, 0, 15), at(10, 0, 20));
    let failed = harness.read_result(at(9, 0, 0), at(11, 0, 0));
    harness.input.fails.set(false);
    harness.input(at(10, 0, 22));
    harness.sample_every_5_seconds(at(10, 0, 25), at(10, 0, 30));
    harness.input(at(10, 0, 33));
    harness.sample_every_5_seconds(at(10, 0, 35), at(10, 0, 35));
    let recovered = harness.read_result(at(9, 0, 0), at(11, 0, 0));

    assert_eq!(
        failed.health,
        SourceHealth::degraded(
            "built-in",
            SourceFreshness::Unavailable,
            SourceQuality::Incomplete,
            "input-query-failed",
            Some(at(10, 0, 10)),
        )
        .unwrap()
    );
    assert_eq!(segments(&failed), vec![active(at(10, 0, 0), at(10, 0, 8))]);
    assert_eq!(
        recovered.health,
        SourceHealth::healthy("built-in", at(10, 0, 35))
    );
    assert_eq!(
        segments(&recovered),
        vec![
            active(at(10, 0, 0), at(10, 0, 8)),
            active(at(10, 0, 25), at(10, 0, 33)),
        ]
    );
}

#[test]
fn health_is_stale_once_the_latest_sample_is_older_than_two_polling_intervals() {
    harness!(harness);
    harness.input(at(10, 0, 0));
    harness.sample_every_5_seconds(at(10, 0, 0), at(10, 0, 0));

    harness.clock.set(at(10, 0, 10));
    let fresh = harness.read_result(at(9, 0, 0), at(11, 0, 0));
    harness.clock.set(at(10, 0, 11));
    let stale = harness.read_result(at(9, 0, 0), at(11, 0, 0));

    assert_eq!(
        fresh.health,
        SourceHealth::healthy("built-in", at(10, 0, 0))
    );
    assert_eq!(
        stale.health,
        SourceHealth::degraded(
            "built-in",
            SourceFreshness::Stale,
            SourceQuality::Incomplete,
            "not-sampling",
            Some(at(10, 0, 0)),
        )
        .unwrap()
    );
}

#[test]
fn polling_samples_with_the_saved_threshold_only_while_the_built_in_source_is_selected() {
    harness!(harness);
    let built_in = Settings {
        built_in_activity: BuiltInActivitySettings {
            idle_threshold: seconds(30),
        },
        ..built_in()
    };
    let activity_watch = Settings {
        activity_source: Some(ActivitySourceKind::ActivityWatch),
        ..built_in.clone()
    };
    harness.input(at(10, 0, 0));
    harness.poll_every_5_seconds(&built_in, at(10, 0, 0), at(10, 0, 5));
    harness.input(at(10, 0, 10));
    harness.poll_every_5_seconds(&built_in, at(10, 0, 10), at(10, 1, 0));
    harness.input(at(10, 1, 2));
    harness.poll_every_5_seconds(&built_in, at(10, 1, 5), at(10, 1, 10));
    harness.input(at(10, 1, 12));
    harness.poll_every_5_seconds(&built_in, at(10, 1, 15), at(10, 1, 20));
    harness.poll_every_5_seconds(&activity_watch, at(10, 1, 25), at(10, 30, 0));
    harness.input(at(10, 30, 5));
    harness.poll_every_5_seconds(&built_in, at(10, 30, 5), at(10, 30, 10));
    harness.input(at(10, 30, 12));
    harness.poll_every_5_seconds(&built_in, at(10, 30, 15), at(10, 30, 15));

    assert_eq!(
        harness.read(at(9, 0, 0), at(11, 0, 0)),
        vec![
            active(at(10, 0, 0), at(10, 0, 10)),
            Segment(at(10, 0, 10), at(10, 1, 2), ActivityState::Afk),
            active(at(10, 1, 2), at(10, 1, 12)),
            active(at(10, 30, 5), at(10, 30, 12)),
        ]
    );
}

#[test]
fn the_idle_threshold_stays_within_thirty_seconds_and_thirty_minutes() {
    harness!(harness);

    assert_eq!(
        harness.source.set_idle_threshold(seconds(29)),
        Err(IdleThresholdOutOfRange)
    );
    assert_eq!(
        harness
            .source
            .set_idle_threshold(SignedDuration::from_mins(31)),
        Err(IdleThresholdOutOfRange)
    );
    harness
        .source
        .set_idle_threshold(SignedDuration::from_mins(30))
        .unwrap();
    assert_eq!(
        harness.source.idle_threshold(),
        SignedDuration::from_mins(30)
    );
}

#[test]
fn idle_duration_is_the_tick_distance_to_the_last_input() {
    let reading = reading_from_ticks(90_000, 30_000);

    assert_eq!(reading.idle_duration(), seconds(60));
}

#[test]
fn idle_duration_survives_the_thirty_two_bit_tick_wraparound() {
    let reading = reading_from_ticks((1 << 32) + 1_000, u32::MAX - 999);

    assert_eq!(reading.idle_duration(), SignedDuration::from_millis(2_000));
}

#[test]
fn idle_duration_ignores_the_high_tick_bits_after_several_wraparounds() {
    let reading = reading_from_ticks((5 << 32) + 10_000, 4_000);

    assert_eq!(reading.idle_duration(), SignedDuration::from_millis(6_000));
}
