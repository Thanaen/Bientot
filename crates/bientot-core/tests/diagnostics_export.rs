//! Ported from `DiagnosticPrivacyTests.cs`: each test opens the archive the export wrote and reads
//! its entries. `archives_match_dotnet` replays `tests/fixtures/diagnostics/`, which the
//! .NET `DiagnosticExportService` wrote before the .NET app was removed, and requires the same
//! entries byte for byte.

use std::cell::Cell;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use bientot_core::diagnostics::{
    DiagnosticActivityWatchTimingSnapshot, DiagnosticArchitecture, DiagnosticAssessmentReason,
    DiagnosticExport, DiagnosticExportError, DiagnosticExportResult,
    DiagnosticForecastAvailability, DiagnosticIndicator, DiagnosticInitializationState,
    DiagnosticLogLevel, DiagnosticLogOptions, DiagnosticLoggingSettingsSnapshot,
    DiagnosticNotificationRuleSnapshot, DiagnosticNotificationSettingsSnapshot,
    DiagnosticNumericVersion, DiagnosticRuntimeSnapshot, DiagnosticSettingsSnapshot,
    DiagnosticSnapshot, DiagnosticSourceFreshness, DiagnosticSourceQuality,
    DiagnosticSummaryAvailability, DiagnosticSummarySnapshot, DiagnosticValue, FinalPaths,
    LocalDiagnosticLogger, NoFinalPathCheck, create_snapshot,
};
use bientot_domain::configuration::{NotificationRule, Settings};
use bientot_domain::time::Clock;
use flate2::Crc;
use flate2::read::DeflateDecoder;
use jiff::civil::Time;
use jiff::{SignedDuration, Timestamp};
use tempfile::TempDir;

const TICKS_PER_SECOND: i64 = 10_000_000;
const TICKS_PER_MINUTE: i64 = 60 * TICKS_PER_SECOND;
const TICKS_PER_HOUR: i64 = 60 * TICKS_PER_MINUTE;

fn now() -> Timestamp {
    "2026-09-10T12:34:56Z".parse().unwrap()
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        now()
    }
}

fn export(
    destination: &Path,
    log_directory: Option<&Path>,
    snapshot: &DiagnosticSnapshot,
) -> Result<DiagnosticExportResult, DiagnosticExportError> {
    DiagnosticExport::new(&FixedClock, &NoFinalPathCheck).export(
        destination,
        log_directory,
        snapshot,
    )
}

fn version(major: i32, minor: i32, build: i32, revision: i32) -> DiagnosticNumericVersion {
    DiagnosticNumericVersion {
        major,
        minor,
        build,
        revision,
    }
}

fn useful_snapshot() -> DiagnosticSnapshot {
    let rule = |enabled, lead_time_ticks| DiagnosticNotificationRuleSnapshot {
        enabled,
        lead_time_ticks,
    };
    DiagnosticSnapshot {
        initialization: DiagnosticInitializationState::Ready,
        settings: Some(DiagnosticSettingsSnapshot {
            nominal_daily_target_ticks: 7 * TICKS_PER_HOUR,
            minimum_effective_target_ticks: 6 * TICKS_PER_HOUR,
            lunch_start_minute_of_day: 750,
            lunch_end_minute_of_day: 840,
            preferred_latest_departure_minute_of_day: 1110,
            orange_warning_margin_ticks: 15 * TICKS_PER_MINUTE,
            long_lock_review_threshold_ticks: 90 * TICKS_PER_MINUTE,
            weekly_credit_enabled: true,
            start_with_windows: true,
            notifications: DiagnosticNotificationSettingsSnapshot {
                completion_approach: rule(true, 15 * TICKS_PER_MINUTE),
                latest_departure_approach: rule(true, 15 * TICKS_PER_MINUTE),
                day_complete: rule(false, 0),
            },
            activity_watch: DiagnosticActivityWatchTimingSnapshot {
                request_timeout_ticks: 5 * TICKS_PER_SECOND,
                afk_timeout_ticks: 180 * TICKS_PER_SECOND,
                poll_interval_ticks: 5 * TICKS_PER_SECOND,
                maximum_jitter_ticks: 5 * TICKS_PER_SECOND,
            },
            logging: DiagnosticLoggingSettingsSnapshot {
                maximum_file_count: 5,
                maximum_file_bytes: 1_048_576,
            },
            check_for_updates: true,
        }),
        runtime: DiagnosticRuntimeSnapshot {
            application_version: version(1, 2, 3, 4),
            operating_system_version: version(10, 0, 26200, 0),
            dot_net_version: version(10, 0, 11, 0),
            process_architecture: DiagnosticArchitecture::X64,
            operating_system_architecture: DiagnosticArchitecture::X64,
        },
        summary: Some(DiagnosticSummarySnapshot {
            availability: DiagnosticSummaryAvailability::Current,
            credited_work_ticks: 4 * TICKS_PER_HOUR,
            nominal_target_ticks: 7 * TICKS_PER_HOUR,
            effective_target_ticks: 6 * TICKS_PER_HOUR,
            remaining_work_ticks: 2 * TICKS_PER_HOUR,
            credited_overtime_ticks: 0,
            applied_carry_credit_ticks: TICKS_PER_HOUR,
            indicator: DiagnosticIndicator::Green,
            forecast: DiagnosticForecastAvailability::Estimated,
            assessment_reasons: vec![DiagnosticAssessmentReason::PendingSettlement],
            source_freshness: DiagnosticSourceFreshness::Fresh,
            source_quality: DiagnosticSourceQuality::Incomplete,
        }),
    }
}

/// A failed startup that kept a stale summary; .NET writes its out-of-range indicator as
/// `Unknown`.
fn failed_snapshot(reasons: Vec<DiagnosticAssessmentReason>) -> DiagnosticSnapshot {
    let useful = useful_snapshot();
    DiagnosticSnapshot {
        initialization: DiagnosticInitializationState::Failed,
        settings: None,
        summary: Some(DiagnosticSummarySnapshot {
            availability: DiagnosticSummaryAvailability::Stale,
            indicator: DiagnosticIndicator::Unknown,
            assessment_reasons: reasons,
            ..useful.summary.unwrap()
        }),
        ..useful
    }
}

/// A log line as the .NET logger serializes it; `properties` is JSON.
fn log_line(timestamp: Timestamp, level: &str, event: &str, properties: Option<&str>) -> String {
    format!(
        r#"{{"TimestampUtc":"{}","Level":"{level}","Event":"{event}","Properties":{}}}"#,
        timestamp.strftime("%Y-%m-%dT%H:%M:%S+00:00"),
        properties.unwrap_or("null")
    )
}

fn strict_log_path(directory: &Path, timestamp: Timestamp, suffix: u32) -> PathBuf {
    directory.join(format!(
        "worktime-{}-0000000-{suffix:032x}.jsonl",
        timestamp.strftime("%Y%m%d-%H%M%S")
    ))
}

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

fn logs_directory(root: &TempDir, name: &str) -> PathBuf {
    let directory = root.path().join(name);
    fs::create_dir_all(&directory).unwrap();
    directory
}

struct Archive {
    entries: Vec<(String, Vec<u8>)>,
}

impl Archive {
    /// Reads every entry through the central directory and checks it against its local header
    /// and its CRC.
    fn open(path: &Path) -> Self {
        let bytes = fs::read(path).unwrap();
        let u16_at = |at: usize| usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let end = bytes.len() - 22;
        assert_eq!(u32_at(end), 0x0605_4b50, "end of central directory");
        let mut at = u32_at(end + 16) as usize;
        let entries = (0..u16_at(end + 10))
            .map(|_| {
                assert_eq!(u32_at(at), 0x0201_4b50, "central directory header");
                let method = u16_at(at + 10);
                let crc = u32_at(at + 16);
                let compressed_size = u32_at(at + 20) as usize;
                let size = u32_at(at + 24) as usize;
                let name_length = u16_at(at + 28);
                let offset = u32_at(at + 42) as usize;
                let name =
                    String::from_utf8(bytes[at + 46..at + 46 + name_length].to_vec()).unwrap();
                at += 46 + name_length + u16_at(at + 30) + u16_at(at + 32);

                assert_eq!(u32_at(offset), 0x0403_4b50, "local header of {name}");
                assert_eq!(
                    &bytes[offset + 30..offset + 30 + name_length],
                    name.as_bytes()
                );
                assert_eq!(
                    u16_at(offset + 6),
                    2,
                    "flags of {name}, as .NET writes them"
                );
                assert_eq!(u16_at(offset + 8), method);
                assert_eq!(u32_at(offset + 14), crc);
                assert_eq!(u32_at(offset + 18) as usize, compressed_size);
                assert_eq!(u32_at(offset + 22) as usize, size);
                let start = offset + 30 + name_length + u16_at(offset + 28);
                let compressed = &bytes[start..start + compressed_size];
                let data = match method {
                    0 => compressed.to_vec(),
                    8 => {
                        let mut data = Vec::new();
                        DeflateDecoder::new(compressed)
                            .read_to_end(&mut data)
                            .unwrap();
                        data
                    }
                    other => panic!("{name} uses compression method {other}"),
                };
                assert_eq!(data.len(), size, "size of {name}");
                let mut check = Crc::new();
                check.update(&data);
                assert_eq!(check.sum(), crc, "CRC-32 of {name}");
                (name, data)
            })
            .collect();
        Self { entries }
    }

    fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|(name, _)| name.as_str()).collect()
    }

    fn bytes(&self, name: &str) -> &[u8] {
        &self
            .entries
            .iter()
            .find(|(entry, _)| entry == name)
            .unwrap_or_else(|| panic!("no entry {name}"))
            .1
    }

    fn text(&self, name: &str) -> String {
        String::from_utf8(self.bytes(name).to_vec()).unwrap()
    }

    fn log_lines(&self) -> Vec<String> {
        self.text("logs.jsonl").lines().map(str::to_owned).collect()
    }
}

/// The scalar value written for the first property named `name`.
fn value<'a>(json: &'a str, name: &str) -> &'a str {
    let key = format!("\"{name}\":");
    let start = json
        .find(&key)
        .unwrap_or_else(|| panic!("no {name} in {json}"))
        + key.len();
    let rest = &json[start..];
    &rest[..rest.find([',', '}']).unwrap()]
}

fn temporary_files(directory: &Path) -> Vec<String> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with(".worktime-diagnostics-") && name.ends_with(".tmp"))
        .collect()
}

#[test]
fn writes_useful_fixed_artifacts_without_copying_hostile_values() {
    let root = TempDir::new().unwrap();
    let logs = logs_directory(&root, "logs");
    let secret = "SENTINEL-api-key-9ad";
    let bucket = "SENTINEL-bucket-machine";
    let host = "SENTINEL-host.example";
    let path = r"C:\Users\SENTINEL-user\private.txt";
    let app = "SENTINEL-activity-app";
    let title = "SENTINEL-window-title";
    let escaped_path = path.replace('\\', r"\\");
    let lines = [
        log_line(
            now() - minutes(1),
            "Error",
            "periodic_refresh.failed",
            Some(&format!(
                r#"{{"exceptionType":"System.IO.IOException","message":"{escaped_path} {secret} {bucket} {host} {app} {title}"}}"#
            )),
        ),
        log_line(
            now() - minutes(2),
            "Warning",
            "power_history.recovery_unavailable",
            Some(&format!(r#"{{"status":"Truncated","reason":"{title}"}}"#)),
        ),
        log_line(
            now() - minutes(3),
            "Error",
            "notification.failed",
            Some(&format!(
                r#"{{"exceptionType":"Harmless.CustomException","message":"{secret}","details":"{bucket}","window":"{title}","process":"{app}","endpoint":"{host}"}}"#
            )),
        ),
        log_line(
            now() - minutes(4),
            "Information",
            "untrusted.event",
            Some(&format!(r#"{{"message":"{secret}"}}"#)),
        ),
        format!("{{ malformed {secret}"),
    ];
    fs::write(
        strict_log_path(&logs, now() - minutes(5), 1),
        lines.join("\n") + "\n",
    )
    .unwrap();

    let mut raw_settings = Settings {
        lunch_start: Time::constant(12, 30, 0, 0),
        ..Settings::default()
    };
    raw_settings.activity_watch.base_url = format!("https://{host}");
    raw_settings.activity_watch.selected_bucket_id = Some(bucket.to_owned());
    raw_settings.activity_watch.api_key_reference = Some(secret.to_owned());
    let baseline = useful_snapshot();
    let projected = DiagnosticSnapshot {
        summary: baseline.summary,
        ..create_snapshot(
            DiagnosticInitializationState::Ready,
            Some(&raw_settings),
            None,
            None,
            now(),
            baseline.runtime,
        )
    };
    let destination = root.path().join("diagnostics.zip");
    let result = export(&destination, Some(&logs), &projected).unwrap();

    let archive = Archive::open(&destination);
    assert_eq!(
        archive.names(),
        ["manifest.json", "snapshot.json", "logs.jsonl"]
    );
    let all_text: String = archive
        .entries
        .iter()
        .map(|(name, data)| format!("{name}{}\n", String::from_utf8_lossy(data)))
        .collect();
    for sentinel in [secret, bucket, host, path, app, title, "SENTINEL"] {
        assert!(!all_text.contains(sentinel), "{sentinel} was exported");
    }

    let snapshot = archive.text("snapshot.json");
    assert_eq!(value(&snapshot, "nominalDailyTargetTicks"), "252000000000");
    assert_eq!(value(&snapshot, "lunchStartMinuteOfDay"), "750");
    assert_eq!(value(&snapshot, "creditedWorkTicks"), "144000000000");
    let summary = &snapshot[snapshot.find("\"summary\":").unwrap()..];
    assert_eq!(value(summary, "availability"), "\"Current\"");

    let projected_lines = archive.log_lines();
    assert_eq!(projected_lines.len(), 3);
    assert_eq!(
        projected_lines[0],
        r#"{"timestampUtc":"2026-09-10T12:33:00+00:00","level":"Error","event":"periodic_refresh.failed","failureCategory":"Io"}"#
    );
    assert!(
        projected_lines
            .iter()
            .any(|line| line.contains(r#""powerHistoryStatus":"Truncated""#))
    );
    assert!(
        projected_lines
            .iter()
            .any(|line| line.contains(r#""failureCategory":"Other""#))
    );

    let manifest = archive.text("manifest.json");
    assert_eq!(value(&manifest, "logs"), "\"Partial\"");
    assert_eq!(value(&manifest, "unknownEvents"), "1");
    assert_eq!(value(&manifest, "malformedRecords"), "1");
    assert_eq!(result.log_record_count, 3);
    assert!((1..=DiagnosticExport::MAXIMUM_ARCHIVE_BYTES).contains(&result.archive_bytes));
    assert_eq!(
        result.archive_bytes,
        fs::metadata(&destination).unwrap().len()
    );
}

#[test]
fn keeps_update_check_and_startup_events_with_the_store_hresult() {
    let root = TempDir::new().unwrap();
    let logs = logs_directory(&root, "logs");
    let secret = "SENTINEL-store-message";
    let lines = [
        log_line(
            now() - minutes(1),
            "Warning",
            "update_check.failed",
            Some(&format!(
                r#"{{"exceptionType":"System.Runtime.InteropServices.COMException","message":"{secret}","hresult":-2147012867}}"#
            )),
        ),
        log_line(
            now() - minutes(2),
            "Warning",
            "update_check.unavailable",
            Some(&format!(
                r#"{{"exceptionType":"System.InvalidOperationException","message":"{secret}","hresult":5}}"#
            )),
        ),
        log_line(
            now() - minutes(3),
            "Information",
            "startup.tray_ready",
            None,
        ),
        log_line(
            now() - minutes(4),
            "Error",
            "periodic_refresh.failed",
            Some(r#"{"exceptionType":"System.IO.IOException","hresult":7}"#),
        ),
    ];
    fs::write(
        strict_log_path(&logs, now() - minutes(5), 3),
        lines.join("\n") + "\n",
    )
    .unwrap();

    let destination = root.path().join("diagnostics.zip");
    let result = export(&destination, Some(&logs), &useful_snapshot()).unwrap();

    let archive = Archive::open(&destination);
    assert_eq!(
        archive.log_lines(),
        [
            r#"{"timestampUtc":"2026-09-10T12:33:00+00:00","level":"Warning","event":"update_check.failed","failureCategory":"Other","hresult":-2147012867}"#,
            r#"{"timestampUtc":"2026-09-10T12:32:00+00:00","level":"Warning","event":"update_check.unavailable","failureCategory":"InvalidOperation"}"#,
            r#"{"timestampUtc":"2026-09-10T12:31:00+00:00","level":"Information","event":"startup.tray_ready"}"#,
            r#"{"timestampUtc":"2026-09-10T12:30:00+00:00","level":"Error","event":"periodic_refresh.failed","failureCategory":"Io"}"#,
        ]
    );
    assert_eq!(value(&archive.text("manifest.json"), "unknownEvents"), "0");
    assert_eq!(result.log_record_count, 4);
}

#[test]
fn keeps_toast_clicks_without_their_properties() {
    let root = TempDir::new().unwrap();
    let logs = logs_directory(&root, "logs");
    fs::write(
        strict_log_path(&logs, now() - minutes(2), 3),
        log_line(
            now() - minutes(1),
            "Information",
            "notification.activated",
            Some(r#"{"origin":"cold"}"#),
        ) + "\n",
    )
    .unwrap();

    let destination = root.path().join("diagnostics.zip");
    export(&destination, Some(&logs), &useful_snapshot()).unwrap();

    let archive = Archive::open(&destination);
    assert_eq!(
        archive.log_lines(),
        [
            r#"{"timestampUtc":"2026-09-10T12:33:00+00:00","level":"Information","event":"notification.activated"}"#
        ]
    );
    assert_eq!(value(&archive.text("manifest.json"), "unknownEvents"), "0");
}

#[test]
fn reads_what_the_logger_writes() {
    struct StepClock(Cell<Timestamp>);
    impl Clock for StepClock {
        fn now(&self) -> Timestamp {
            self.0.get()
        }
    }
    let root = TempDir::new().unwrap();
    let logs = root.path().join("logs");
    let secret = "SENTINEL-store-message";
    let clock = StepClock(Cell::new(now() - minutes(4)));
    let mut logger = LocalDiagnosticLogger::new(&logs, DiagnosticLogOptions::default(), &clock);
    let mut write = |level, event, properties: &[(&str, DiagnosticValue)]| {
        logger.write(level, event, properties).unwrap();
        clock.0.set(clock.0.get() + minutes(1));
    };
    write(
        DiagnosticLogLevel::Error,
        "periodic_refresh.failed",
        &[
            ("exceptionType", "System.IO.IOException".into()),
            ("message", secret.into()),
        ],
    );
    write(DiagnosticLogLevel::Information, "startup.tray_ready", &[]);
    write(
        DiagnosticLogLevel::Warning,
        "power_history.recovery_unavailable",
        &[("status", "Truncated".into()), ("reason", secret.into())],
    );
    write(
        DiagnosticLogLevel::Warning,
        "update_check.failed",
        &[
            (
                "exceptionType",
                "System.Runtime.InteropServices.COMException".into(),
            ),
            ("message", secret.into()),
            ("hresult", (-2_147_012_867).into()),
        ],
    );
    write(DiagnosticLogLevel::Debug, "unlisted.event", &[]);

    let destination = root.path().join("diagnostics.zip");
    let result = export(&destination, Some(&logs), &useful_snapshot()).unwrap();

    let archive = Archive::open(&destination);
    assert_eq!(
        archive.log_lines(),
        [
            r#"{"timestampUtc":"2026-09-10T12:30:00+00:00","level":"Error","event":"periodic_refresh.failed","failureCategory":"Io"}"#,
            r#"{"timestampUtc":"2026-09-10T12:31:00+00:00","level":"Information","event":"startup.tray_ready"}"#,
            r#"{"timestampUtc":"2026-09-10T12:32:00+00:00","level":"Warning","event":"power_history.recovery_unavailable","powerHistoryStatus":"Truncated"}"#,
            r#"{"timestampUtc":"2026-09-10T12:33:00+00:00","level":"Warning","event":"update_check.failed","failureCategory":"Other","hresult":-2147012867}"#,
        ]
    );
    let manifest = archive.text("manifest.json");
    assert_eq!(value(&manifest, "unknownEvents"), "1");
    assert_eq!(value(&manifest, "malformedRecords"), "0");
    assert_eq!(result.log_record_count, 4);
}

#[test]
fn reports_malformed_expired_future_oversize_and_partial_input() {
    let root = TempDir::new().unwrap();
    let logs = logs_directory(&root, "logs");
    let deep = format!(
        r#"{{"TimestampUtc":"2026-09-10T12:30:00.0000000+00:00","Level":"Error","Event":"notification.failed","Properties":{}0{}}}"#,
        r#"{"x":"#.repeat(10),
        "}".repeat(10)
    );
    let lines = [
        log_line(
            now() - SignedDuration::from_hours(15 * 24),
            "Error",
            "notification.failed",
            None,
        ),
        log_line(now() + minutes(1), "Error", "notification.failed", None),
        log_line(now() - minutes(1), "Verbose", "notification.failed", None),
        log_line(now() - minutes(1), "Warning", "unknown.event", None),
        "not-json".to_owned(),
        deep,
        "x".repeat(DiagnosticExport::MAXIMUM_LINE_BYTES + 1)
            + &log_line(now() - minutes(1), "Error", "notification.failed", None),
    ];
    fs::write(
        strict_log_path(&logs, now() - minutes(2), 2),
        lines.join("\n")
            + "\n"
            + &log_line(
                now() - minutes(1),
                "Warning",
                "lifecycle.current_probe_unavailable",
                None,
            ),
    )
    .unwrap();

    let destination = root.path().join("diagnostics.zip");
    export(&destination, Some(&logs), &useful_snapshot()).unwrap();

    let archive = Archive::open(&destination);
    assert!(archive.bytes("logs.jsonl").is_empty());
    let manifest = archive.text("manifest.json");
    assert_eq!(value(&manifest, "expiredRecords"), "1");
    assert_eq!(value(&manifest, "futureRecords"), "1");
    assert_eq!(value(&manifest, "unknownLevels"), "1");
    assert_eq!(value(&manifest, "unknownEvents"), "1");
    assert_eq!(value(&manifest, "malformedRecords"), "2");
    assert_eq!(value(&manifest, "oversizeLines"), "1");
    assert_eq!(value(&manifest, "partialLines"), "1");
}

#[test]
fn selects_five_newest_strict_files_by_timestamp_in_name() {
    let root = TempDir::new().unwrap();
    let logs = logs_directory(&root, "logs");
    for index in 0..6 {
        let timestamp = now() - minutes(index);
        fs::write(
            strict_log_path(&logs, timestamp, u32::try_from(index).unwrap() + 10),
            log_line(
                timestamp,
                "Information",
                "notification.smoke_accepted",
                None,
            ) + "\n",
        )
        .unwrap();
    }

    let destination = root.path().join("diagnostics.zip");
    export(&destination, Some(&logs), &useful_snapshot()).unwrap();

    let archive = Archive::open(&destination);
    let records = archive.log_lines();
    assert_eq!(records.len(), 5);
    let timestamps: Vec<_> = records
        .iter()
        .map(|record| value(record, "timestampUtc"))
        .collect();
    assert!(timestamps.contains(&"\"2026-09-10T12:34:00+00:00\""));
    assert!(!timestamps.contains(&"\"2026-09-10T12:29:00+00:00\""));
    let manifest = archive.text("manifest.json");
    assert_eq!(value(&manifest, "eligibleFiles"), "6");
    assert_eq!(value(&manifest, "selectedFiles"), "5");
    assert_eq!(value(&manifest, "filesOmittedByCount"), "1");
}

#[test]
fn caps_emitted_records() {
    let root = TempDir::new().unwrap();
    let logs = logs_directory(&root, "logs");
    let line = log_line(
        now() - minutes(1),
        "Information",
        "notification.smoke_registered",
        None,
    );
    fs::write(
        strict_log_path(&logs, now(), 99),
        vec![line; DiagnosticExport::MAXIMUM_LOG_RECORDS + 1].join("\n") + "\n",
    )
    .unwrap();

    let destination = root.path().join("record-limit.zip");
    export(&destination, Some(&logs), &useful_snapshot()).unwrap();

    let archive = Archive::open(&destination);
    assert_eq!(
        archive.log_lines().len(),
        DiagnosticExport::MAXIMUM_LOG_RECORDS
    );
    assert!(
        (1..=DiagnosticExport::MAXIMUM_LOGS_BYTES).contains(&archive.bytes("logs.jsonl").len())
    );
    assert_eq!(
        value(&archive.text("manifest.json"), "recordsOmittedByCount"),
        "1"
    );
}

#[test]
fn enforces_directory_and_input_byte_limits() {
    let root = TempDir::new().unwrap();
    let entry_directory = logs_directory(&root, "entry-limit");
    for index in 0..DiagnosticExport::MAXIMUM_DIRECTORY_ENTRIES + 10 {
        fs::write(
            entry_directory.join(format!("untrusted-{index:03}.txt")),
            "ignored",
        )
        .unwrap();
    }

    let entry_destination = root.path().join("entry-limit.zip");
    export(
        &entry_destination,
        Some(&entry_directory),
        &useful_snapshot(),
    )
    .unwrap();
    let manifest = Archive::open(&entry_destination).text("manifest.json");
    assert_eq!(value(&manifest, "examinedDirectoryEntries"), "256");
    assert_eq!(value(&manifest, "directoryEntryLimitReached"), "true");

    let byte_directory = logs_directory(&root, "byte-limit");
    let input_bytes = usize::try_from(DiagnosticExport::MAXIMUM_INPUT_BYTES).unwrap();
    fs::write(
        strict_log_path(&byte_directory, now(), 200),
        "x".repeat(input_bytes + 31) + "\n",
    )
    .unwrap();
    let byte_destination = root.path().join("byte-limit.zip");
    export(&byte_destination, Some(&byte_directory), &useful_snapshot()).unwrap();
    let manifest = Archive::open(&byte_destination).text("manifest.json");
    assert_eq!(value(&manifest, "inputLimitReached"), "true");
    assert_eq!(value(&manifest, "inputBytesOmitted"), "32");
    assert_eq!(value(&manifest, "partialLines"), "1");
}

#[test]
fn reports_failure_before_initialization_without_inventing_unavailable_state() {
    let root = TempDir::new().unwrap();
    let failed = failed_snapshot(vec![DiagnosticAssessmentReason::PendingSettlement]);
    let destination = root.path().join("failed-startup.zip");

    export(&destination, None, &failed).unwrap();

    let archive = Archive::open(&destination);
    assert!(archive.bytes("logs.jsonl").is_empty());
    let manifest = archive.text("manifest.json");
    assert_eq!(value(&manifest, "settings"), "\"Unavailable\"");
    assert_eq!(value(&manifest, "summary"), "\"Stale\"");
    assert_eq!(value(&manifest, "logs"), "\"Unavailable\"");
    let snapshot = archive.text("snapshot.json");
    assert_eq!(value(&snapshot, "initialization"), "\"Failed\"");
    assert!(!snapshot.contains("\"settings\""));
    assert_eq!(value(&snapshot, "availability"), "\"Stale\"");
    assert_eq!(value(&snapshot, "indicator"), "\"Unknown\"");
    assert_eq!(value(&snapshot, "creditedWorkTicks"), "144000000000");
}

#[test]
fn preserves_existing_destination_and_cleans_owned_temporary_files() {
    let root = TempDir::new().unwrap();
    let destination = root.path().join("existing.zip");
    let original = b"existing-diagnostic";
    fs::write(&destination, original).unwrap();

    let oversized = failed_snapshot(vec![DiagnosticAssessmentReason::PendingSettlement; 10_000]);
    let error = export(&destination, None, &oversized).unwrap_err();
    assert!(
        matches!(error, DiagnosticExportError::TooManyAssessmentReasons),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        "The diagnostic assessment exceeds its finite reason set."
    );
    assert_eq!(fs::read(&destination).unwrap(), original);
    assert_eq!(temporary_files(root.path()), Vec::<String>::new());

    let blocked = root.path().join("destination-is-a-directory.zip");
    fs::create_dir(&blocked).unwrap();
    let error = export(&blocked, None, &useful_snapshot()).unwrap_err();
    assert!(matches!(error, DiagnosticExportError::Io(_)), "{error:?}");
    assert!(blocked.is_dir());
    assert_eq!(temporary_files(root.path()), Vec::<String>::new());

    export(&destination, None, &useful_snapshot()).unwrap();
    assert_ne!(fs::read(&destination).unwrap(), original);
    assert_eq!(temporary_files(root.path()), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn rejects_reparse_root() {
    let root = TempDir::new().unwrap();
    let target = logs_directory(&root, "target");
    fs::write(
        strict_log_path(&target, now(), 300),
        log_line(
            now(),
            "Error",
            "notification.failed",
            Some(r#"{"message":"outside"}"#),
        ) + "\n",
    )
    .unwrap();
    let link = root.path().join("linked-logs");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let destination = root.path().join("linked.zip");
    export(&destination, Some(&link), &useful_snapshot()).unwrap();

    let archive = Archive::open(&destination);
    assert!(archive.bytes("logs.jsonl").is_empty());
    let manifest = archive.text("manifest.json");
    assert_eq!(value(&manifest, "logs"), "\"Unavailable\"");
    assert_eq!(value(&manifest, "rejectedReparsePoints"), "1");
}

/// Not in the .NET suite, whose final path checks only run on Windows: a file whose final path
/// leaves the log directory is not read, and a directory that is not its own final path is not
/// listed.
#[test]
fn skips_what_the_final_path_check_rejects() {
    struct Rejecting {
        directory: bool,
    }

    impl FinalPaths for Rejecting {
        fn directory_is_final(&self, _directory: &Path) -> bool {
            !self.directory
        }

        fn file_is_in(&self, _file: &File, _directory: &Path) -> bool {
            self.directory
        }
    }

    let root = TempDir::new().unwrap();
    let logs = logs_directory(&root, "logs");
    fs::write(
        strict_log_path(&logs, now(), 1),
        log_line(now(), "Error", "notification.failed", None) + "\n",
    )
    .unwrap();
    let destination = root.path().join("diagnostics.zip");
    for (directory, availability, counter) in [
        (false, "\"Partial\"", "rejectedOutsideRoot"),
        (true, "\"Unavailable\"", "rejectedReparsePoints"),
    ] {
        DiagnosticExport::new(&FixedClock, &Rejecting { directory })
            .export(&destination, Some(&logs), &useful_snapshot())
            .unwrap();

        let archive = Archive::open(&destination);
        assert!(archive.bytes("logs.jsonl").is_empty());
        let manifest = archive.text("manifest.json");
        assert_eq!(value(&manifest, "logs"), availability);
        assert_eq!(value(&manifest, counter), "1");
    }
}

/// Not in the .NET suite: only regular files with the logger's exact name are read, so neither a
/// directory nor a link named like a log file, nor a near miss of the name, is.
#[cfg(unix)]
#[test]
fn reads_only_regular_files_with_the_strict_name() {
    let root = TempDir::new().unwrap();
    let logs = logs_directory(&root, "logs");
    let outside = root.path().join("outside.jsonl");
    fs::write(
        &outside,
        log_line(
            now(),
            "Error",
            "notification.failed",
            Some(r#"{"message":"SENTINEL-outside"}"#),
        ) + "\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside, strict_log_path(&logs, now(), 1)).unwrap();
    fs::create_dir(strict_log_path(&logs, now(), 2)).unwrap();
    let near_misses = [
        "worktime-20260910-123456-0000000-0000000000000000000000000000000A.jsonl",
        "worktime-20260910-123456-0000000-000000000000000000000000000000a.jsonl",
        "worktime-20260931-123456-0000000-0000000000000000000000000000000a.jsonl",
        "worktime-20260910-123456-0000000-0000000000000000000000000000000a.JSONL",
        "Worktime-20260910-123456-0000000-0000000000000000000000000000000a.jsonl",
    ];
    for name in near_misses {
        fs::copy(&outside, logs.join(name)).unwrap();
    }
    let kept = strict_log_path(&logs, now() - minutes(1), 3);
    fs::write(
        &kept,
        log_line(
            now() - minutes(1),
            "Information",
            "startup.tray_ready",
            None,
        ) + "\n",
    )
    .unwrap();

    let destination = root.path().join("diagnostics.zip");
    export(&destination, Some(&logs), &useful_snapshot()).unwrap();

    let archive = Archive::open(&destination);
    assert_eq!(
        archive.log_lines(),
        [
            r#"{"timestampUtc":"2026-09-10T12:33:00+00:00","level":"Information","event":"startup.tray_ready"}"#
        ]
    );
    let manifest = archive.text("manifest.json");
    assert_eq!(value(&manifest, "examinedDirectoryEntries"), "8");
    assert_eq!(value(&manifest, "rejectedNames"), "6");
    assert_eq!(value(&manifest, "rejectedReparsePoints"), "1");
    assert_eq!(value(&manifest, "eligibleFiles"), "1");
    assert_eq!(value(&manifest, "logs"), "\"Partial\"");
}

/// The settings of the .NET fixture's "factory" snapshot, full of sentinels the snapshot drops.
fn factory_snapshot() -> DiagnosticSnapshot {
    let mut settings = Settings {
        nominal_daily_target: SignedDuration::from_mins(450),
        minimum_effective_target: SignedDuration::from_mins(390),
        lunch_start: Time::constant(12, 15, 0, 0),
        lunch_end: Time::constant(13, 45, 0, 0),
        preferred_latest_departure: Time::constant(18, 20, 0, 0),
        orange_warning_margin: minutes(20),
        long_lock_review_threshold: minutes(75),
        weekly_credit_enabled: false,
        start_with_windows: false,
        language_override: Some("SENTINEL-language".to_owned()),
        ..Settings::default()
    };
    settings.notifications.completion_approach = NotificationRule::new(false, minutes(10));
    settings.notifications.latest_departure_approach = NotificationRule::new(true, minutes(25));
    settings.notifications.day_complete = NotificationRule::new(true, SignedDuration::ZERO);
    settings.activity_watch.base_url = "https://SENTINEL-host.example".to_owned();
    settings.activity_watch.selected_bucket_id = Some("SENTINEL-bucket".to_owned());
    settings.activity_watch.api_key_reference = Some("SENTINEL-key-reference".to_owned());
    settings.logging.maximum_file_count = 3;
    settings.logging.maximum_file_bytes = 65_536;
    settings.updates.check_for_updates = false;
    settings.updates.last_checked_at = Some("2026-09-09T01:02:03Z".parse().unwrap());
    create_snapshot(
        DiagnosticInitializationState::Ready,
        Some(&settings),
        None,
        None,
        now(),
        DiagnosticRuntimeSnapshot {
            application_version: version(0, 1, 0, 0),
            operating_system_version: version(10, 0, 22631, 0),
            dot_net_version: version(10, 0, 0, 0),
            process_architecture: DiagnosticArchitecture::Arm64,
            operating_system_architecture: DiagnosticArchitecture::S390x,
        },
    )
}

#[test]
fn archives_match_dotnet() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diagnostics");
    let cases = [
        ("hostile", factory_snapshot()),
        ("update", useful_snapshot()),
        ("omissions", useful_snapshot()),
        ("edge", useful_snapshot()),
        (
            "failed",
            failed_snapshot(vec![
                DiagnosticAssessmentReason::SourceStale,
                DiagnosticAssessmentReason::PendingSettlement,
                DiagnosticAssessmentReason::ProvisionalLockReview,
            ]),
        ),
    ];
    for (name, snapshot) in cases {
        let fixture = fixtures.join(name);
        let root = TempDir::new().unwrap();
        let input = fixture.join("input.jsonl");
        let logs = input.exists().then(|| {
            let logs = logs_directory(&root, "logs");
            fs::copy(
                &input,
                logs.join(
                    "worktime-20260910-123000-0000000-0000000000000000000000000000000a.jsonl",
                ),
            )
            .unwrap();
            logs
        });
        let destination = root.path().join("diagnostics.zip");
        export(&destination, logs.as_deref(), &snapshot).unwrap();

        let archive = Archive::open(&destination);
        assert_eq!(
            archive.names(),
            ["manifest.json", "snapshot.json", "logs.jsonl"],
            "{name}"
        );
        for entry in archive.names() {
            assert_eq!(
                String::from_utf8_lossy(archive.bytes(entry)),
                String::from_utf8_lossy(&fs::read(fixture.join(entry)).unwrap()),
                "{name}/{entry}"
            );
        }
    }
}
