//! SC-001: one recorded day of input, read as Windows reads it (`reading_from_ticks`) and as
//! Linux does through `IdleProbe` (`reading_from_idle`), gives the built-in source the same
//! ledger rows. Linux reads it twice: as a polled backend tells it (Mutter, Muffin,
//! `MIT-SCREEN-SAVER`: the milliseconds since the input), and as `ext_idle_notifier_v1` tells
//! it (the compositor's `idled` and `resumed` alone). Everything here builds on any OS, so the
//! comparison runs on every CI system.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use bientot_domain::configuration::Settings;
use bientot_domain::sources::{ActivitySourceKind, LastInputReading};
use bientot_domain::time::Clock;
use bientot_linux::idle::ext_idle::{Notification, TIMEOUT};
use bientot_linux::idle::{Backend, Backends, IdleProbe, IdleReader, MonotonicClock, Opened};
use bientot_platform::activity::{BuiltInActivitySource, LastInputProbe, reading_from_ticks};
use bientot_platform::system::SessionType;
use bientot_store::activity::ActivitySpanRepository;
use jiff::{SignedDuration, Timestamp};
use rusqlite::Connection;
use tempfile::TempDir;

/// Stretches of work, each an input every `every` from `from` up to `until`; between them the
/// user is away: past the idle threshold (3 minutes) for coffee and lunch, under it once. The
/// inputs are more than the notification's timeout apart, so that the compositor's events pin
/// each of them, as `GetLastInputInfo` does.
const WORK: [(&str, &str, u64); 5] = [
    ("2026-11-26T08:30:00Z", "2026-11-26T10:12:13Z", 7),
    ("2026-11-26T10:31:40Z", "2026-11-26T12:02:31Z", 9),
    ("2026-11-26T12:04:00Z", "2026-11-26T12:15:00Z", 6),
    ("2026-11-26T13:20:07Z", "2026-11-26T15:41:52Z", 11),
    ("2026-11-26T15:44:39Z", "2026-11-26T17:45:52Z", 8),
];
/// The host samples every 5 s from here, off the inputs' whole seconds.
const FIRST_SAMPLE: &str = "2026-11-26T08:30:01.250Z";
const LAST_SAMPLE: &str = "2026-11-26T18:00:00Z";
const POLL: SignedDuration = SignedDuration::from_secs(5);
/// Windows' 64-bit tick count at the first input: its 32 low bits wrap at 12:30.
const TICK_AT_START: u64 = (3 << 32) - 4 * 3_600_000;
/// `CLOCK_MONOTONIC` at the first input: up three days, two hours.
const MONOTONIC_AT_START: Duration = Duration::from_secs(74 * 3_600);

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn start() -> Timestamp {
    at(WORK[0].0)
}

/// Every input of the day, in order.
fn inputs() -> Vec<Timestamp> {
    WORK.iter()
        .flat_map(|&(from, until, every)| {
            let (from, until) = (at(from), at(until));
            std::iter::successors(Some(from), move |&input| {
                Some(input + SignedDuration::from_secs(every as i64))
            })
            .take_while(move |&input| input <= until)
        })
        .collect()
}

fn last_input(inputs: &[Timestamp], now: Timestamp) -> Timestamp {
    inputs[inputs.partition_point(|&input| input <= now) - 1]
}

fn since_start(instant: Timestamp) -> Duration {
    instant.duration_since(start()).unsigned_abs()
}

fn monotonic(instant: Timestamp) -> Duration {
    MONOTONIC_AT_START + since_start(instant)
}

/// The wall clock of the host, which the source stamps its rows with, shared with the probes.
#[derive(Clone)]
struct Wall(Arc<AtomicI64>);

impl Wall {
    fn new() -> Self {
        Self(Arc::new(AtomicI64::new(start().as_millisecond())))
    }

    fn set(&self, now: Timestamp) {
        self.0.store(now.as_millisecond(), Ordering::SeqCst);
    }
}

impl Clock for Wall {
    fn now(&self) -> Timestamp {
        Timestamp::from_millisecond(self.0.load(Ordering::SeqCst)).unwrap()
    }
}

/// `CLOCK_MONOTONIC` of the same machine.
struct Monotonic(Wall);

impl MonotonicClock for Monotonic {
    fn now(&self) -> Duration {
        monotonic(self.0.now())
    }
}

/// `GetTickCount64` and `GetLastInputInfo`, as `SystemLastInput` reads them.
struct Windows {
    wall: Wall,
    inputs: Vec<Timestamp>,
}

impl Windows {
    fn tick(instant: Timestamp) -> u64 {
        TICK_AT_START + since_start(instant).as_millis() as u64
    }
}

impl LastInputProbe for Windows {
    fn read(&self) -> Option<LastInputReading> {
        let now = self.wall.now();
        let last_input = Self::tick(last_input(&self.inputs, now)) as u32;
        Some(reading_from_ticks(Self::tick(now), last_input))
    }
}

/// A backend polled for the milliseconds since the last input.
struct Polled(Vec<Duration>);

impl IdleReader for Polled {
    fn idle(&mut self, now: Duration) -> Option<Duration> {
        let last = self.0[self.0.partition_point(|&input| input <= now) - 1];
        Some(now - last)
    }
}

/// The compositor's events for these inputs, created at the first: `idled` one timeout after an
/// input that none follows within it, `resumed` at the input after an `idled`.
fn compositor_events(inputs: &[Duration]) -> Vec<Notification> {
    let mut events = vec![Notification::Resumed(inputs[0])];
    for (index, &input) in inputs.iter().enumerate() {
        if matches!(events.last(), Some(Notification::Idled(_))) {
            events.push(Notification::Resumed(input));
        }
        let next = inputs.get(index + 1).copied();
        if next.is_none_or(|next| next - input > TIMEOUT) {
            events.push(Notification::Idled(input + TIMEOUT));
        }
    }
    events
}

/// `ext_idle_notifier_v1`: only the last event that arrived.
struct Notified(Vec<Notification>);

impl IdleReader for Notified {
    fn idle(&mut self, now: Duration) -> Option<Duration> {
        let arrived = |event: &Notification| match *event {
            Notification::Resumed(at) | Notification::Idled(at) => at <= now,
        };
        let last = self.0[self.0.partition_point(arrived) - 1];
        Some(last.idle_at(now))
    }
}

/// A machine with one backend, that reader.
struct Only(Backend, Option<Box<dyn IdleReader>>);

impl Backends for Only {
    fn open(&mut self, backend: Backend) -> Option<Opened> {
        (backend == self.0).then(|| Opened {
            reader: self.1.take().expect("the backend opens once"),
            inhibitor_blind: true,
        })
    }
}

fn linux(wall: &Wall, backend: Backend, reader: impl IdleReader + 'static) -> impl LastInputProbe {
    let session = match backend {
        Backend::XScreenSaver | Backend::MuffinIdleMonitor => SessionType::X11,
        _ => SessionType::Wayland,
    };
    IdleProbe::new(
        Some(session),
        Only(backend, Some(Box::new(reader))),
        Monotonic(wall.clone()),
        |_| {},
    )
}

struct Ledger {
    _directory: TempDir,
    connection: Connection,
}

impl Ledger {
    /// The rows the built-in source writes over the day, sampling `input` every 5 s.
    fn of_the_day(wall: &Wall, input: &dyn LastInputProbe) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let connection =
            bientot_store::database::open(&directory.path().join("worktime.db")).unwrap();
        bientot_store::database::migrate(&connection).unwrap();
        let settings = Settings {
            activity_source: Some(ActivitySourceKind::BuiltIn),
            ..Settings::default()
        };
        {
            let spans = ActivitySpanRepository::new(&connection, wall);
            let mut source = BuiltInActivitySource::new(input, spans, wall);
            let mut now = at(FIRST_SAMPLE);
            while now <= at(LAST_SAMPLE) {
                wall.set(now);
                source.poll(&settings).unwrap();
                now += POLL;
            }
            source.end_observation().unwrap();
        }
        Self {
            _directory: directory,
            connection,
        }
    }

    /// Every column of every activity span, in the order written.
    fn rows(&self) -> Vec<[String; 4]> {
        self.connection
            .prepare(
                "SELECT State, StartUtc, SettledThroughUtc, CreatedAtUtc \
                 FROM ActivitySpan ORDER BY Id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok([row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?])
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}

#[test]
fn windows_and_linux_write_the_same_activity_rows_for_the_same_day() {
    let inputs = inputs();
    let monotonic_inputs: Vec<_> = inputs.iter().map(|&input| monotonic(input)).collect();
    let wall = Wall::new();

    let windows = Ledger::of_the_day(
        &wall,
        &Windows {
            wall: wall.clone(),
            inputs: inputs.clone(),
        },
    )
    .rows();
    let polled = Ledger::of_the_day(
        &wall,
        &linux(
            &wall,
            Backend::XScreenSaver,
            Polled(monotonic_inputs.clone()),
        ),
    )
    .rows();
    let notified = Ledger::of_the_day(
        &wall,
        &linux(
            &wall,
            Backend::ExtIdleNotify,
            Notified(compositor_events(&monotonic_inputs)),
        ),
    )
    .rows();

    let spans: Vec<_> = windows
        .iter()
        .map(|[state, start, settled, _]| (state.as_str(), start.as_str(), settled.as_str()))
        .collect();
    assert_eq!(
        spans,
        [
            (
                "active",
                "2026-11-26T08:30:01.2500000+00:00",
                "2026-11-26T10:12:12.0000000+00:00"
            ),
            (
                "idle",
                "2026-11-26T10:12:12.0000000+00:00",
                "2026-11-26T10:31:40.0000000+00:00"
            ),
            (
                "active",
                "2026-11-26T10:31:40.0000000+00:00",
                "2026-11-26T12:15:00.0000000+00:00"
            ),
            (
                "idle",
                "2026-11-26T12:15:00.0000000+00:00",
                "2026-11-26T13:20:07.0000000+00:00"
            ),
            (
                "active",
                "2026-11-26T13:20:07.0000000+00:00",
                "2026-11-26T17:45:51.0000000+00:00"
            ),
            (
                "idle",
                "2026-11-26T17:45:51.0000000+00:00",
                "2026-11-26T17:59:56.2500000+00:00"
            ),
        ]
    );
    assert_eq!(polled, windows, "a polled backend");
    assert_eq!(notified, windows, "ext_idle_notifier_v1");
}
