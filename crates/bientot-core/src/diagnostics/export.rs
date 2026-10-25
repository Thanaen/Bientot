//! The diagnostic ZIP, ported from `Diagnostics/DiagnosticExportService.cs`: `manifest.json`,
//! `snapshot.json` and `logs.jsonl`, each under a fixed size limit.
//!
//! Only the newest strictly named log files of one directory are read, never through a link,
//! and only within fixed limits. Each record is projected onto its minute, its level and an
//! event from a fixed list, with a failure category, a power history status or a Store HRESULT
//! where that event has one; any other property is dropped. The manifest counts what was left
//! out. The archive is written beside the destination and moved over it only once complete.

use std::fs::{self, DirEntry, File, Metadata, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use bientot_domain::time::Clock;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use super::json::{JsonObject, JsonValue};
use super::{
    DiagnosticAssessmentReason, DiagnosticSnapshot, DiagnosticSummaryAvailability, named_enum, zip,
};

/// Records older than this are left out; the logger keeps its files as long.
const RETENTION_DAYS: i32 = 14;
pub(super) const RETENTION: SignedDuration = SignedDuration::from_hours(RETENTION_DAYS as i64 * 24);
/// The deepest nesting a record may have, as `JsonDocumentOptions.MaxDepth` counts it.
const MAXIMUM_RECORD_DEPTH: usize = 8;

const LEVELS: [&str; 4] = ["Debug", "Information", "Warning", "Error"];
const EVENTS: [&str; 15] = [
    "notification.activated",
    "notification.smoke_registered",
    "notification.smoke_accepted",
    "notification.failed",
    "notification.smoke_failed",
    "notification.registration_failed",
    "tracking_refresh.failed",
    "tray_command.failed",
    "window_command.failed",
    "periodic_refresh.failed",
    "lifecycle.current_probe_unavailable",
    "power_history.recovery_unavailable",
    "update_check.failed",
    "update_check.unavailable",
    "startup.tray_ready",
];
/// The events whose `exceptionType` property is kept as a failure category.
const FAILURE_EVENTS: [&str; 9] = [
    "notification.failed",
    "notification.smoke_failed",
    "notification.registration_failed",
    "tracking_refresh.failed",
    "tray_command.failed",
    "window_command.failed",
    "periodic_refresh.failed",
    "update_check.failed",
    "update_check.unavailable",
];
const POWER_HISTORY_EVENT: &str = "power_history.recovery_unavailable";
/// The Store's HRESULT is the only detail that tells update failures apart.
const HRESULT_EVENT: &str = "update_check.failed";

/// Where a path really leads once links and mount points are followed. .NET asks Windows for
/// the final path of the log directory and of each opened file; elsewhere it checks nothing.
pub trait FinalPaths {
    /// Whether `directory` is its own final path.
    fn directory_is_final(&self, directory: &Path) -> bool;

    /// Whether the final path of `file` is directly inside `directory`.
    fn file_is_in(&self, file: &File, directory: &Path) -> bool;
}

/// The checks .NET runs off Windows: none. The link checks on the path itself still run.
pub struct NoFinalPathCheck;

impl FinalPaths for NoFinalPathCheck {
    fn directory_is_final(&self, _directory: &Path) -> bool {
        true
    }

    fn file_is_in(&self, _file: &File, _directory: &Path) -> bool {
        true
    }
}

#[derive(Debug)]
pub enum DiagnosticExportError {
    TooManyAssessmentReasons,
    NoParentDirectory,
    SnapshotTooLarge,
    ManifestTooLarge,
    ContentsTooLarge,
    ArchiveTooLarge,
    /// Writing or replacing the archive failed; the destination is left as it was.
    Io(io::Error),
}

impl std::fmt::Display for DiagnosticExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyAssessmentReasons => {
                f.write_str("The diagnostic assessment exceeds its finite reason set.")
            }
            Self::NoParentDirectory => f.write_str("The destination must have a parent directory."),
            Self::SnapshotTooLarge => {
                f.write_str("The diagnostic snapshot exceeds its fixed size limit.")
            }
            Self::ManifestTooLarge => {
                f.write_str("The diagnostic manifest exceeds its fixed size limit.")
            }
            Self::ContentsTooLarge => {
                f.write_str("The diagnostic contents exceed their fixed size limit.")
            }
            Self::ArchiveTooLarge => {
                f.write_str("The diagnostic archive exceeds its fixed size limit.")
            }
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for DiagnosticExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for DiagnosticExportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticExportResult {
    pub log_record_count: usize,
    pub uncompressed_bytes: usize,
    pub archive_bytes: u64,
}

/// Writes diagnostic archives, reading the clock for record retention and entry times.
pub struct DiagnosticExport<'a> {
    clock: &'a dyn Clock,
    final_paths: &'a dyn FinalPaths,
}

impl<'a> DiagnosticExport<'a> {
    pub const MAXIMUM_DIRECTORY_ENTRIES: usize = 256;
    pub const MAXIMUM_LOG_FILES: usize = 5;
    pub const MAXIMUM_INPUT_BYTES: u64 = 5 * 1024 * 1024;
    pub const MAXIMUM_LINE_BYTES: usize = 16 * 1024;
    pub const MAXIMUM_LOG_RECORDS: usize = 10_000;
    pub const MAXIMUM_LOGS_BYTES: usize = 2 * 1024 * 1024;
    pub const MAXIMUM_SNAPSHOT_BYTES: usize = 32 * 1024;
    pub const MAXIMUM_MANIFEST_BYTES: usize = 16 * 1024;
    pub const MAXIMUM_UNCOMPRESSED_BYTES: usize =
        Self::MAXIMUM_LOGS_BYTES + Self::MAXIMUM_SNAPSHOT_BYTES + Self::MAXIMUM_MANIFEST_BYTES;
    pub const MAXIMUM_ARCHIVE_BYTES: u64 = 3 * 1024 * 1024;

    pub fn new(clock: &'a dyn Clock, final_paths: &'a dyn FinalPaths) -> Self {
        Self { clock, final_paths }
    }

    /// Writes the archive of `snapshot` and of the logs in `log_directory` to `destination`,
    /// replacing it. On failure `destination` is unchanged and no temporary file is left.
    pub fn export(
        &self,
        destination: &Path,
        log_directory: Option<&Path>,
        snapshot: &DiagnosticSnapshot,
    ) -> Result<DiagnosticExportResult, DiagnosticExportError> {
        if snapshot.summary.as_ref().is_some_and(|summary| {
            summary.assessment_reasons.len() > DiagnosticAssessmentReason::COUNT
        }) {
            return Err(DiagnosticExportError::TooManyAssessmentReasons);
        }
        let destination = std::path::absolute(destination)?;
        let directory = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or(DiagnosticExportError::NoParentDirectory)?;
        let snapshot_json = snapshot.to_json();
        if snapshot_json.len() > Self::MAXIMUM_SNAPSHOT_BYTES {
            return Err(DiagnosticExportError::SnapshotTooLarge);
        }

        let mut counters = Omissions::default();
        let logs = self.read_logs(log_directory, &mut counters);
        let manifest_json = manifest(snapshot, logs.availability, &counters);
        if manifest_json.len() > Self::MAXIMUM_MANIFEST_BYTES {
            return Err(DiagnosticExportError::ManifestTooLarge);
        }
        let uncompressed_bytes = manifest_json.len() + snapshot_json.len() + logs.bytes.len();
        if uncompressed_bytes > Self::MAXIMUM_UNCOMPRESSED_BYTES {
            return Err(DiagnosticExportError::ContentsTooLarge);
        }

        fs::create_dir_all(directory)?;
        let temporary = directory.join(format!(".worktime-diagnostics-{}.tmp", unique_hex()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let modified = self.clock.now().to_zoned(TimeZone::system()).datetime();
        let entries: [(&str, &[u8]); 3] = [
            ("manifest.json", manifest_json.as_bytes()),
            ("snapshot.json", snapshot_json.as_bytes()),
            ("logs.jsonl", &logs.bytes),
        ];
        let written = write_archive(&mut file, &entries, modified);
        drop(file);
        let published = written.and_then(|archive_bytes| {
            fs::rename(&temporary, &destination)?;
            Ok(archive_bytes)
        });
        if published.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        Ok(DiagnosticExportResult {
            log_record_count: logs.record_count,
            uncompressed_bytes,
            archive_bytes: published?,
        })
    }

    fn read_logs(&self, log_directory: Option<&Path>, counters: &mut Omissions) -> LogProjection {
        let Some(log_directory) = log_directory.filter(|path| !is_blank(path)) else {
            return LogProjection::UNAVAILABLE;
        };
        let Ok(root) = std::path::absolute(log_directory) else {
            counters.unreadable_files += 1;
            return LogProjection::UNAVAILABLE;
        };
        if !root.is_dir() {
            return LogProjection::UNAVAILABLE;
        }
        match has_reparse_point_in_chain(&root) {
            Err(_) => {
                counters.unreadable_files += 1;
                return LogProjection::UNAVAILABLE;
            }
            Ok(true) => {
                counters.rejected_reparse_points += 1;
                return LogProjection::UNAVAILABLE;
            }
            Ok(false) if !self.final_paths.directory_is_final(&root) => {
                counters.rejected_reparse_points += 1;
                return LogProjection::UNAVAILABLE;
            }
            Ok(false) => {}
        }

        let mut candidates = enumerate_candidates(&root, counters);
        if counters.directory_enumeration_failed && candidates.is_empty() {
            return LogProjection::UNAVAILABLE;
        }
        candidates.sort_by(|left, right| {
            right
                .timestamp
                .cmp(&left.timestamp)
                .then_with(|| right.name.cmp(&left.name))
        });
        counters.files_omitted_by_count = candidates.len().saturating_sub(Self::MAXIMUM_LOG_FILES);
        candidates.truncate(Self::MAXIMUM_LOG_FILES);
        counters.selected_files = candidates.len();

        let mut output = Vec::new();
        let mut remaining_input_bytes = Self::MAXIMUM_INPUT_BYTES;
        let mut record_count = 0;
        for candidate in &candidates {
            if remaining_input_bytes == 0 {
                counters.input_limit_reached = true;
                counters.files_omitted_by_input_limit += 1;
                continue;
            }
            let read = File::open(&candidate.path).and_then(|file| {
                if !self.final_paths.file_is_in(&file, &root) {
                    return Ok(None);
                }
                let initial_length = file.metadata()?.len();
                let bytes_to_read = initial_length.min(remaining_input_bytes);
                if bytes_to_read < initial_length {
                    counters.input_limit_reached = true;
                    counters.input_bytes_omitted += initial_length - bytes_to_read;
                }
                // A failed read keeps the records already projected but not their count, as
                // .NET does.
                let (consumed, count) =
                    self.read_file(file, bytes_to_read, &mut output, counters, record_count)?;
                Ok(Some((bytes_to_read, consumed, count)))
            });
            match read {
                Ok(None) => counters.rejected_outside_root += 1,
                Ok(Some((bytes_to_read, consumed, count))) => {
                    remaining_input_bytes -= consumed;
                    record_count = count;
                    if consumed < bytes_to_read {
                        counters.files_disappeared += 1;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    counters.files_disappeared += 1;
                }
                Err(_) => counters.unreadable_files += 1,
            }
        }

        LogProjection {
            availability: if counters.has_log_omissions() {
                LogAvailability::Partial
            } else {
                LogAvailability::Available
            },
            bytes: output,
            record_count,
        }
    }

    /// Projects the complete lines of the first `bytes_to_read` bytes of `file`; returns how
    /// many bytes it read and the record count after them.
    fn read_file(
        &self,
        file: File,
        bytes_to_read: u64,
        output: &mut Vec<u8>,
        counters: &mut Omissions,
        mut record_count: usize,
    ) -> io::Result<(u64, usize)> {
        let mut reader = file.take(bytes_to_read);
        let mut buffer = [0_u8; 8 * 1024];
        let mut line = Vec::with_capacity(Self::MAXIMUM_LINE_BYTES);
        let mut oversize = false;
        let mut consumed = 0;
        loop {
            let read = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            consumed += read as u64;
            for &byte in &buffer[..read] {
                if byte == b'\n' {
                    if oversize {
                        counters.oversize_lines += 1;
                    } else {
                        let content = line.strip_suffix(b"\r").unwrap_or(&line);
                        record_count = self.project_line(content, output, counters, record_count);
                    }
                    line.clear();
                    oversize = false;
                } else if !oversize {
                    if line.len() < Self::MAXIMUM_LINE_BYTES {
                        line.push(byte);
                    } else {
                        oversize = true;
                    }
                }
            }
        }
        if oversize {
            counters.oversize_lines += 1;
            counters.partial_lines += 1;
        } else if !line.is_empty() {
            counters.partial_lines += 1;
        }
        Ok((consumed, record_count))
    }

    fn project_line(
        &self,
        line: &[u8],
        output: &mut Vec<u8>,
        counters: &mut Omissions,
        record_count: usize,
    ) -> usize {
        let Some(value) = JsonValue::parse(line, MAXIMUM_RECORD_DEPTH) else {
            counters.malformed_records += 1;
            return record_count;
        };
        let Some(record) = self.project_record(&value, counters) else {
            return record_count;
        };
        if record_count >= Self::MAXIMUM_LOG_RECORDS {
            counters.records_omitted_by_count += 1;
            return record_count;
        }
        let json = record.to_json();
        if output.len() + json.len() + 1 > Self::MAXIMUM_LOGS_BYTES {
            counters.records_omitted_by_output_bytes += 1;
            return record_count;
        }
        output.extend_from_slice(json.as_bytes());
        output.push(b'\n');
        record_count + 1
    }

    fn project_record(&self, root: &JsonValue, counters: &mut Omissions) -> Option<LogRecord> {
        let Some(minute) = root
            .property("TimestampUtc")
            .and_then(JsonValue::string)
            .and_then(|text| parse_utc_timestamp(&text))
        else {
            counters.malformed_records += 1;
            return None;
        };
        let now = self.clock.now();
        if minute.instant > now {
            counters.future_records += 1;
            return None;
        }
        if minute.instant < now - RETENTION {
            counters.expired_records += 1;
            return None;
        }
        let Some(level) = known(root, "Level", &LEVELS) else {
            counters.unknown_levels += 1;
            return None;
        };
        let Some(event) = known(root, "Event", &EVENTS) else {
            counters.unknown_events += 1;
            return None;
        };

        let mut record = LogRecord {
            minute: minute.civil,
            level,
            event,
            failure_category: None,
            power_history_status: None,
            hresult: None,
        };
        if let Some(properties) = root
            .property("Properties")
            .filter(|value| value.is_object())
        {
            let string = |name| properties.property(name).and_then(JsonValue::string);
            if FAILURE_EVENTS.contains(&event) {
                record.failure_category =
                    string("exceptionType").map(|exception| match exception.as_str() {
                        "System.IO.IOException" => FailureCategory::Io,
                        "System.UnauthorizedAccessException" => FailureCategory::Unauthorized,
                        "System.OperationCanceledException"
                        | "System.Threading.Tasks.TaskCanceledException" => {
                            FailureCategory::Cancellation
                        }
                        "System.TimeoutException" => FailureCategory::Timeout,
                        "System.Net.Http.HttpRequestException"
                        | "System.Net.Sockets.SocketException" => FailureCategory::Network,
                        "System.InvalidOperationException" => FailureCategory::InvalidOperation,
                        _ => FailureCategory::Other,
                    });
            }
            if event == POWER_HISTORY_EVENT {
                record.power_history_status = Some(match string("status").as_deref() {
                    Some("Complete") => PowerHistoryStatus::Complete,
                    Some("Unavailable") => PowerHistoryStatus::Unavailable,
                    Some("Truncated") => PowerHistoryStatus::Truncated,
                    _ => PowerHistoryStatus::Unknown,
                });
            }
            if event == HRESULT_EVENT {
                record.hresult = properties.property("hresult").and_then(JsonValue::int32);
            }
        }
        Some(record)
    }
}

named_enum! {
    enum AvailabilityState { Available, Unavailable }
}

named_enum! {
    enum LogAvailability { Available, Partial, Unavailable }
}

named_enum! {
    enum FailureCategory { Io, Unauthorized, Cancellation, Timeout, Network, InvalidOperation, Other }
}

named_enum! {
    enum PowerHistoryStatus { Unknown, Complete, Unavailable, Truncated }
}

struct LogProjection {
    bytes: Vec<u8>,
    record_count: usize,
    availability: LogAvailability,
}

impl LogProjection {
    const UNAVAILABLE: Self = Self {
        bytes: Vec::new(),
        record_count: 0,
        availability: LogAvailability::Unavailable,
    };
}

struct LogCandidate {
    path: PathBuf,
    name: String,
    timestamp: DateTime,
}

struct LogRecord {
    /// The record's UTC time, truncated to the minute.
    minute: DateTime,
    level: &'static str,
    event: &'static str,
    failure_category: Option<FailureCategory>,
    power_history_status: Option<PowerHistoryStatus>,
    hresult: Option<i32>,
}

impl LogRecord {
    fn to_json(&self) -> String {
        JsonObject::write(|object| {
            let minute = self.minute;
            object.name(
                "timestampUtc",
                &format!(
                    "{:04}-{:02}-{:02}T{:02}:{:02}:00+00:00",
                    minute.year(),
                    minute.month(),
                    minute.day(),
                    minute.hour(),
                    minute.minute()
                ),
            );
            object.name("level", self.level);
            object.name("event", self.event);
            if let Some(category) = self.failure_category {
                object.name("failureCategory", category.name());
            }
            if let Some(status) = self.power_history_status {
                object.name("powerHistoryStatus", status.name());
            }
            if let Some(hresult) = self.hresult {
                object.value("hresult", hresult);
            }
        })
    }
}

/// What the export left out or could not read, written to the manifest.
#[derive(Default)]
struct Omissions {
    examined_directory_entries: usize,
    directory_entry_limit_reached: bool,
    directory_enumeration_failed: bool,
    eligible_files: usize,
    selected_files: usize,
    files_omitted_by_count: usize,
    files_omitted_by_input_limit: usize,
    rejected_names: usize,
    rejected_reparse_points: usize,
    rejected_outside_root: usize,
    unreadable_files: usize,
    files_disappeared: usize,
    input_limit_reached: bool,
    input_bytes_omitted: u64,
    malformed_records: usize,
    unknown_events: usize,
    unknown_levels: usize,
    expired_records: usize,
    future_records: usize,
    oversize_lines: usize,
    partial_lines: usize,
    records_omitted_by_count: usize,
    records_omitted_by_output_bytes: usize,
}

impl Omissions {
    fn has_log_omissions(&self) -> bool {
        self.directory_entry_limit_reached
            || self.directory_enumeration_failed
            || self.input_limit_reached
            || [
                self.files_omitted_by_count,
                self.files_omitted_by_input_limit,
                self.rejected_names,
                self.rejected_reparse_points,
                self.rejected_outside_root,
                self.unreadable_files,
                self.files_disappeared,
                self.malformed_records,
                self.unknown_events,
                self.unknown_levels,
                self.expired_records,
                self.future_records,
                self.oversize_lines,
                self.partial_lines,
                self.records_omitted_by_count,
                self.records_omitted_by_output_bytes,
            ]
            .iter()
            .any(|&count| count != 0)
    }

    fn write(&self, object: &mut JsonObject) {
        object.value("examinedDirectoryEntries", self.examined_directory_entries);
        object.value(
            "directoryEntryLimitReached",
            self.directory_entry_limit_reached,
        );
        object.value(
            "directoryEnumerationFailed",
            self.directory_enumeration_failed,
        );
        object.value("eligibleFiles", self.eligible_files);
        object.value("selectedFiles", self.selected_files);
        object.value("filesOmittedByCount", self.files_omitted_by_count);
        object.value(
            "filesOmittedByInputLimit",
            self.files_omitted_by_input_limit,
        );
        object.value("rejectedNames", self.rejected_names);
        object.value("rejectedReparsePoints", self.rejected_reparse_points);
        object.value("rejectedOutsideRoot", self.rejected_outside_root);
        object.value("unreadableFiles", self.unreadable_files);
        object.value("filesDisappeared", self.files_disappeared);
        object.value("inputLimitReached", self.input_limit_reached);
        object.value("inputBytesOmitted", self.input_bytes_omitted);
        object.value("malformedRecords", self.malformed_records);
        object.value("unknownEvents", self.unknown_events);
        object.value("unknownLevels", self.unknown_levels);
        object.value("expiredRecords", self.expired_records);
        object.value("futureRecords", self.future_records);
        object.value("oversizeLines", self.oversize_lines);
        object.value("partialLines", self.partial_lines);
        object.value("recordsOmittedByCount", self.records_omitted_by_count);
        object.value(
            "recordsOmittedByOutputBytes",
            self.records_omitted_by_output_bytes,
        );
    }
}

fn manifest(snapshot: &DiagnosticSnapshot, logs: LogAvailability, omissions: &Omissions) -> String {
    type Export<'a> = DiagnosticExport<'a>;
    JsonObject::write(|root| {
        root.value("schemaVersion", 1);
        root.object("limits", |limits| {
            limits.value("retentionDays", RETENTION_DAYS);
            limits.value("maximumDirectoryEntries", Export::MAXIMUM_DIRECTORY_ENTRIES);
            limits.value("maximumLogFiles", Export::MAXIMUM_LOG_FILES);
            limits.value("maximumInputBytes", Export::MAXIMUM_INPUT_BYTES);
            limits.value("maximumLineBytes", Export::MAXIMUM_LINE_BYTES);
            limits.value("maximumLogRecords", Export::MAXIMUM_LOG_RECORDS);
            limits.value("maximumLogsBytes", Export::MAXIMUM_LOGS_BYTES);
            limits.value("maximumSnapshotBytes", Export::MAXIMUM_SNAPSHOT_BYTES);
            limits.value("maximumManifestBytes", Export::MAXIMUM_MANIFEST_BYTES);
            limits.value(
                "maximumUncompressedBytes",
                Export::MAXIMUM_UNCOMPRESSED_BYTES,
            );
            limits.value("maximumArchiveBytes", Export::MAXIMUM_ARCHIVE_BYTES);
        });
        root.object("availability", |availability| {
            let settings = if snapshot.settings.is_some() {
                AvailabilityState::Available
            } else {
                AvailabilityState::Unavailable
            };
            let summary = snapshot
                .summary
                .as_ref()
                .map_or(DiagnosticSummaryAvailability::Unavailable, |summary| {
                    summary.availability
                });
            availability.name("settings", settings.name());
            availability.name("summary", summary.name());
            availability.name("logs", logs.name());
        });
        root.object("omissions", |object| omissions.write(object));
    })
}

/// Writes the archive to `file` and returns its length.
fn write_archive(
    file: &mut File,
    entries: &[(&str, &[u8])],
    modified: DateTime,
) -> Result<u64, DiagnosticExportError> {
    let mut output = BufWriter::with_capacity(64 * 1024, &mut *file);
    zip::write(&mut output, entries, modified)?;
    output.flush()?;
    drop(output);
    file.sync_all()?;
    let archive_bytes = file.metadata()?.len();
    if archive_bytes > DiagnosticExport::MAXIMUM_ARCHIVE_BYTES {
        return Err(DiagnosticExportError::ArchiveTooLarge);
    }
    Ok(archive_bytes)
}

fn enumerate_candidates(root: &Path, counters: &mut Omissions) -> Vec<LogCandidate> {
    let mut candidates = Vec::new();
    match fs::read_dir(root) {
        Err(_) => counters.directory_enumeration_failed = true,
        Ok(mut entries) => {
            while counters.examined_directory_entries < DiagnosticExport::MAXIMUM_DIRECTORY_ENTRIES
            {
                let entry = match entries.next() {
                    None => break,
                    Some(Ok(entry)) => entry,
                    Some(Err(_)) => {
                        counters.directory_enumeration_failed = true;
                        break;
                    }
                };
                counters.examined_directory_entries += 1;
                match candidate(&entry) {
                    Ok(Ok(candidate)) => candidates.push(candidate),
                    Ok(Err(Rejection::Name)) => counters.rejected_names += 1,
                    Ok(Err(Rejection::ReparsePoint)) => counters.rejected_reparse_points += 1,
                    Err(_) => counters.unreadable_files += 1,
                }
            }
            if !counters.directory_enumeration_failed {
                counters.directory_entry_limit_reached = counters.examined_directory_entries
                    == DiagnosticExport::MAXIMUM_DIRECTORY_ENTRIES;
            }
        }
    }
    counters.eligible_files = candidates.len();
    candidates
}

enum Rejection {
    Name,
    ReparsePoint,
}

fn candidate(entry: &DirEntry) -> io::Result<Result<LogCandidate, Rejection>> {
    let name = entry.file_name();
    let timestamp = name.to_str().and_then(parse_log_file_name);
    let (Some(name), Some(timestamp)) = (name.to_str(), timestamp) else {
        return Ok(Err(Rejection::Name));
    };
    if is_directory(entry)? {
        return Ok(Err(Rejection::Name));
    }
    if is_reparse_point(&entry.metadata()?) {
        return Ok(Err(Rejection::ReparsePoint));
    }
    Ok(Ok(LogCandidate {
        path: entry.path(),
        name: name.to_owned(),
        timestamp,
    }))
}

/// .NET lists a link to a directory as a directory.
#[cfg(not(windows))]
pub(super) fn is_directory(entry: &DirEntry) -> io::Result<bool> {
    let file_type = entry.file_type()?;
    Ok(file_type.is_dir()
        || (file_type.is_symlink() && fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir())))
}

#[cfg(windows)]
pub(super) fn is_directory(entry: &DirEntry) -> io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    Ok(entry.metadata()?.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0)
}

/// `metadata` must describe the path itself, not what it links to.
#[cfg(not(windows))]
fn is_reparse_point(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn is_reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn has_reparse_point_in_chain(path: &Path) -> io::Result<bool> {
    for ancestor in path.ancestors() {
        if is_reparse_point(&fs::symlink_metadata(ancestor)?) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The UTC time in `worktime-yyyyMMdd-HHmmss-fffffff-<32 lowercase hex digits>.jsonl`, the name
/// the logger gives its files.
fn parse_log_file_name(name: &str) -> Option<DateTime> {
    let stamp = name
        .strip_prefix("worktime-")?
        .strip_suffix(".jsonl")?
        .as_bytes();
    if stamp.len() != 56
        || [stamp[8], stamp[15], stamp[23]] != [b'-'; 3]
        || !stamp[24..]
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    let field = |range: std::ops::Range<usize>| decimal(&stamp[range]);
    civil_date_time(
        field(0..4)?,
        field(4..6)?,
        field(6..8)?,
        field(9..11)?,
        field(11..13)?,
        field(13..15)?,
        field(16..23)? * 100,
    )
}

fn is_blank(path: &Path) -> bool {
    path.to_str().is_some_and(|text| text.trim().is_empty())
}

/// The value of the string property `name` when it is one of `allowed`.
fn known(object: &JsonValue, name: &str, allowed: &[&'static str]) -> Option<&'static str> {
    let value = object.property(name)?.string()?;
    allowed
        .iter()
        .copied()
        .find(|candidate| *candidate == value)
}

struct ParsedTimestamp {
    instant: Timestamp,
    civil: DateTime,
}

/// A JSON timestamp as `JsonElement.TryGetDateTimeOffset` reads it, kept only with a zero
/// offset: `yyyy-MM-ddTHH:mm[:ss[.f{0,16}]]` then `Z`, `+00`, `-00`, `+00:00` or `-00:00`.
/// Fraction digits past the seventh are ignored, as .NET keeps 100 ns ticks. A timestamp with no
/// offset, which .NET reads in the local zone, is not kept even where that zone is UTC.
fn parse_utc_timestamp(text: &str) -> Option<ParsedTimestamp> {
    let bytes = text.as_bytes();
    let number = |range: std::ops::Range<usize>| decimal(bytes.get(range)?);
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
    {
        return None;
    }
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let mut index = 16;
    let mut second = 0;
    let mut nanosecond = 0;
    if bytes.get(index) == Some(&b':') {
        second = number(index + 1..index + 3)?;
        index += 3;
        if bytes.get(index) == Some(&b'.') {
            index += 1;
            let digits = bytes[index..]
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            if digits > 16 {
                return None;
            }
            let ticks = (0..7).fold(0, |ticks, position| {
                let digit = if position < digits {
                    i32::from(bytes[index + position] - b'0')
                } else {
                    0
                };
                ticks * 10 + digit
            });
            nanosecond = ticks * 100;
            index += digits;
        }
    }
    if !matches!(
        &bytes[index..],
        b"Z" | b"+00" | b"-00" | b"+00:00" | b"-00:00"
    ) {
        return None;
    }
    let civil = civil_date_time(year, month, day, hour, minute, second, nanosecond)?;
    Some(ParsedTimestamp {
        instant: civil.to_zoned(TimeZone::UTC).ok()?.timestamp(),
        civil,
    })
}

/// A date and time .NET can hold: years 1 to 9999.
pub(super) fn civil_date_time(
    year: i32,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
    second: i32,
    nanosecond: i32,
) -> Option<DateTime> {
    if !(1..=9999).contains(&year) {
        return None;
    }
    DateTime::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
        i8::try_from(hour).ok()?,
        i8::try_from(minute).ok()?,
        i8::try_from(second).ok()?,
        nanosecond,
    )
    .ok()
}

/// The value of a run of ASCII digits; `None` when it is empty or holds anything else.
pub(super) fn decimal(digits: &[u8]) -> Option<i32> {
    if digits.is_empty() {
        return None;
    }
    digits.iter().try_fold(0_i32, |value, &digit| {
        digit
            .is_ascii_digit()
            .then(|| value * 10 + i32::from(digit - b'0'))
    })
}

/// 32 hexadecimal digits for a temporary file name, unique within this process and unlikely to
/// repeat across processes.
pub(super) fn unique_hex() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!(
        "{:016x}{:08x}{:08x}",
        nanos as u64,
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
