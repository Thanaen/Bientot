use std::cell::RefCell;
use std::collections::VecDeque;
use std::error::Error;
use std::fs;
use std::io;

use jiff::SignedDuration;
use jiff::civil::{DateTime, date};
use jiff::tz::TimeZone;
use tempfile::TempDir;
use worktime_core::usage_ping::{PingRequest, UsagePingSender, UsagePingTransport};
use worktime_domain::sources::LastInputReading;
use worktime_domain::time::{FixedTimeZoneProvider, FrozenClock};
use worktime_domain::usage_ping::{Build, Channel};
use worktime_store::app_state::AppStateStore;
use worktime_store::data_directory::DataDirectory;
use worktime_windows::activity::LastInputProbe;

use super::*;
use crate::live;

/// Tuesday 13 October 2026, 08:00, when WorkTime starts.
const START: DateTime = date(2026, 10, 13).at(8, 0, 0, 0);

/// Answers each request with the next scripted status, or no answer for `None`.
struct ScriptedTransport {
    replies: RefCell<VecDeque<Option<u16>>>,
    sent: RefCell<usize>,
}

impl ScriptedTransport {
    fn new(replies: impl IntoIterator<Item = Option<u16>>) -> Self {
        Self {
            replies: RefCell::new(replies.into_iter().collect()),
            sent: RefCell::new(0),
        }
    }

    fn sent(&self) -> usize {
        *self.sent.borrow()
    }
}

impl UsagePingTransport for &ScriptedTransport {
    fn post(&self, _: &PingRequest) -> Result<u16, Box<dyn Error + Send + Sync>> {
        *self.sent.borrow_mut() += 1;
        let reply = self.replies.borrow_mut().pop_front();
        reply
            .expect("an unexpected request")
            .ok_or_else(|| "offline".into())
    }
}

/// The app host's loop on a fake clock: a check every 30 s from `START`, the sender asked
/// whenever the schedule says.
struct Shell<'t> {
    _state: TempDir,
    schedule: Schedule,
    sender: UsagePingSender<&'t ScriptedTransport>,
    started: Instant,
    elapsed: Duration,
    enabled: bool,
}

impl<'t> Shell<'t> {
    fn start(transport: &'t ScriptedTransport) -> Self {
        let state = TempDir::new().unwrap();
        let store = AppStateStore::new(state.path().join("app-state.json")).unwrap();
        let build = Build::new("1.0.0", Channel::Zip).unwrap();
        let mut shell = Self {
            _state: state,
            schedule: Schedule::default(),
            sender: UsagePingSender::new(transport, store, build).unwrap(),
            started: Instant::now(),
            elapsed: Duration::ZERO,
            enabled: true,
        };
        shell.check();
        shell
    }

    fn check(&mut self) {
        let today = START
            .checked_add(SignedDuration::try_from(self.elapsed).unwrap())
            .unwrap()
            .date();
        if self.schedule.attempt(today, self.started + self.elapsed) {
            self.sender.send_if_due(self.enabled, today);
        }
    }

    /// Lets `duration` go by, checking every 30 s.
    fn run_for(&mut self, duration: Duration) {
        let end = self.elapsed + duration;
        while self.elapsed < end {
            self.elapsed += Duration::from_secs(30);
            self.check();
        }
    }
}

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(60 * 60);

#[test]
fn the_signal_goes_at_startup_then_once_on_the_next_day() {
    let transport = ScriptedTransport::new([Some(204), Some(204)]);
    let mut shell = Shell::start(&transport);
    assert_eq!(transport.sent(), 1);

    // Until 23:59:30 on the same day.
    shell.run_for(16 * HOUR - MINUTE);
    assert_eq!(transport.sent(), 1);

    shell.run_for(MINUTE);
    assert_eq!(transport.sent(), 2);
    shell.run_for(24 * HOUR - MINUTE);
    assert_eq!(transport.sent(), 2);
}

#[test]
fn a_failed_signal_is_tried_again_an_hour_later_not_before() {
    let transport = ScriptedTransport::new([Some(503), None, Some(204)]);
    let mut shell = Shell::start(&transport);

    shell.run_for(HOUR - MINUTE);
    assert_eq!(transport.sent(), 1);
    shell.run_for(MINUTE);
    assert_eq!(transport.sent(), 2);
    shell.run_for(HOUR);
    assert_eq!(transport.sent(), 3);
    shell.run_for(10 * HOUR);
    assert_eq!(transport.sent(), 3);
}

#[test]
fn a_new_day_tries_at_once_even_within_the_hour() {
    let transport = ScriptedTransport::new([Some(204), Some(500), Some(204)]);
    let mut shell = Shell::start(&transport);
    // 23:40, tomorrow fails at 00:00, then 01:00 sends.
    shell.run_for(15 * HOUR + 40 * MINUTE);
    assert_eq!(transport.sent(), 1);

    shell.run_for(20 * MINUTE);
    assert_eq!(transport.sent(), 2);
    shell.run_for(HOUR);
    assert_eq!(transport.sent(), 3);
}

#[test]
fn turning_the_setting_off_stops_the_retries_and_on_again_resumes_them() {
    let transport = ScriptedTransport::new([Some(503), Some(204)]);
    let mut shell = Shell::start(&transport);

    shell.enabled = false;
    shell.run_for(5 * HOUR);
    assert_eq!(transport.sent(), 1);

    shell.enabled = true;
    shell.run_for(HOUR);
    assert_eq!(transport.sent(), 2);
}

#[test]
fn a_rejected_signal_is_not_tried_again_by_this_launch() {
    let transport = ScriptedTransport::new([Some(413)]);
    let mut shell = Shell::start(&transport);

    shell.run_for(48 * HOUR);

    assert_eq!(transport.sent(), 1);
}

#[test]
fn the_schedule_tries_again_when_the_clock_goes_back_a_day() {
    let mut schedule = Schedule::default();
    let now = Instant::now();

    assert!(schedule.attempt(date(2026, 10, 13), now));
    assert!(!schedule.attempt(date(2026, 10, 13), now + 59 * MINUTE));
    assert!(schedule.attempt(date(2026, 10, 12), now + 59 * MINUTE));
}

struct Typing;

impl LastInputProbe for Typing {
    fn read(&self) -> Option<LastInputReading> {
        None
    }
}

/// The user's data in `folder`, opened as a launch does.
fn host(folder: &TempDir) -> LiveHost<'static> {
    let directory = DataDirectory::from(folder.path().to_owned());
    let connection = worktime_store::database::open(&directory.database_file()).unwrap();
    LiveHost::start(
        &directory,
        live::load_settings(&directory).unwrap(),
        Box::leak(Box::new(connection)),
        Box::leak(Box::new(FrozenClock::new(
            "2026-10-13T08:00:00Z".parse().unwrap(),
        ))),
        Box::leak(Box::new(FixedTimeZoneProvider::new("UTC", TimeZone::UTC))),
        &Typing,
    )
    .unwrap()
}

/// The `(event, level, status)` of each line of the diagnostic log that names the signal.
fn logged(folder: &TempDir) -> Vec<(String, String, serde_json::Value)> {
    let mut lines = Vec::new();
    for file in fs::read_dir(folder.path().join("logs")).unwrap() {
        let text = fs::read_to_string(file.unwrap().path()).unwrap();
        for line in text.lines() {
            let entry: serde_json::Value = serde_json::from_str(line).unwrap();
            let event = entry["Event"].as_str().unwrap_or_default().to_owned();
            if event.starts_with("usage_ping") || event.starts_with("app_state") {
                let level = entry["Level"].as_str().unwrap_or_default().to_owned();
                let status = entry["Properties"]["status"].clone();
                lines.push((event, level, status));
            }
        }
    }
    lines
}

#[test]
fn each_outcome_logs_its_event_with_the_status_code_only() {
    let folder = TempDir::new().unwrap();
    let mut host = host(&folder);

    for outcome in [
        UsagePingOutcome::Skipped,
        UsagePingOutcome::Sent,
        UsagePingOutcome::SentUnsaved(io::Error::other("disk full")),
        UsagePingOutcome::Rejected { status: 400 },
        UsagePingOutcome::Failed { status: Some(503) },
        UsagePingOutcome::Failed { status: None },
    ] {
        log(&mut host, outcome);
    }

    let entry = |event: &str, level: &str, status| (event.to_owned(), level.to_owned(), status);
    assert_eq!(
        logged(&folder),
        [
            entry("usage_ping.sent", "Information", 204.into()),
            entry("app_state.save_failed", "Error", serde_json::Value::Null),
            entry("usage_ping.sent", "Information", 204.into()),
            entry("usage_ping.failed", "Error", 400.into()),
            entry("usage_ping.failed", "Warning", 503.into()),
            entry("usage_ping.failed", "Warning", serde_json::Value::Null),
        ]
    );
}
