use jiff::civil::date;

use super::*;

use SessionBoundaryKind::{Lock, Logoff, Logon, Unlock};

fn at(hour: i64, minute: i64, millisecond: i64) -> Timestamp {
    date(2026, 10, 26)
        .at(0, 0, 0, 0)
        .to_zoned_datetime(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
        + SignedDuration::from_hours(hour)
        + SignedDuration::from_mins(minute)
        + SignedDuration::from_millis(millisecond)
}

fn boundary(kind: SessionBoundaryKind, record_id: u64, at: Timestamp) -> SessionBoundary {
    SessionBoundary {
        kind,
        at,
        source_key: record_id.to_string(),
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
