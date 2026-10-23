use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;

use jiff::civil::date;
use tempfile::TempDir;
use worktime_domain::usage_ping::Channel;

use super::*;

const TODAY: Date = date(2026, 10, 13);

/// Answers each request with the next scripted reply and records what it was sent.
struct ScriptedTransport {
    replies: RefCell<VecDeque<Result<u16, &'static str>>>,
    sent: RefCell<Vec<PingRequest>>,
}

impl ScriptedTransport {
    fn new(replies: impl IntoIterator<Item = Result<u16, &'static str>>) -> Self {
        Self {
            replies: RefCell::new(replies.into_iter().collect()),
            sent: RefCell::new(Vec::new()),
        }
    }
}

impl UsagePingTransport for &ScriptedTransport {
    fn post(&self, request: &PingRequest) -> Result<u16, Box<dyn Error + Send + Sync>> {
        self.sent.borrow_mut().push(request.clone());
        let reply = self
            .replies
            .borrow_mut()
            .pop_front()
            .expect("an unexpected request");
        reply.map_err(Into::into)
    }
}

struct Fixture {
    directory: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            directory: TempDir::new().unwrap(),
        }
    }

    fn with_last_sent(last_sent: Date) -> Self {
        let fixture = Self::new();
        fixture
            .store()
            .save(&AppState {
                usage_ping_last_sent: Some(last_sent),
            })
            .unwrap();
        fixture
    }

    fn store(&self) -> AppStateStore {
        AppStateStore::new(self.directory.path().join("app-state.json")).unwrap()
    }

    fn last_sent(&self) -> Option<Date> {
        self.store().load().unwrap().usage_ping_last_sent
    }

    fn sender<'a>(
        &self,
        transport: &'a ScriptedTransport,
    ) -> UsagePingSender<&'a ScriptedTransport> {
        UsagePingSender::new(transport, self.store(), build()).unwrap()
    }
}

fn build() -> Build {
    Build::new("1.0.0", Channel::Store).unwrap()
}

fn bodies(transport: &ScriptedTransport) -> Vec<String> {
    transport
        .sent
        .borrow()
        .iter()
        .map(|request| request.body.clone())
        .collect()
}

#[test]
fn the_first_signal_is_exactly_the_three_fields_the_service_accepts() {
    let fixture = Fixture::new();
    let transport = ScriptedTransport::new([Ok(204)]);

    let outcome = fixture.sender(&transport).send_if_due(true, TODAY);

    assert!(matches!(outcome, UsagePingOutcome::Sent), "{outcome:?}");
    assert_eq!(
        *transport.sent.borrow(),
        [PingRequest {
            url: "https://worktime.thanaen.dev/v1/ping",
            headers: [
                ("Content-Type", "application/json"),
                ("User-Agent", "WorkTime"),
            ],
            body: r#"{"version":"1.0.0","channel":"store","first_this_month":true}"#.to_owned(),
            timeout: Duration::from_secs(10),
        }]
    );
    assert_eq!(fixture.last_sent(), Some(TODAY));
}

#[test]
fn a_later_day_of_the_same_month_is_not_the_first_of_the_month() {
    let fixture = Fixture::with_last_sent(date(2026, 10, 1));
    let transport = ScriptedTransport::new([Ok(204)]);
    let zip = Build::new("2.10.0-beta.1", Channel::Zip).unwrap();

    let outcome = UsagePingSender::new(&transport, fixture.store(), zip)
        .unwrap()
        .send_if_due(true, TODAY);

    assert!(matches!(outcome, UsagePingOutcome::Sent), "{outcome:?}");
    assert_eq!(
        bodies(&transport),
        [r#"{"version":"2.10.0-beta.1","channel":"zip","first_this_month":false}"#]
    );
}

#[test]
fn one_signal_a_day_then_another_the_next_day() {
    let fixture = Fixture::new();
    let transport = ScriptedTransport::new([Ok(204), Ok(204)]);
    let sender = fixture.sender(&transport);

    sender.send_if_due(true, TODAY);
    let again = sender.send_if_due(true, TODAY);
    let tomorrow = sender.send_if_due(true, date(2026, 10, 14));

    assert!(matches!(again, UsagePingOutcome::Skipped), "{again:?}");
    assert!(matches!(tomorrow, UsagePingOutcome::Sent), "{tomorrow:?}");
    assert_eq!(transport.sent.borrow().len(), 2);
    assert_eq!(fixture.last_sent(), Some(date(2026, 10, 14)));
}

#[test]
fn a_signal_sent_by_an_earlier_launch_today_is_not_sent_again() {
    let fixture = Fixture::with_last_sent(TODAY);
    let transport = ScriptedTransport::new([]);

    let outcome = fixture.sender(&transport).send_if_due(true, TODAY);

    assert!(matches!(outcome, UsagePingOutcome::Skipped), "{outcome:?}");
    assert!(transport.sent.borrow().is_empty());
}

#[test]
fn turning_the_setting_off_stops_every_send_without_a_restart() {
    let fixture = Fixture::with_last_sent(date(2026, 10, 12));
    let transport = ScriptedTransport::new([Ok(503)]);
    let sender = fixture.sender(&transport);

    let failed = sender.send_if_due(true, TODAY);
    let off = sender.send_if_due(false, TODAY);
    let next_day = sender.send_if_due(false, date(2026, 10, 14));

    assert!(
        matches!(failed, UsagePingOutcome::Failed { status: Some(503) }),
        "{failed:?}"
    );
    assert!(matches!(off, UsagePingOutcome::Skipped), "{off:?}");
    assert!(
        matches!(next_day, UsagePingOutcome::Skipped),
        "{next_day:?}"
    );
    assert_eq!(transport.sent.borrow().len(), 1);
    assert_eq!(fixture.last_sent(), Some(date(2026, 10, 12)));
}

#[test]
fn a_failure_keeps_the_signal_due_and_saves_nothing() {
    let fixture = Fixture::new();
    let transport = ScriptedTransport::new([Err("offline"), Ok(500), Ok(204)]);
    let sender = fixture.sender(&transport);

    let offline = sender.send_if_due(true, TODAY);
    assert_eq!(fixture.last_sent(), None);
    let server_error = sender.send_if_due(true, TODAY);
    assert_eq!(fixture.last_sent(), None);
    let sent = sender.send_if_due(true, TODAY);

    assert!(
        matches!(offline, UsagePingOutcome::Failed { status: None }),
        "{offline:?}"
    );
    assert!(
        matches!(server_error, UsagePingOutcome::Failed { status: Some(500) }),
        "{server_error:?}"
    );
    assert!(matches!(sent, UsagePingOutcome::Sent), "{sent:?}");
    let first = r#"{"version":"1.0.0","channel":"store","first_this_month":true}"#;
    assert_eq!(bodies(&transport), [first, first, first]);
    assert_eq!(fixture.last_sent(), Some(TODAY));
}

#[test]
fn a_rejected_signal_is_not_sent_again_until_the_next_launch() {
    for status in [400, 413] {
        let fixture = Fixture::new();
        let transport = ScriptedTransport::new([Ok(status)]);
        let sender = fixture.sender(&transport);

        let rejected = sender.send_if_due(true, TODAY);
        let later = sender.send_if_due(true, TODAY);
        let tomorrow = sender.send_if_due(true, date(2026, 10, 14));

        assert!(
            matches!(rejected, UsagePingOutcome::Rejected { status: s } if s == status),
            "{rejected:?}"
        );
        assert!(matches!(later, UsagePingOutcome::Skipped), "{later:?}");
        assert!(
            matches!(tomorrow, UsagePingOutcome::Skipped),
            "{tomorrow:?}"
        );
        assert_eq!(transport.sent.borrow().len(), 1);
        assert_eq!(fixture.last_sent(), None);
    }
}

#[test]
fn a_recorded_signal_whose_date_cannot_be_saved_is_not_sent_again_today() {
    let directory = TempDir::new().unwrap();
    let folder = directory.path().join("WorkTime");
    let store = AppStateStore::new(folder.join("app-state.json")).unwrap();
    let transport = ScriptedTransport::new([Ok(204)]);
    let sender = UsagePingSender::new(&transport, store, build()).unwrap();
    fs::write(&folder, "a file where the folder should be").unwrap();

    let unsaved = sender.send_if_due(true, TODAY);
    let again = sender.send_if_due(true, TODAY);

    assert!(
        matches!(unsaved, UsagePingOutcome::SentUnsaved(_)),
        "{unsaved:?}"
    );
    assert!(matches!(again, UsagePingOutcome::Skipped), "{again:?}");
    assert_eq!(transport.sent.borrow().len(), 1);
}

/// Holds its request until the test lets it answer.
struct HeldTransport {
    entered: Arc<Barrier>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl UsagePingTransport for HeldTransport {
    fn post(&self, _: &PingRequest) -> Result<u16, Box<dyn Error + Send + Sync>> {
        self.entered.wait();
        self.release.lock().unwrap().recv().unwrap();
        Ok(204)
    }
}

#[test]
fn a_second_call_while_a_signal_is_in_flight_sends_nothing() {
    let fixture = Fixture::new();
    let entered = Arc::new(Barrier::new(2));
    let (release, held) = mpsc::channel();
    let transport = HeldTransport {
        entered: Arc::clone(&entered),
        release: Mutex::new(held),
    };
    let sender = Arc::new(UsagePingSender::new(transport, fixture.store(), build()).unwrap());

    let first = thread::spawn({
        let sender = Arc::clone(&sender);
        move || sender.send_if_due(true, TODAY)
    });
    entered.wait();
    let second = sender.send_if_due(true, TODAY);
    release.send(()).unwrap();
    let first = first.join().unwrap();

    assert!(matches!(second, UsagePingOutcome::Skipped), "{second:?}");
    assert!(matches!(first, UsagePingOutcome::Sent), "{first:?}");
}

#[test]
fn the_workspace_version_is_one_the_service_accepts() {
    assert!(Build::new(env!("CARGO_PKG_VERSION"), Channel::Store).is_some());
}
