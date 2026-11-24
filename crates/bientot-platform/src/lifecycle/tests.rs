use std::cell::Cell;
use std::time::Instant;

use bientot_domain::configuration::Settings;
use bientot_domain::intervals::UtcInterval;
use bientot_domain::sources::{ActivitySourceKind, LastInputReading};
use bientot_domain::time::{Clock, FrozenClock};
use bientot_store::activity::ActivitySpanRepository;
use jiff::{SignedDuration, Timestamp};
use rusqlite::Connection;
use tempfile::TempDir;

use super::*;
use crate::activity::{BuiltInActivitySource, LastInputProbe};

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
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

    /// Operation key, source and kind of every stored lifecycle event, in insertion order.
    fn events(&self) -> Vec<(String, String, String)> {
        self.connection
            .prepare("SELECT OperationKey, Source, Kind FROM SessionEvent ORDER BY Id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}

fn event(key: &str, source: &str, kind: &str) -> (String, String, String) {
    (key.into(), source.into(), kind.into())
}

fn utc(change: LifecycleChange, occurred_at: Timestamp) -> LifecycleTransition {
    LifecycleTransition {
        change,
        occurred_at,
        time_zone_id: "UTC".into(),
    }
}

/// Hooks that log each call with the number of stored events at that moment.
struct LoggingHooks<'a> {
    database: &'a Database,
    log: Vec<String>,
}

impl LifecycleHooks for LoggingHooks<'_> {
    fn power_changing(&mut self, state: PowerState) {
        let stored = self.database.events().len();
        self.log.push(format!("{state:?} with {stored} events"));
    }

    fn refresh(&mut self) {
        let stored = self.database.events().len();
        self.log.push(format!("refresh with {stored} events"));
    }
}

#[test]
fn startup_and_watched_changes_are_recorded_once_with_deterministic_keys() {
    let database = Database::new();
    let clock = FrozenClock::new(at("2026-10-12T08:00:00Z"));
    let recorder =
        LifecycleEventRecorder::new(SessionEventRepository::new(&database.connection, &clock));
    let mut hooks = LoggingHooks {
        database: &database,
        log: Vec::new(),
    };

    recorder
        .prepare(
            SessionState::Unlocked,
            at("2026-10-12T08:00:00Z"),
            "UTC",
            &mut hooks,
        )
        .unwrap();
    let locked = utc(
        LifecycleChange::Session(SessionState::Locked),
        at("2026-10-12T09:00:00Z"),
    );
    recorder.record(&locked, &mut hooks).unwrap();
    recorder.record(&locked, &mut hooks).unwrap();
    recorder
        .record(
            &utc(
                LifecycleChange::Power(PowerState::Suspended),
                at("2026-10-12T10:00:00Z"),
            ),
            &mut hooks,
        )
        .unwrap();

    assert_eq!(
        database.events(),
        vec![
            event(
                "lifecycle:session:639273888000000000:unlocked:UTC",
                "windows-session",
                "unlocked"
            ),
            event(
                "lifecycle:power:639273888000000000:active:UTC",
                "windows-power",
                "resume"
            ),
            event(
                "lifecycle:session:639273924000000000:locked:UTC",
                "windows-session",
                "locked"
            ),
            event(
                "lifecycle:power:639273960000000000:suspended:UTC",
                "windows-power",
                "suspend"
            ),
        ]
    );
    assert_eq!(
        hooks.log,
        [
            "refresh with 1 events",
            "refresh with 2 events",
            "refresh with 3 events",
            "Suspended with 3 events",
            "refresh with 4 events",
        ]
    );
}

#[test]
fn an_unknown_session_at_startup_records_only_the_active_power_state() {
    let database = Database::new();
    let clock = FrozenClock::new(at("2026-10-12T08:00:00Z"));
    let recorder =
        LifecycleEventRecorder::new(SessionEventRepository::new(&database.connection, &clock));
    let mut hooks = LoggingHooks {
        database: &database,
        log: Vec::new(),
    };

    recorder
        .prepare(
            SessionState::Unknown,
            at("2026-10-12T08:00:00Z"),
            "Romance Standard Time",
            &mut hooks,
        )
        .unwrap();

    assert_eq!(
        database.events(),
        vec![event(
            "lifecycle:power:639273888000000000:active:Romance Standard Time",
            "windows-power",
            "resume"
        )]
    );
}

#[test]
fn each_watched_power_change_reaches_the_hooks_before_it_is_recorded() {
    let database = Database::new();
    let clock = FrozenClock::new(at("2026-10-12T08:00:00Z"));
    let recorder =
        LifecycleEventRecorder::new(SessionEventRepository::new(&database.connection, &clock));
    let mut hooks = LoggingHooks {
        database: &database,
        log: Vec::new(),
    };

    for (state, occurred_at) in [
        (PowerState::Suspended, "2026-10-12T10:00:00Z"),
        (PowerState::Active, "2026-10-12T11:00:00Z"),
    ] {
        recorder
            .record(
                &utc(LifecycleChange::Power(state), at(occurred_at)),
                &mut hooks,
            )
            .unwrap();
    }

    assert_eq!(
        hooks.log,
        [
            "Suspended with 0 events",
            "refresh with 1 events",
            "Active with 1 events",
            "refresh with 2 events",
        ]
    );
}

#[test]
fn a_blank_local_time_zone_is_refused_and_records_nothing() {
    let database = Database::new();
    let clock = FrozenClock::new(at("2026-10-12T08:00:00Z"));
    let recorder =
        LifecycleEventRecorder::new(SessionEventRepository::new(&database.connection, &clock));
    let mut hooks = LoggingHooks {
        database: &database,
        log: Vec::new(),
    };

    let refused = recorder.prepare(
        SessionState::Locked,
        at("2026-10-12T08:00:00Z"),
        " ",
        &mut hooks,
    );

    assert!(matches!(refused, Err(LifecycleError::BlankTimeZone(_))));
    assert_eq!(database.events(), vec![]);
}

#[test]
fn a_session_probe_that_hangs_gives_up_at_the_timeout_and_a_prompt_one_answers() {
    let (release, hung) = std::sync::mpsc::channel::<()>();
    let started = Instant::now();

    let timed_out = probe_within(Duration::from_millis(50), move || {
        let _ = hung.recv();
        SessionState::Locked
    });
    let waited = started.elapsed();
    drop(release);

    assert_eq!(timed_out, None);
    assert!(waited < Duration::from_secs(2), "waited {waited:?}");
    assert_eq!(
        probe_within(LifecycleEventRecorder::PREPARE_TIMEOUT, || {
            SessionState::Unlocked
        }),
        Some(SessionState::Unlocked)
    );
}

/// A `GetLastInputInfo` whose tick count follows the clock, as in the activity tests.
struct FakeLastInput<'a> {
    clock: &'a FrozenClock,
    last_input: Cell<Timestamp>,
}

impl LastInputProbe for FakeLastInput<'_> {
    fn read(&self) -> Option<LastInputReading> {
        let tick = |at: Timestamp| at.as_millisecond() as u32;
        Some(LastInputReading {
            tick_count: tick(self.clock.now()),
            last_input_tick: tick(self.last_input.get()),
        })
    }
}

/// The app's wiring: a power change goes to the built-in source, and a refresh polls it.
struct BuiltInHooks<'s, 'a> {
    source: &'s mut BuiltInActivitySource<'a>,
    settings: &'s Settings,
}

impl LifecycleHooks for BuiltInHooks<'_, '_> {
    fn power_changing(&mut self, state: PowerState) {
        self.source.observe_power(state, self.settings).unwrap();
    }

    fn refresh(&mut self) {
        self.source.poll(self.settings).unwrap();
    }
}

#[test]
fn refreshes_and_ticks_between_a_suspend_and_a_short_sleeps_resume_leave_the_sleep_unknown() {
    let database = Database::new();
    let clock = FrozenClock::new(at("2026-10-12T10:00:00Z"));
    let input = FakeLastInput {
        clock: &clock,
        last_input: Cell::new(at("2026-10-12T10:00:00Z")),
    };
    let mut source = BuiltInActivitySource::new(
        &input,
        ActivitySpanRepository::new(&database.connection, &clock),
        &clock,
    );
    let recorder =
        LifecycleEventRecorder::new(SessionEventRepository::new(&database.connection, &clock));
    let built_in = Settings {
        activity_source: Some(ActivitySourceKind::BuiltIn),
        ..Settings::default()
    };
    let poll_every_5_seconds = |source: &mut BuiltInActivitySource, from: &str, to: &str| {
        let mut now = at(from);
        while now <= at(to) {
            clock.set(now);
            source.poll(&built_in).unwrap();
            now += SignedDuration::from_secs(5);
        }
    };
    let power = |source: &mut BuiltInActivitySource, state, occurred_at: &str| {
        clock.set(at(occurred_at));
        let transition = utc(LifecycleChange::Power(state), at(occurred_at));
        let mut hooks = BuiltInHooks {
            source,
            settings: &built_in,
        };
        recorder.record(&transition, &mut hooks).unwrap();
    };

    poll_every_5_seconds(&mut source, "2026-10-12T10:00:00Z", "2026-10-12T10:00:25Z");
    input.last_input.set(at("2026-10-12T10:00:28Z"));
    poll_every_5_seconds(&mut source, "2026-10-12T10:00:30Z", "2026-10-12T10:00:35Z");
    power(&mut source, PowerState::Suspended, "2026-10-12T10:00:37Z");
    poll_every_5_seconds(&mut source, "2026-10-12T10:00:40Z", "2026-10-12T10:00:40Z");
    input.last_input.set(at("2026-10-12T10:00:44Z"));
    power(&mut source, PowerState::Active, "2026-10-12T10:00:45Z");
    input.last_input.set(at("2026-10-12T10:00:48Z"));
    poll_every_5_seconds(&mut source, "2026-10-12T10:00:50Z", "2026-10-12T10:00:50Z");

    let read = source
        .read(UtcInterval::new(at("2026-10-12T09:00:00Z"), at("2026-10-12T11:00:00Z")).unwrap())
        .unwrap();
    let spans: Vec<_> = read
        .coverage
        .segments()
        .iter()
        .map(|segment| (segment.interval.start(), segment.interval.end()))
        .collect();
    assert_eq!(
        spans,
        vec![
            (at("2026-10-12T10:00:00Z"), at("2026-10-12T10:00:28Z")),
            (at("2026-10-12T10:00:45Z"), at("2026-10-12T10:00:48Z")),
        ]
    );
    assert_eq!(
        database.events(),
        vec![
            event(
                "lifecycle:power:639273960370000000:suspended:UTC",
                "windows-power",
                "suspend"
            ),
            event(
                "lifecycle:power:639273960450000000:active:UTC",
                "windows-power",
                "resume"
            ),
        ]
    );
}
