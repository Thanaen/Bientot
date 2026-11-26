//! SC-001: the same timeline (lock, unlock, suspend, resume, Switch User and back) written by the
//! Windows decoder and by the logind mapping gives the same ledger rows. Both decoding halves
//! build on any OS, so the comparison runs on every CI system.

use bientot_domain::accounting::SessionState;
use bientot_domain::time::{Clock, FixedTimeZoneProvider, FrozenClock, TimeZoneProvider};
use bientot_linux::logind::{SessionReadings, SessionTracker, Signal, Step};
use bientot_platform::PowerState;
use bientot_platform::lifecycle::{LifecycleEventRecorder, LifecycleHooks, LifecycleTransition};
use bientot_store::sessions::SessionEventRepository;
use bientot_windows::lifecycle::{LifecycleMessages, WindowMessage, session_state};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use rusqlite::Connection;
use tempfile::TempDir;

const SESSION_ID: u32 = 1;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_CONSOLE_CONNECT: usize = 0x1;
const WTS_CONSOLE_DISCONNECT: usize = 0x2;
const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;
const WM_POWERBROADCAST: u32 = 0x0218;
const PBT_APMSUSPEND: usize = 0x4;
const PBT_APMRESUMESUSPEND: usize = 0x7;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
const WTS_SESSIONSTATE_UNLOCK: i32 = 1;

const fn session_change(event: usize) -> WindowMessage {
    WindowMessage {
        message: WM_WTSSESSION_CHANGE,
        w_param: event,
        l_param: SESSION_ID as isize,
    }
}

const fn power_broadcast(event: usize) -> WindowMessage {
    WindowMessage {
        message: WM_POWERBROADCAST,
        w_param: event,
        l_param: 0,
    }
}

const fn session_properties(locked_hint: Option<bool>, active: Option<bool>) -> Signal {
    Signal::SessionChanged {
        locked_hint,
        active,
    }
}

/// One instant of the timeline, as each system reports it.
struct Moment {
    at: &'static str,
    windows: &'static [WindowMessage],
    linux: &'static [Signal],
}

/// A GNOME session: the shell announces its lock screen on the screensaver interface and sets
/// `LockedHint`; Switch User only changes `Active`.
const TIMELINE: [Moment; 6] = [
    Moment {
        at: "2026-11-26T12:15:00Z",
        windows: &[session_change(WTS_SESSION_LOCK)],
        linux: &[
            Signal::ScreenSaverActive(true),
            session_properties(Some(true), None),
        ],
    },
    Moment {
        at: "2026-11-26T13:05:00Z",
        windows: &[session_change(WTS_SESSION_UNLOCK)],
        linux: &[
            session_properties(Some(false), None),
            Signal::ScreenSaverActive(false),
        ],
    },
    Moment {
        at: "2026-11-26T18:40:00Z",
        windows: &[power_broadcast(PBT_APMSUSPEND)],
        linux: &[Signal::PrepareForSleep(true)],
    },
    Moment {
        at: "2026-11-27T07:55:00Z",
        windows: &[
            power_broadcast(PBT_APMRESUMEAUTOMATIC),
            power_broadcast(PBT_APMRESUMESUSPEND),
        ],
        linux: &[Signal::PrepareForSleep(false)],
    },
    Moment {
        at: "2026-11-27T10:20:00Z",
        windows: &[
            session_change(WTS_SESSION_LOCK),
            session_change(WTS_CONSOLE_DISCONNECT),
        ],
        linux: &[session_properties(None, Some(false))],
    },
    Moment {
        at: "2026-11-27T10:35:00Z",
        windows: &[
            session_change(WTS_CONSOLE_CONNECT),
            session_change(WTS_SESSION_UNLOCK),
        ],
        linux: &[session_properties(None, Some(true))],
    },
];

const START: &str = "2026-11-26T08:30:00Z";
/// The shutdown that ends the Linux observation; Windows ends the process without a message.
const SHUTDOWN: &str = "2026-11-27T19:00:00Z";

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn zones() -> FixedTimeZoneProvider {
    FixedTimeZoneProvider::new("Europe/Paris", TimeZone::get("Europe/Paris").unwrap())
}

struct Now<'a>(&'a FrozenClock);

impl Clock for Now<'_> {
    fn now(&self) -> Timestamp {
        self.0.now()
    }
}

struct NoHooks;

impl LifecycleHooks for NoHooks {
    fn power_changing(&mut self, _state: PowerState) {}

    fn refresh(&mut self) {}
}

/// A fresh ledger, its clock and its recorder.
struct Ledger {
    _directory: TempDir,
    connection: Connection,
    clock: FrozenClock,
}

impl Ledger {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let connection =
            bientot_store::database::open(&directory.path().join("worktime.db")).unwrap();
        bientot_store::database::migrate(&connection).unwrap();
        Self {
            _directory: directory,
            connection,
            clock: FrozenClock::new(at(START)),
        }
    }

    fn prepare(&self, session: SessionState) {
        let recorder = self.recorder();
        recorder
            .prepare(session, at(START), &zones().local().0, &mut NoHooks)
            .unwrap();
    }

    fn record(&self, transition: &LifecycleTransition) {
        self.recorder().record(transition, &mut NoHooks).unwrap();
    }

    fn recorder(&self) -> LifecycleEventRecorder<'_> {
        LifecycleEventRecorder::new(SessionEventRepository::new(&self.connection, &self.clock))
    }

    /// Every column of every lifecycle row, in the order written.
    fn rows(&self) -> Vec<[String; 6]> {
        self.connection
            .prepare(
                "SELECT OccurredAtUtc, TimezoneId, Kind, Source, CreatedAtUtc, OperationKey \
                 FROM SessionEvent ORDER BY Id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok([
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ])
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}

fn windows_ledger() -> Ledger {
    let ledger = Ledger::new();
    ledger.prepare(session_state(Some(WTS_SESSIONSTATE_UNLOCK)));
    let messages = LifecycleMessages::new(SESSION_ID, Now(&ledger.clock), zones());
    for moment in &TIMELINE {
        ledger.clock.set(at(moment.at));
        for &message in moment.windows {
            if let Some(transition) = messages.transition(message) {
                ledger.record(&transition);
            }
        }
    }
    ledger
}

fn linux_ledger() -> Ledger {
    let ledger = Ledger::new();
    let mut tracker = SessionTracker::new(SessionReadings {
        locked_hint: false,
        active: true,
        screensaver: Some(false),
    });
    ledger.prepare(tracker.snapshot().session);
    let shutdown = TIMELINE
        .iter()
        .map(|moment| (moment.at, moment.linux))
        .chain([(SHUTDOWN, [Signal::PrepareForShutdown(true)].as_slice())]);
    let mut ended = false;
    for (instant, signals) in shutdown {
        ledger.clock.set(at(instant));
        for &signal in signals {
            assert!(!ended, "no signal is applied after the observation ended");
            match tracker.apply(signal) {
                Some(Step::Changed(change)) => ledger.record(&LifecycleTransition {
                    change,
                    occurred_at: ledger.clock.now(),
                    time_zone_id: zones().local().0,
                }),
                Some(Step::Ended) => ended = true,
                None => {}
            }
        }
    }
    assert!(ended, "the shutdown ends the Linux observation");
    ledger
}

#[test]
fn windows_and_linux_write_the_same_rows_for_the_same_timeline() {
    let windows = windows_ledger().rows();
    let linux = linux_ledger().rows();

    let kinds: Vec<_> = windows
        .iter()
        .map(|[occurred_at, _, kind, source, ..]| {
            (occurred_at.as_str(), kind.as_str(), source.as_str())
        })
        .collect();
    assert_eq!(
        kinds,
        [
            (
                "2026-11-26T08:30:00.0000000+00:00",
                "unlocked",
                "windows-session"
            ),
            (
                "2026-11-26T08:30:00.0000000+00:00",
                "resume",
                "windows-power"
            ),
            (
                "2026-11-26T12:15:00.0000000+00:00",
                "locked",
                "windows-session"
            ),
            (
                "2026-11-26T13:05:00.0000000+00:00",
                "unlocked",
                "windows-session"
            ),
            (
                "2026-11-26T18:40:00.0000000+00:00",
                "suspend",
                "windows-power"
            ),
            (
                "2026-11-27T07:55:00.0000000+00:00",
                "resume",
                "windows-power"
            ),
            (
                "2026-11-27T10:20:00.0000000+00:00",
                "locked",
                "windows-session"
            ),
            (
                "2026-11-27T10:35:00.0000000+00:00",
                "unlocked",
                "windows-session"
            ),
        ]
    );
    assert_eq!(linux, windows);
}
