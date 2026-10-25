use std::cell::Cell;
use std::time::Instant;

use bientot_domain::accounting::{
    AccountingInput, AccountingPolicy, AccountingResult, ActivityCoverage, ActivitySegment,
    ActivityState, AutomaticOrigin, RecordedTransition, SegmentKind, WorkdayEventHistory,
    calculate,
};
use bientot_domain::time::{FrozenClock, LocalWorkday};
use jiff::civil::date;
use jiff::{SignedDuration, Timestamp};
use rusqlite::Connection;
use tempfile::TempDir;

use super::*;

use PowerHistoryBoundaryKind::{Boot, Resume, Shutdown, Suspend};

fn at_ms(hour: i64, minute: i64, second: i64, millisecond: i64) -> Timestamp {
    date(2026, 9, 7)
        .at(0, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
        + SignedDuration::from_hours(hour)
        + SignedDuration::from_mins(minute)
        + SignedDuration::from_secs(second)
        + SignedDuration::from_millis(millisecond)
}

fn at(hour: i64) -> Timestamp {
    at_ms(hour, 0, 0, 0)
}

fn boundary(kind: PowerHistoryBoundaryKind, record_id: u64, at: Timestamp) -> PowerHistoryBoundary {
    PowerHistoryBoundary {
        kind,
        record_id: NonZeroU64::new(record_id).unwrap(),
        occurred_at: at,
    }
}

fn interval(
    kind: PowerHistoryIntervalKind,
    start: PowerHistoryBoundary,
    end: PowerHistoryBoundary,
) -> PowerHistoryInterval {
    PowerHistoryInterval::new(kind, start, end).unwrap()
}

fn powered_off(shutdown: (u64, Timestamp), boot: (u64, Timestamp)) -> PowerHistoryInterval {
    interval(
        PowerHistoryIntervalKind::PoweredOff,
        boundary(Shutdown, shutdown.0, shutdown.1),
        boundary(Boot, boot.0, boot.1),
    )
}

/// An event as the .NET tests write it: double quotes, the event namespace, and at most one
/// `Data` element.
fn event_xml(
    provider: &str,
    event_id: u32,
    record_id: u64,
    created_at: &str,
    data: Option<(&str, &str)>,
) -> String {
    let data = data
        .map(|(name, value)| format!(r#"<Data Name="{name}">{value}</Data>"#))
        .unwrap_or_default();
    format!(
        r#"<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event">
  <System>
    <Provider Name="{provider}" />
    <EventID>{event_id}</EventID>
    <EventRecordID>{record_id}</EventRecordID>
    <TimeCreated SystemTime="{created_at}" />
  </System>
  <EventData>{data}</EventData>
</Event>"#
    )
}

/// A boot event shaped as `EvtRender` writes it: single quotes, no white space between
/// elements, and every field of the event data.
const RENDERED_BOOT: &str = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='Microsoft-Windows-Kernel-General' Guid='{a68ca8b7-004f-d7b6-a698-07e2de0f1f5d}'/><EventID>12</EventID><Version>2</Version><Level>4</Level><Task>1</Task><Opcode>0</Opcode><Keywords>0x8000000000000080</Keywords><TimeCreated SystemTime='2026-09-07T06:58:03.5418117Z'/><EventRecordID>48211</EventRecordID><Correlation/><Execution ProcessID='4' ThreadID='8'/><Channel>System</Channel><Computer>DESK&amp;TOP</Computer><Security UserID='S-1-5-18'/></System><EventData><Data Name='MajorVersion'>10</Data><Data Name='MinorVersion'>0</Data><Data Name='BuildVersion'>26100</Data><Data Name='QfeVersion'>4061</Data><Data Name='ServiceVersion'>0</Data><Data Name='BootMode'>0</Data><Data Name='StartTime'>2026-09-07T06:57:59.500000000Z</Data></EventData></Event>";

#[test]
fn a_rendered_boot_takes_its_instant_from_the_start_time_of_its_event_data() {
    assert_eq!(
        event_xml::parse_boundary(RENDERED_BOOT),
        Some(boundary(Boot, 48211, at_ms(6, 57, 59, 500)))
    );
    assert_eq!(
        event_xml::parse_boundary(&event_xml(
            "Microsoft-Windows-Kernel-General",
            12,
            701,
            "2026-09-07T16:20:12.0800000Z",
            Some(("StartTime", "2026-09-07T16:20:11.5000000Z")),
        )),
        Some(boundary(Boot, 701, at_ms(16, 20, 11, 500)))
    );
}

#[test]
fn a_shutdown_takes_its_instant_from_the_stop_time_and_suspend_and_resume_from_time_created() {
    let parsed = [
        event_xml(
            "Microsoft-Windows-Kernel-General",
            13,
            700,
            "2026-09-07T16:20:12Z",
            Some(("StopTime", "2026-09-07T08:00:00.0000000+00:00")),
        ),
        event_xml(
            "Microsoft-Windows-Kernel-Power",
            42,
            744,
            "2026-09-07T12:34:56.7890000Z",
            None,
        ),
        event_xml(
            "Microsoft-Windows-Kernel-Power",
            107,
            809,
            "2026-09-07T12:34:56.789",
            None,
        ),
    ]
    .map(|xml| event_xml::parse_boundary(&xml));

    assert_eq!(
        parsed,
        [
            Some(boundary(Shutdown, 700, at(8))),
            Some(boundary(Suspend, 744, at_ms(12, 34, 56, 789))),
            Some(boundary(Resume, 809, at_ms(12, 34, 56, 789))),
        ]
    );
}

#[test]
fn malformed_records_and_records_of_other_events_parse_as_nothing() {
    let suspend = event_xml(
        "Microsoft-Windows-Kernel-Power",
        42,
        5,
        "2026-09-07T12:00:00Z",
        None,
    );
    let boot = |data| {
        event_xml(
            "Microsoft-Windows-Kernel-General",
            12,
            5,
            "2026-09-07T12:00:00Z",
            data,
        )
    };
    let malformed = [
        "<Event><broken /></Event>".to_owned(),
        suspend.replace("</Event>", ""),
        suspend.replace("</System>", "</Sys>"),
        suspend.replace("<EventData>", "<System/><EventData>"),
        suspend.replace("<EventID>42", "<EventID>+42"),
        suspend.replace("<EventID>42", "<EventID> 42"),
        suspend.replace("<EventID>42", "<EventID>41"),
        suspend.replace("<EventRecordID>5", "<EventRecordID>0"),
        suspend.replace("Kernel-Power", "Kernel-Processor-Power"),
        suspend.replace("2026-09-07T12:00:00Z", "noon"),
        suspend.replace(r#" />"#, r#" Name="again" />"#),
        suspend.replace("Name=\"", "Name=\"&nbsp;"),
        format!("<!DOCTYPE Event>{suspend}"),
        format!("{suspend}<Event/>"),
        boot(None),
        boot(Some(("StopTime", "2026-09-07T11:00:00Z"))),
        boot(Some(("StartTime", "2026-09-07T11:00:00Z"))).replace(
            "</EventData>",
            r#"<Data Name="StartTime">2026-09-07T10:00:00Z</Data></EventData>"#,
        ),
    ];

    for xml in malformed {
        assert_eq!(event_xml::parse_boundary(&xml), None, "{xml}");
    }
}

#[test]
fn comments_declarations_cdata_and_references_read_as_xml_does() {
    let xml = "\u{feff}<?xml version='1.0'?><!-- recorded --><e:Event xmlns:e='urn:event'>\
        <e:System><e:Provider Name='Microsoft&#45;Windows-Kernel&#x2D;Power'/>\
        <e:EventID><![CDATA[1]]>07</e:EventID><e:EventRecordID>9<!-- note -->1</e:EventRecordID>\
        <e:TimeCreated SystemTime=\"2026-09-07T12:00:00Z\"/></e:System></e:Event>";

    assert_eq!(
        event_xml::parse_boundary(xml),
        Some(boundary(Resume, 91, at(12)))
    );
}

#[test]
fn the_pairer_returns_verified_off_and_suspend_intervals_from_unsorted_input() {
    let shutdown = boundary(Shutdown, 10, at(8));
    let boot = boundary(Boot, 11, at(9));
    let suspend = boundary(Suspend, 12, at(10));
    let resume = boundary(Resume, 13, at(11));

    let history = pair([resume, shutdown, suspend, boot]);

    assert_eq!(
        history,
        PowerHistory {
            intervals: vec![
                interval(PowerHistoryIntervalKind::PoweredOff, shutdown, boot),
                interval(PowerHistoryIntervalKind::Suspended, suspend, resume),
            ],
            boots: vec![boot],
        }
    );
}

#[test]
fn missing_and_conflicting_boundaries_do_not_create_intervals() {
    let history = pair([
        boundary(Boot, 1, at(7)),
        boundary(Shutdown, 2, at(8)),
        boundary(Shutdown, 3, at_ms(8, 30, 0, 0)),
        boundary(Boot, 4, at(9)),
        boundary(Suspend, 5, at(10)),
        boundary(Suspend, 6, at_ms(10, 30, 0, 0)),
        boundary(Resume, 7, at(11)),
    ]);

    assert_eq!(history.intervals, []);
}

#[test]
fn a_boot_breaks_an_open_suspend_pair() {
    let history = pair([
        boundary(Suspend, 1, at(8)),
        boundary(Boot, 2, at(9)),
        boundary(Resume, 3, at(10)),
    ]);

    assert_eq!(history.intervals, []);
}

#[test]
fn a_conflict_at_one_instant_cannot_close_an_earlier_shutdown() {
    let history = pair([
        boundary(Shutdown, 1, at(8)),
        boundary(Boot, 2, at(9)),
        boundary(Suspend, 3, at(9)),
    ]);

    assert_eq!(history, PowerHistory::default());
}

#[test]
fn a_conflict_at_one_instant_also_spoils_a_later_shutdown_until_the_next_boot() {
    let history = pair([
        boundary(Shutdown, 1, at(8)),
        boundary(Suspend, 2, at(8)),
        boundary(Shutdown, 3, at(9)),
        boundary(Boot, 4, at(10)),
        boundary(Shutdown, 5, at(11)),
        boundary(Boot, 6, at(12)),
    ]);

    assert_eq!(history.intervals, [powered_off((5, at(11)), (6, at(12)))]);
}

#[test]
fn a_standalone_boot_is_returned_without_fabricating_off_time() {
    let boot = boundary(Boot, 2, at(9));

    assert_eq!(
        pair([boot]),
        PowerHistory {
            intervals: vec![],
            boots: vec![boot],
        }
    );
}

#[test]
fn an_exact_duplicate_is_harmless_but_a_reused_record_id_at_another_instant_is_distinct() {
    let first = boundary(Shutdown, 90, at(8));
    let boot = boundary(Boot, 91, at(9));
    let reused = boundary(Shutdown, 90, at(10));
    let second_boot = boundary(Boot, 91, at(11));

    let history = pair([first, first, boot, reused, second_boot]);

    assert_eq!(
        history
            .intervals
            .iter()
            .map(|interval| interval.start().occurred_at)
            .collect::<Vec<_>>(),
        [at(8), at(10)]
    );
}

#[test]
fn an_interval_refuses_reversed_or_mismatched_boundaries() {
    let shutdown = boundary(Shutdown, 90, at(9));
    let boot = boundary(Boot, 91, at(8));

    assert_eq!(
        PowerHistoryInterval::new(PowerHistoryIntervalKind::PoweredOff, shutdown, boot),
        None
    );
    assert_eq!(
        PowerHistoryInterval::new(
            PowerHistoryIntervalKind::Suspended,
            boundary(Shutdown, 1, at(8)),
            boundary(Boot, 2, at(9)),
        ),
        None
    );
}

/// A log that answers every read with `read`, after logging the record limit it was given.
struct StubLog<F> {
    read: F,
    limits: Cell<Vec<usize>>,
}

impl<F: Fn() -> Result<CollectedRecords, SystemLogUnavailable>> StubLog<F> {
    fn new(read: F) -> Self {
        Self {
            read,
            limits: Cell::new(Vec::new()),
        }
    }
}

impl<F: Fn() -> Result<CollectedRecords, SystemLogUnavailable>> SystemEventLog for &StubLog<F> {
    fn read(&self, record_limit: usize) -> Result<CollectedRecords, SystemLogUnavailable> {
        let mut limits = self.limits.take();
        limits.push(record_limit);
        self.limits.set(limits);
        (self.read)()
    }
}

fn hours(start: i64, end: i64) -> UtcInterval {
    UtcInterval::new(at(start), at(end)).unwrap()
}

#[test]
fn the_reader_pairs_a_long_off_interval_without_a_calendar_cutoff() {
    let month_ago = at(0) - SignedDuration::from_hours(30 * 24);
    let log = StubLog::new(|| {
        let month_ago = month_ago.to_string();
        Ok(CollectedRecords::Complete(vec![
            event_xml(
                "Microsoft-Windows-Kernel-General",
                13,
                20,
                &month_ago,
                Some(("StopTime", &month_ago)),
            ),
            event_xml(
                "Microsoft-Windows-Kernel-General",
                12,
                21,
                &at(8).to_string(),
                Some(("StartTime", &at(8).to_string())),
            ),
        ]))
    });

    let read = PowerHistoryReader::new(&log, 32).read(hours(0, 12));

    assert_eq!(
        read,
        PowerHistoryRead::Complete(PowerHistory {
            intervals: vec![powered_off((20, month_ago), (21, at(8)))],
            boots: vec![boundary(Boot, 21, at(8))],
        })
    );
    assert_eq!(log.limits.take(), [32]);
}

#[test]
fn the_reader_pairs_authoritative_times_despite_reversed_log_order() {
    let log = StubLog::new(|| {
        Ok(CollectedRecords::Complete(vec![
            event_xml(
                "Microsoft-Windows-Kernel-General",
                12,
                21,
                &at(-1).to_string(),
                Some(("StartTime", &at(9).to_string())),
            ),
            event_xml(
                "Microsoft-Windows-Kernel-General",
                13,
                20,
                &at(13).to_string(),
                Some(("StopTime", &at(8).to_string())),
            ),
        ]))
    });

    let read = PowerHistoryReader::new(&log, 32).read(hours(0, 12));

    assert_eq!(
        read,
        PowerHistoryRead::Complete(PowerHistory {
            intervals: vec![powered_off((20, at(8)), (21, at(9)))],
            boots: vec![boundary(Boot, 21, at(9))],
        })
    );
}

#[test]
fn the_reader_keeps_what_overlaps_the_window_and_the_boots_inside_it() {
    let log = StubLog::new(|| {
        let kernel_power = |event_id, record_id, hour| {
            event_xml(
                "Microsoft-Windows-Kernel-Power",
                event_id,
                record_id,
                &at(hour).to_string(),
                None,
            )
        };
        let boot = |record_id, hour| {
            event_xml(
                "Microsoft-Windows-Kernel-General",
                12,
                record_id,
                &at(hour).to_string(),
                Some(("StartTime", &at(hour).to_string())),
            )
        };
        Ok(CollectedRecords::Complete(vec![
            kernel_power(42, 1, -3),
            kernel_power(107, 2, 0),
            boot(3, 1),
            kernel_power(42, 4, 11),
            kernel_power(107, 5, 13),
            boot(6, 12),
        ]))
    });

    let read = PowerHistoryReader::new(&log, 32).read(hours(0, 12));

    assert_eq!(
        read,
        PowerHistoryRead::Complete(PowerHistory {
            intervals: vec![],
            boots: vec![boundary(Boot, 3, at(1))],
        })
    );
    let PowerHistoryRead::Complete(history) = PowerHistoryReader::new(&log, 32).read(hours(0, 13))
    else {
        panic!("the log is complete");
    };
    assert_eq!(history.boots.len(), 2);
    assert_eq!(history.intervals, []);
}

#[test]
fn the_collector_tells_the_end_of_the_log_from_the_record_cap() {
    fn reading(records: &[&str]) -> impl FnMut() -> Result<Option<String>, SystemLogUnavailable> {
        let mut records = records
            .iter()
            .map(|record| record.to_string())
            .collect::<Vec<_>>()
            .into_iter();
        move || Ok(records.next())
    }

    assert_eq!(
        collect(reading(&[]), 2),
        Ok(CollectedRecords::Complete(vec![]))
    );
    assert_eq!(
        collect(reading(&["record-1"]), 1),
        Ok(CollectedRecords::Complete(vec!["record-1".into()]))
    );
    assert_eq!(
        collect(reading(&["record-2", "record-1"]), 1),
        Ok(CollectedRecords::Truncated)
    );
    assert_eq!(
        collect(|| Err::<Option<String>, _>(SystemLogUnavailable), 4),
        Err(SystemLogUnavailable)
    );
}

#[test]
fn a_truncated_malformed_or_unreadable_log_returns_no_evidence_and_its_reason() {
    let truncated = StubLog::new(|| Ok(CollectedRecords::Truncated));
    let malformed = StubLog::new(|| {
        Ok(CollectedRecords::Complete(vec![
            "<Event><broken /></Event>".into(),
        ]))
    });
    let denied = StubLog::new(|| Err(SystemLogUnavailable));

    let reads = [
        PowerHistoryReader::new(&truncated, 8).read(hours(0, 12)),
        PowerHistoryReader::new(&malformed, 8).read(hours(0, 12)),
        PowerHistoryReader::new(&denied, 8).read(hours(0, 12)),
    ];

    assert_eq!(
        reads.each_ref().map(PowerHistoryRead::reason),
        [
            Some("windows-system-log-record-limit"),
            Some("windows-system-log-malformed-event"),
            Some("windows-system-log-unavailable"),
        ]
    );
    assert_eq!(
        reads,
        [
            PowerHistoryRead::Truncated,
            PowerHistoryRead::Unavailable(PowerHistoryUnavailable::MalformedEvent),
            PowerHistoryRead::Unavailable(PowerHistoryUnavailable::LogUnavailable),
        ]
    );
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

    fn event_count(&self) -> i64 {
        self.connection
            .query_row("SELECT COUNT(*) FROM SessionEvent", [], |row| row.get(0))
            .unwrap()
    }

    /// Operation key, instant and kind of every stored event, in insertion order.
    fn events(&self) -> Vec<(String, String, String)> {
        self.connection
            .prepare("SELECT OperationKey, OccurredAtUtc, Kind FROM SessionEvent ORDER BY Id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}

/// The workday whose summary is calculated at 10:00.
fn window() -> WorkdayWindow {
    WorkdayWindow {
        workday: LocalWorkday::new(date(2026, 9, 7), "UTC"),
        start: at(0),
        end: at(24),
    }
}

fn now() -> FrozenClock {
    FrozenClock::new(at(10))
}

fn recovery<'a>(database: &'a Database, clock: &'a FrozenClock) -> PowerHistoryRecovery<'a> {
    PowerHistoryRecovery::new(
        SessionEventRepository::new(&database.connection, clock),
        PowerHistoryRecovery::READ_TIMEOUT,
    )
}

fn complete(intervals: Vec<PowerHistoryInterval>) -> PowerHistoryRead {
    PowerHistoryRead::Complete(PowerHistory {
        intervals,
        boots: vec![],
    })
}

fn returning(read: PowerHistoryRead) -> impl FnOnce(UtcInterval) -> PowerHistoryRead + Send {
    move |_| read
}

fn record(
    database: &Database,
    clock: &FrozenClock,
    key: &str,
    at: Timestamp,
    kind: WorkdayTransitionKind,
) {
    SessionEventRepository::new(&database.connection, clock)
        .record(OperationKey::new(key).unwrap(), utc(at, kind))
        .unwrap();
}

fn history(database: &Database, clock: &FrozenClock) -> WorkdayEventHistory {
    SessionEventRepository::new(&database.connection, clock)
        .read_for_accounting(&window())
        .unwrap()
}

fn account(history: WorkdayEventHistory, activity: ActivityCoverage) -> AccountingResult {
    calculate(
        &AccountingInput::new(
            window(),
            at(10),
            AccountingPolicy::new(hours(11, 12), SignedDuration::from_mins(90)).unwrap(),
            activity,
            history,
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn without_activity() -> ActivityCoverage {
    ActivityCoverage::unknown(
        hours(0, 10),
        AutomaticOrigin::new("activity:missing").unwrap(),
    )
}

/// Excluded and unknown work, the figures the recovery changes.
fn excluded_and_unknown(result: &AccountingResult) -> (SignedDuration, SignedDuration) {
    (
        result.timeline().excluded_work(),
        result.timeline().unknown_work(),
    )
}

fn provisional_intervals(result: &AccountingResult) -> Vec<UtcInterval> {
    result
        .timeline()
        .segments()
        .iter()
        .filter(|segment| matches!(segment.kind(), SegmentKind::ProvisionalCredit { .. }))
        .map(|segment| segment.interval())
        .collect()
}

#[test]
fn recovered_power_excludes_the_off_time_and_leaves_observed_activity_credited() {
    let database = Database::new();
    let clock = now();
    let activity = || {
        ActivityCoverage::new(
            hours(0, 10),
            [ActivitySegment::new(
                UtcInterval::new(at_ms(8, 7, 0, 0), at_ms(9, 59, 57, 500)).unwrap(),
                ActivityState::Active,
                AutomaticOrigin::new("activitywatch:test").unwrap(),
            )],
        )
    };
    let off = complete(vec![powered_off((400, at(-1)), (401, at(8)))]);
    let before = account(history(&database, &clock), activity());

    recovery(&database, &clock)
        .recover(&window(), returning(off.clone()))
        .unwrap();
    let after = account(history(&database, &clock), activity());
    let rows = database.event_count();
    recovery(&database, &clock)
        .recover(&window(), returning(off))
        .unwrap();
    let repeated = account(history(&database, &clock), activity());

    let credited = SignedDuration::from_mins(113) - SignedDuration::from_millis(2500);
    let after_boot = SignedDuration::from_mins(7) + SignedDuration::from_millis(2500);
    assert_eq!(
        excluded_and_unknown(&before),
        (
            SignedDuration::ZERO,
            SignedDuration::from_hours(8) + after_boot
        )
    );
    assert_eq!(
        excluded_and_unknown(&after),
        (SignedDuration::from_hours(8), after_boot)
    );
    assert_eq!(
        [
            before.credited_work(),
            after.credited_work(),
            repeated.credited_work()
        ],
        [credited; 3]
    );
    assert_eq!(
        excluded_and_unknown(&repeated),
        excluded_and_unknown(&after)
    );
    assert_eq!(database.event_count(), rows);
}

#[test]
fn recovery_before_the_first_summary_excludes_only_the_verified_off_interval() {
    let database = Database::new();
    let clock = now();
    record(
        &database,
        &clock,
        "stale-lock",
        at(-2),
        WorkdayTransitionKind::SessionObserved(SessionState::Locked),
    );
    let off = complete(vec![powered_off((400, at(-1)), (401, at(8)))]);

    recovery(&database, &clock)
        .recover(&window(), returning(off.clone()))
        .unwrap();
    let first = account(history(&database, &clock), without_activity());
    let rows = database.event_count();
    recovery(&database, &clock)
        .recover(&window(), returning(off))
        .unwrap();
    let rerun = account(history(&database, &clock), without_activity());

    assert_eq!(
        excluded_and_unknown(&first),
        (SignedDuration::from_hours(8), SignedDuration::from_hours(2))
    );
    assert_eq!(provisional_intervals(&first), []);
    assert_eq!(excluded_and_unknown(&rerun), excluded_and_unknown(&first));
    assert_eq!(database.event_count(), rows);
}

#[test]
fn reused_record_ids_at_different_instants_are_distinct_intervals_and_retry_exactly() {
    let database = Database::new();
    let clock = now();
    let read = complete(vec![
        powered_off((400, at(1)), (401, at(2))),
        powered_off((400, at(4)), (401, at(5))),
    ]);

    recovery(&database, &clock)
        .recover(&window(), returning(read.clone()))
        .unwrap();
    let rows = database.event_count();
    let first = account(history(&database, &clock), without_activity());
    recovery(&database, &clock)
        .recover(&window(), returning(read))
        .unwrap();
    let retry = account(history(&database, &clock), without_activity());

    assert_eq!(
        excluded_and_unknown(&first),
        (SignedDuration::from_hours(2), SignedDuration::from_hours(8))
    );
    assert_eq!(database.event_count(), rows);
    assert_eq!(excluded_and_unknown(&retry), excluded_and_unknown(&first));
}

#[test]
fn a_successful_retry_replaces_an_earlier_unknown_at_the_clipped_interval_start() {
    let database = Database::new();
    let clock = now();
    recovery(&database, &clock)
        .recover(
            &window(),
            returning(PowerHistoryRead::Unavailable(
                PowerHistoryUnavailable::LogUnavailable,
            )),
        )
        .unwrap();
    let off = complete(vec![powered_off((600, at(-1)), (601, at(8)))]);

    recovery(&database, &clock)
        .recover(&window(), returning(off.clone()))
        .unwrap();
    let rows = database.event_count();
    let first = account(history(&database, &clock), without_activity());
    recovery(&database, &clock)
        .recover(&window(), returning(off))
        .unwrap();
    let retry = account(history(&database, &clock), without_activity());

    assert_eq!(
        excluded_and_unknown(&first),
        (SignedDuration::from_hours(8), SignedDuration::from_hours(2))
    );
    assert_eq!(database.event_count(), rows);
    assert_eq!(excluded_and_unknown(&retry), excluded_and_unknown(&first));
}

#[test]
fn a_read_that_times_out_returns_after_recording_the_unknown_states_at_the_window_start() {
    let database = Database::new();
    let clock = now();
    let (release, hung) = std::sync::mpsc::channel::<()>();
    let recovery = PowerHistoryRecovery::new(
        SessionEventRepository::new(&database.connection, &clock),
        Duration::from_millis(50),
    );
    let started = Instant::now();

    let read = recovery
        .recover(&window(), move |_| {
            let _ = hung.recv();
            complete(vec![])
        })
        .unwrap();
    let waited = started.elapsed();
    drop(release);

    assert_eq!(read.reason(), Some("windows-system-log-timeout"));
    assert!(waited < Duration::from_secs(2), "waited {waited:?}");
    assert_eq!(database.event_count(), 2);
}

#[test]
fn a_read_that_panics_is_a_failed_recovery_and_still_records_the_unknown_states() {
    let database = Database::new();
    let clock = now();

    let read = recovery(&database, &clock)
        .recover(&window(), |_| panic!("the log reader failed"))
        .unwrap();

    assert_eq!(read.reason(), Some("windows-power-history-recovery-failed"));
    assert_eq!(database.event_count(), 2);
}

#[test]
fn a_standalone_boot_clears_a_stale_lock_without_creating_an_off_exclusion() {
    let database = Database::new();
    let clock = now();
    record(
        &database,
        &clock,
        "stale-lock",
        at(-2),
        WorkdayTransitionKind::SessionObserved(SessionState::Locked),
    );
    let boot = PowerHistoryRead::Complete(PowerHistory {
        intervals: vec![],
        boots: vec![boundary(Boot, 501, at(8))],
    });

    recovery(&database, &clock)
        .recover(&window(), returning(boot))
        .unwrap();
    let result = account(history(&database, &clock), without_activity());

    assert_eq!(
        excluded_and_unknown(&result),
        (SignedDuration::ZERO, SignedDuration::from_hours(10))
    );
    assert_eq!(provisional_intervals(&result), []);
}

#[test]
fn an_incomplete_history_clears_only_the_old_anchor_and_keeps_an_in_window_lock() {
    for incomplete in [
        PowerHistoryRead::Unavailable(PowerHistoryUnavailable::LogUnavailable),
        PowerHistoryRead::Truncated,
    ] {
        let database = Database::new();
        let clock = now();
        for (key, hour) in [("stale-lock", -2), ("observed-lock", 7)] {
            record(
                &database,
                &clock,
                key,
                at(hour),
                WorkdayTransitionKind::SessionObserved(SessionState::Locked),
            );
        }

        recovery(&database, &clock)
            .recover(&window(), returning(incomplete))
            .unwrap();
        let result = account(history(&database, &clock), without_activity());

        assert_eq!(
            excluded_and_unknown(&result),
            (SignedDuration::ZERO, SignedDuration::from_hours(7))
        );
        assert_eq!(provisional_intervals(&result), [hours(7, 10)]);
    }
}

#[test]
fn an_incomplete_history_does_not_extend_a_stale_suspend_anchor() {
    let database = Database::new();
    let clock = now();
    record(
        &database,
        &clock,
        "stale-suspend",
        at(-2),
        WorkdayTransitionKind::PowerObserved(PowerState::Suspended),
    );

    recovery(&database, &clock)
        .recover(
            &window(),
            returning(PowerHistoryRead::Unavailable(
                PowerHistoryUnavailable::LogUnavailable,
            )),
        )
        .unwrap();
    let result = account(history(&database, &clock), without_activity());

    assert_eq!(
        excluded_and_unknown(&result),
        (SignedDuration::ZERO, SignedDuration::from_hours(10))
    );
}

#[test]
fn the_unknown_states_do_not_override_real_observations_at_midnight() {
    let database = Database::new();
    let clock = now();
    record(
        &database,
        &clock,
        "midnight-lock",
        at(0),
        WorkdayTransitionKind::SessionObserved(SessionState::Locked),
    );
    record(
        &database,
        &clock,
        "midnight-suspend",
        at(0),
        WorkdayTransitionKind::PowerObserved(PowerState::Suspended),
    );

    recovery(&database, &clock)
        .recover(
            &window(),
            returning(PowerHistoryRead::Unavailable(
                PowerHistoryUnavailable::LogUnavailable,
            )),
        )
        .unwrap();
    let history = history(&database, &clock);
    let last_session = history
        .in_window()
        .iter()
        .rev()
        .find_map(|transition| match transition {
            RecordedTransition::Session(recorded) => Some(recorded.state),
            _ => None,
        });
    let last_power = history
        .in_window()
        .iter()
        .rev()
        .find_map(|transition| match transition {
            RecordedTransition::Power(recorded) => Some(recorded.state),
            _ => None,
        });
    let result = account(history, without_activity());

    assert_eq!(last_session, Some(SessionState::Locked));
    assert_eq!(last_power, Some(PowerState::Suspended));
    assert_eq!(
        result.timeline().excluded_work(),
        SignedDuration::from_hours(10)
    );
}

#[test]
fn every_recorded_boundary_is_keyed_by_provider_event_record_and_instant_as_in_dotnet() {
    let database = Database::new();
    let clock = now();
    let read = PowerHistoryRead::Complete(PowerHistory {
        intervals: vec![
            powered_off((400, at(-1)), (401, at(8))),
            interval(
                PowerHistoryIntervalKind::Suspended,
                boundary(Suspend, 402, at(9)),
                boundary(Resume, 403, at_ms(9, 30, 0, 0)),
            ),
        ],
        boots: vec![boundary(Boot, 401, at(8)), boundary(Boot, 7, at(18))],
    });

    let returned = recovery(&database, &clock)
        .recover(&window(), returning(read.clone()))
        .unwrap();

    assert_eq!(returned, read);
    let event = |key: &str, occurred_at: &str, kind: &str| {
        (key.to_owned(), occurred_at.to_owned(), kind.to_owned())
    };
    assert_eq!(
        database.events(),
        [
            event(
                "startup-knowledge:session:639243360000000000",
                "2026-09-07T00:00:00.0000000+00:00",
                "session-unknown"
            ),
            event(
                "power-history:power:suspended:microsoft-windows-kernel-general:13:400:639243324000000000",
                "2026-09-06T23:00:00.0000000+00:00",
                "suspend"
            ),
            event(
                "power-history:power:active:microsoft-windows-kernel-general:12:401:639243648000000000",
                "2026-09-07T08:00:00.0000000+00:00",
                "resume"
            ),
            event(
                "power-history:power:suspended-at-window-start:microsoft-windows-kernel-general:13:400:639243324000000000:639243360000000000",
                "2026-09-07T00:00:00.0000000+00:00",
                "suspend"
            ),
            event(
                "power-history:session:unknown:microsoft-windows-kernel-general:12:401:639243648000000000",
                "2026-09-07T08:00:00.0000000+00:00",
                "session-unknown"
            ),
            event(
                "power-history:power:suspended:microsoft-windows-kernel-power:42:402:639243684000000000",
                "2026-09-07T09:00:00.0000000+00:00",
                "suspend"
            ),
            event(
                "power-history:power:active:microsoft-windows-kernel-power:107:403:639243702000000000",
                "2026-09-07T09:30:00.0000000+00:00",
                "resume"
            ),
            event(
                "power-history:session:unknown-at-boot:microsoft-windows-kernel-general:12:7:639244008000000000",
                "2026-09-07T18:00:00.0000000+00:00",
                "session-unknown"
            ),
        ]
    );
}

#[test]
fn a_history_that_fails_to_record_is_returned_as_unavailable() {
    let database = Database::new();
    let clock = now();
    record(
        &database,
        &clock,
        "power-history:session:unknown-at-boot:microsoft-windows-kernel-general:12:7:639244008000000000",
        at(18),
        WorkdayTransitionKind::SessionObserved(SessionState::Locked),
    );
    let read = PowerHistoryRead::Complete(PowerHistory {
        intervals: vec![],
        boots: vec![boundary(Boot, 7, at(18))],
    });

    let returned = recovery(&database, &clock)
        .recover(&window(), returning(read))
        .unwrap();

    assert_eq!(
        returned.reason(),
        Some("windows-power-history-persistence-failed")
    );
    assert_eq!(database.event_count(), 3);
}
