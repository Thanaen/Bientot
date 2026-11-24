use std::cell::Cell;

use bientot_domain::intervals::UtcInterval;
use bientot_platform::power_history::{
    PowerBoundary, PowerHistory, PowerHistoryInterval, PowerHistoryIntervalKind, PowerHistoryRead,
    PowerHistoryReader, PowerHistoryUnavailable,
};
use jiff::SignedDuration;
use jiff::civil::date;

use super::*;
use crate::event_log::collect;

use PowerBoundaryKind::{Boot, Resume, Shutdown, Suspend};

fn at_ms(hour: i64, minute: i64, second: i64, millisecond: i64) -> Timestamp {
    date(2026, 9, 7)
        .at(0, 0, 0, 0)
        .to_zoned_datetime(jiff::tz::TimeZone::UTC)
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

fn boundary(kind: PowerBoundaryKind, record_id: u64, at: Timestamp) -> PowerBoundary {
    PowerBoundary {
        kind,
        at,
        source_key: source_key(kind, NonZeroU64::new(record_id).unwrap()),
    }
}

fn interval(
    kind: PowerHistoryIntervalKind,
    start: PowerBoundary,
    end: PowerBoundary,
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

    let read = PowerHistoryReader::new(SystemLogBoundaries(&log), 32).read(hours(0, 12));

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

    let read = PowerHistoryReader::new(SystemLogBoundaries(&log), 32).read(hours(0, 12));

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

    let read = PowerHistoryReader::new(SystemLogBoundaries(&log), 32).read(hours(0, 12));

    assert_eq!(
        read,
        PowerHistoryRead::Complete(PowerHistory {
            intervals: vec![],
            boots: vec![boundary(Boot, 3, at(1))],
        })
    );
    let PowerHistoryRead::Complete(history) =
        PowerHistoryReader::new(SystemLogBoundaries(&log), 32).read(hours(0, 13))
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
        PowerHistoryReader::new(SystemLogBoundaries(&truncated), 8).read(hours(0, 12)),
        PowerHistoryReader::new(SystemLogBoundaries(&malformed), 8).read(hours(0, 12)),
        PowerHistoryReader::new(SystemLogBoundaries(&denied), 8).read(hours(0, 12)),
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
