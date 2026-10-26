use std::cell::Cell;

use jiff::civil::date;

use super::*;

use SessionHistoryBoundaryKind::{Lock, Logoff, Logon, Unlock};

const USER: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";
const OTHER_USER: &str = "S-1-5-21-1004336348-1177238915-682003330-1002";

fn at(hour: i64, minute: i64, millisecond: i64) -> Timestamp {
    date(2026, 10, 26)
        .at(0, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
        + SignedDuration::from_hours(hour)
        + SignedDuration::from_mins(minute)
        + SignedDuration::from_millis(millisecond)
}

fn boundary(
    kind: SessionHistoryBoundaryKind,
    record_id: u64,
    at: Timestamp,
) -> SessionHistoryBoundary {
    SessionHistoryBoundary {
        kind,
        record_id: NonZeroU64::new(record_id).unwrap(),
        occurred_at: at,
    }
}

/// A notification shaped as `EvtRender` writes it: single quotes and no white space between
/// elements.
fn notification_xml(
    event_id: u32,
    record_id: u64,
    created_at: Timestamp,
    user_id: &str,
    notification: u32,
    subscriber: &str,
) -> String {
    let created_at = created_at.strftime("%Y-%m-%dT%H:%M:%S%.7fZ");
    format!(
        "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='Microsoft-Windows-Winlogon' Guid='{{dbe9b383-7cf3-4331-91cc-a3cb16a3b538}}'/><EventID>{event_id}</EventID><Version>0</Version><Level>4</Level><Task>0</Task><Opcode>0</Opcode><Keywords>0x2000200000000000</Keywords><TimeCreated SystemTime='{created_at}'/><EventRecordID>{record_id}</EventRecordID><Correlation/><Execution ProcessID='1164' ThreadID='1432'/><Channel>Microsoft-Windows-Winlogon/Operational</Channel><Computer>DESKTOP</Computer><Security UserID='{user_id}'/></System><EventData><Data Name='Event'>{notification}</Data><Data Name='SubscriberName'>{subscriber}</Data></EventData></Event>"
    )
}

/// An unlock as a real log rendered it, its computer and user renamed.
const RENDERED_UNLOCK: &str = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='Microsoft-Windows-Winlogon' Guid='{dbe9b383-7cf3-4331-91cc-a3cb16a3b538}'/><EventID>811</EventID><Version>0</Version><Level>4</Level><Task>811</Task><Opcode>1</Opcode><Keywords>0x4000000000010000</Keywords><TimeCreated SystemTime='2026-10-26T13:59:31.2724829Z'/><EventRecordID>34811</EventRecordID><Correlation/><Execution ProcessID='1776' ThreadID='2112'/><Channel>Microsoft-Windows-Winlogon/Operational</Channel><Computer>DESKTOP.example.com</Computer><Security UserID='S-1-5-21-1004336348-1177238915-682003330-1001'/></System><EventData><Data Name='Event'>5</Data><Data Name='SubscriberName'>Sens</Data></EventData></Event>";

#[test]
fn a_rendered_unlock_parses_at_its_time_created() {
    assert_eq!(
        event_xml::parse_notification(RENDERED_UNLOCK),
        Some(Some(event_xml::Notification {
            user_id: USER.to_owned(),
            boundary: boundary(
                Unlock,
                34811,
                at(13, 59, 31_272) + SignedDuration::from_nanos(482_900)
            ),
        }))
    );
}

fn started(record_id: u64, at: Timestamp, notification: u32, subscriber: &str) -> String {
    notification_xml(811, record_id, at, USER, notification, subscriber)
}

#[test]
fn a_started_notification_parses_as_the_boundary_of_its_user() {
    let parsed = event_xml::parse_notification(&started(5120, at(8, 32, 15), 5, "Sens"));

    assert_eq!(
        parsed,
        Some(Some(event_xml::Notification {
            user_id: USER.to_owned(),
            boundary: boundary(Unlock, 5120, at(8, 32, 15)),
        }))
    );
}

#[test]
fn notifications_map_to_logon_logoff_lock_and_unlock() {
    let kinds = [2, 12, 3, 13, 4, 8, 5, 9].map(|notification| {
        event_xml::parse_notification(&started(1, at(9, 0, 0), notification, "TermSrv"))
            .flatten()
            .map(|parsed| parsed.boundary.kind)
    });

    assert_eq!(
        kinds,
        [Logon, Logon, Logoff, Logoff, Lock, Lock, Unlock, Unlock].map(Some)
    );
}

#[test]
fn other_notifications_and_events_parse_as_no_boundary_and_malformed_records_as_nothing() {
    for notification in [0, 1, 6, 7, 10, 11] {
        assert_eq!(
            event_xml::parse_notification(&started(1, at(9, 0, 0), notification, "Sens")),
            Some(None),
            "notification {notification}"
        );
    }
    assert_eq!(
        event_xml::parse_notification(&notification_xml(812, 1, at(9, 0, 0), USER, 4, "Sens")),
        Some(None)
    );

    let well_formed = started(1, at(9, 0, 0), 4, "Sens");
    for malformed in [
        well_formed.replace("<EventRecordID>1<", "<EventRecordID>0<"),
        well_formed.replace("<Data Name='Event'>4</Data>", ""),
        well_formed.replace("<Data Name='Event'>4<", "<Data Name='Event'>four<"),
        well_formed.replace("<Security UserID='", "<Security Other='"),
        well_formed.replace("</Event>", ""),
    ] {
        assert_eq!(
            event_xml::parse_notification(&malformed),
            None,
            "{malformed}"
        );
    }
}

#[test]
fn each_subscriber_logging_one_change_collapses_into_its_first_record() {
    let collapsed = collapse([
        boundary(Lock, 12, at(10, 0, 4)),
        boundary(Lock, 11, at(10, 0, 0)),
        boundary(Unlock, 14, at(10, 20, 0)),
        boundary(Lock, 13, at(10, 0, 9)),
        boundary(Unlock, 15, at(10, 20, 3)),
        boundary(Lock, 16, at(11, 0, 0)),
    ]);

    assert_eq!(
        collapsed,
        [
            boundary(Lock, 11, at(10, 0, 0)),
            boundary(Unlock, 14, at(10, 20, 0)),
            boundary(Lock, 16, at(11, 0, 0)),
        ]
    );
}

/// A log that answers with fixed records, newest first, and remembers what it was asked.
struct FakeLog {
    read: Result<CollectedRecords, LogUnavailable>,
    asked_since: Cell<Option<Timestamp>>,
}

impl FakeLog {
    fn holding(records: Vec<String>) -> Self {
        Self::answering(Ok(CollectedRecords::Complete(records)))
    }

    fn answering(read: Result<CollectedRecords, LogUnavailable>) -> Self {
        Self {
            read,
            asked_since: Cell::new(None),
        }
    }
}

impl WinlogonEventLog for &FakeLog {
    fn user_id(&self) -> &str {
        USER
    }

    fn read(
        &self,
        since: Timestamp,
        _record_limit: usize,
    ) -> Result<CollectedRecords, LogUnavailable> {
        self.asked_since.set(Some(since));
        self.read.clone()
    }
}

fn morning() -> UtcInterval {
    UtcInterval::new(at(0, 0, 0), at(14, 5, 0)).unwrap()
}

#[test]
fn the_reader_returns_the_morning_of_its_user_and_the_state_before_it() {
    let mut records = vec![
        started(90, at(-6, 0, 0), 4, "Sens"),
        started(91, at(-6, 0, 2), 4, "TermSrv"),
        started(100, at(8, 32, 0), 5, "Sens"),
        started(101, at(8, 32, 1), 5, "TermSrv"),
        notification_xml(102, 102, at(8, 32, 1), USER, 5, "Sens"),
        notification_xml(811, 103, at(9, 0, 0), OTHER_USER, 4, "Sens"),
        started(110, at(12, 10, 0), 4, "Sens"),
        started(111, at(12, 10, 0), 6, "Sens"),
        started(120, at(13, 20, 0), 5, "Sens"),
        started(130, at(14, 6, 0), 4, "Sens"),
    ];
    records.reverse();
    let log = FakeLog::holding(records);

    let read = SessionHistoryReader::new(&log, RECORD_LIMIT).read(morning());

    assert_eq!(log.asked_since.get(), Some(at(0, 0, 0) - LOOKBACK));
    assert_eq!(
        read,
        SessionHistoryRead::Complete(SessionHistory {
            before_window: Some(boundary(Lock, 90, at(-6, 0, 0))),
            in_window: vec![
                boundary(Unlock, 100, at(8, 32, 0)),
                boundary(Lock, 110, at(12, 10, 0)),
                boundary(Unlock, 120, at(13, 20, 0)),
            ],
        })
    );
}

#[test]
fn a_boundary_older_than_the_lookback_does_not_set_the_state_at_the_window_start() {
    let log = FakeLog::holding(vec![
        started(100, at(8, 32, 0), 5, "Sens"),
        started(90, at(-30, 0, 0), 4, "Sens"),
    ]);

    let read = SessionHistoryReader::new(&log, RECORD_LIMIT).read(morning());

    assert_eq!(
        read,
        SessionHistoryRead::Complete(SessionHistory {
            before_window: None,
            in_window: vec![boundary(Unlock, 100, at(8, 32, 0))],
        })
    );
}

#[test]
fn a_truncated_malformed_or_unreadable_log_returns_no_history_and_its_reason() {
    let reads = [
        FakeLog::answering(Ok(CollectedRecords::Truncated)),
        FakeLog::holding(vec![
            started(100, at(8, 32, 0), 5, "Sens"),
            "<Event>".to_owned(),
        ]),
        FakeLog::answering(Err(LogUnavailable)),
    ]
    .map(|log| SessionHistoryReader::new(&log, RECORD_LIMIT).read(morning()));

    assert_eq!(
        reads.each_ref().map(SessionHistoryRead::reason),
        [
            Some("windows-winlogon-log-record-limit"),
            Some("windows-winlogon-log-malformed-event"),
            Some("windows-winlogon-log-unavailable"),
        ]
    );
}

mod recovery {
    use std::time::Duration;

    use bientot_domain::accounting::{
        AccountingInput, AccountingLabel, AccountingPolicy, AccountingResult, ActivityCoverage,
        ActivitySegment, ActivityState, AutomaticOrigin, OperationKey, SegmentKind, SessionState,
        WorkdayTransition, WorkdayTransitionKind, calculate,
    };
    use bientot_domain::time::{FrozenClock, LocalWorkday, WorkdayWindow};
    use bientot_store::sessions::SessionEventRepository;
    use rusqlite::Connection;
    use tempfile::TempDir;

    use super::*;

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

        fn event_count(&self) -> i64 {
            self.connection
                .query_row("SELECT COUNT(*) FROM SessionEvent", [], |row| row.get(0))
                .unwrap()
        }
    }

    /// The workday Bientôt is installed in, at 14:05.
    fn window() -> WorkdayWindow {
        WorkdayWindow {
            workday: LocalWorkday::new(date(2026, 10, 26), "UTC"),
            start: at(0, 0, 0),
            end: at(24, 0, 0),
        }
    }

    fn installed() -> Timestamp {
        at(14, 5, 0)
    }

    fn recovery<'a>(database: &'a Database, clock: &'a FrozenClock) -> SessionHistoryRecovery<'a> {
        SessionHistoryRecovery::new(
            SessionEventRepository::new(&database.connection, clock),
            SessionHistoryRecovery::READ_TIMEOUT,
        )
    }

    fn morning() -> SessionHistoryRead {
        SessionHistoryRead::Complete(SessionHistory {
            before_window: None,
            in_window: vec![
                boundary(Logon, 100, at(8, 32, 0)),
                boundary(Lock, 110, at(12, 10, 0)),
                boundary(Unlock, 120, at(13, 20, 0)),
            ],
        })
    }

    /// The day at 15:05, Bientôt having watched the session from 14:05 and found it active.
    fn account(database: &Database, clock: &FrozenClock) -> AccountingResult {
        let events = SessionEventRepository::new(&database.connection, clock);
        events
            .record(
                OperationKey::new("lifecycle:startup").unwrap(),
                WorkdayTransition::new(
                    installed(),
                    "UTC",
                    WorkdayTransitionKind::SessionObserved(SessionState::Unlocked),
                )
                .unwrap(),
            )
            .unwrap();
        let history = events.read_for_accounting(&window()).unwrap();
        let watched = UtcInterval::new(installed(), at(15, 5, 0)).unwrap();
        calculate(
            &AccountingInput::new(
                window(),
                at(15, 5, 0),
                AccountingPolicy::new(
                    UtcInterval::new(at(12, 0, 0), at(13, 0, 0)).unwrap(),
                    SignedDuration::from_mins(90),
                )
                .unwrap(),
                ActivityCoverage::new(
                    UtcInterval::new(at(0, 0, 0), at(15, 5, 0)).unwrap(),
                    [ActivitySegment::new(
                        watched,
                        ActivityState::Active,
                        AutomaticOrigin::new("built-in").unwrap(),
                    )],
                ),
                history,
                Vec::new(),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn rebuilt(result: &AccountingResult) -> Vec<(UtcInterval, bool)> {
        result
            .timeline()
            .segments()
            .iter()
            .filter(|segment| segment.label() == AccountingLabel::RebuiltWork)
            .map(|segment| {
                (
                    segment.interval(),
                    matches!(segment.kind(), SegmentKind::ProvisionalCredit { .. }),
                )
            })
            .collect()
    }

    #[test]
    fn the_recovered_morning_counts_provisionally_once_whatever_the_retries() {
        let database = Database::new();
        let clock = FrozenClock::new(installed());
        let read = recovery(&database, &clock).recover(&window(), installed(), |interval| {
            // A failed assertion reads as a failed recovery, which `read` would not equal.
            assert_eq!(
                interval,
                UtcInterval::new(at(0, 0, 0), installed()).unwrap()
            );
            morning()
        });
        let rows = database.event_count();
        recovery(&database, &clock).recover(&window(), installed(), |_| morning());

        assert_eq!(read, morning());
        assert_eq!(rows, 3);
        assert_eq!(database.event_count(), rows);
        let interval = |start: (i64, i64), end: (i64, i64)| {
            UtcInterval::new(at(start.0, start.1, 0), at(end.0, end.1, 0)).unwrap()
        };
        assert_eq!(
            rebuilt(&account(&database, &clock)),
            [
                (interval((8, 32), (12, 0)), true),
                (interval((13, 20), (14, 5)), true),
            ]
        );
    }

    #[test]
    fn a_logoff_leaves_the_session_unknown() {
        let database = Database::new();
        let clock = FrozenClock::new(installed());
        recovery(&database, &clock).recover(&window(), installed(), |_| {
            SessionHistoryRead::Complete(SessionHistory {
                before_window: None,
                in_window: vec![
                    boundary(Logon, 100, at(8, 0, 0)),
                    boundary(Logoff, 110, at(9, 0, 0)),
                ],
            })
        });

        let rebuilt = rebuilt(&account(&database, &clock));

        assert_eq!(
            rebuilt,
            [(UtcInterval::new(at(8, 0, 0), at(9, 0, 0)).unwrap(), true)]
        );
    }

    #[test]
    fn a_read_that_fails_times_out_or_panics_records_nothing() {
        let database = Database::new();
        let clock = FrozenClock::new(installed());
        let slow = SessionHistoryRecovery::new(
            SessionEventRepository::new(&database.connection, &clock),
            Duration::from_millis(10),
        );

        let reads = [
            recovery(&database, &clock)
                .recover(&window(), installed(), |_| SessionHistoryRead::Truncated),
            slow.recover(&window(), installed(), |_| {
                std::thread::sleep(Duration::from_millis(200));
                morning()
            }),
            recovery(&database, &clock)
                .recover(&window(), installed(), |_| panic!("the log reader failed")),
        ];

        assert_eq!(
            reads.each_ref().map(SessionHistoryRead::reason),
            [
                Some("windows-winlogon-log-record-limit"),
                Some("windows-winlogon-log-timeout"),
                Some("windows-session-history-recovery-failed"),
            ]
        );
        assert_eq!(database.event_count(), 0);
    }
}
