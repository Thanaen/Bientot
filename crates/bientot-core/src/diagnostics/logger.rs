//! The local diagnostic log, ported from `Diagnostics/LocalDiagnosticLogger.cs`,
//! `Diagnostics/DiagnosticLogOptions.cs` and `Diagnostics/DiagnosticLogLevel.cs`.
//!
//! Each write appends one JSON line to `worktime-<UTC start>-<32 hex digits>.jsonl`, the files
//! [`DiagnosticExport`](super::DiagnosticExport) reads. Event and property names are restricted
//! to `[A-Za-z0-9._-]`, so they cannot carry user text. A property whose name suggests a secret,
//! a window title, an application name or a message is written as `[REDACTED]` whatever its
//! value; text values lose bearer tokens and `key=value` credentials and are cut at 2,048 UTF-16
//! units. A file is rotated before an entry would push it past the size limit, and files are
//! pruned to the count and total size limits and to 14 days, before and after every write.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use bientot_domain::configuration::LoggingSettings;
use bientot_domain::time::Clock;
use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::export::{RETENTION, civil_date_time, decimal, is_directory, unique_hex};
use super::json::JsonObject;
use super::named_enum;
use word_characters::WORD_CHARACTERS;

const REDACTED: &str = "[REDACTED]";
const NON_FINITE: &str = "[NON_FINITE]";
/// In UTF-16 units, as .NET counts string length.
const MAXIMUM_STRING_LENGTH: usize = 2_048;
const MAXIMUM_EVENT_NAME_LENGTH: usize = 128;
const MAXIMUM_PROPERTY_NAME_LENGTH: usize = 64;

named_enum! {
    pub enum DiagnosticLogLevel { Debug, Information, Warning, Error }
}

/// The limits of the log files, as `DiagnosticLogOptions.Validate` accepts them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticLogOptions {
    maximum_file_count: i32,
    maximum_file_bytes: i64,
    maximum_total_bytes: i64,
}

impl DiagnosticLogOptions {
    pub const ABSOLUTE_MAXIMUM_TOTAL_BYTES: i64 = 20 * 1024 * 1024;

    /// `None` unless there is at least one file, a file holds 4,096 bytes to the absolute
    /// maximum, and the total holds a file and stays within the absolute maximum.
    pub fn new(
        maximum_file_count: i32,
        maximum_file_bytes: i64,
        maximum_total_bytes: i64,
    ) -> Option<Self> {
        let valid = maximum_file_count >= 1
            && (4_096..=Self::ABSOLUTE_MAXIMUM_TOTAL_BYTES).contains(&maximum_file_bytes)
            && (maximum_file_bytes..=Self::ABSOLUTE_MAXIMUM_TOTAL_BYTES)
                .contains(&maximum_total_bytes);
        valid.then_some(Self {
            maximum_file_count,
            maximum_file_bytes,
            maximum_total_bytes,
        })
    }

    /// The saved limits with the absolute maximum total, as the .NET app builds its logger.
    pub fn from_settings(logging: LoggingSettings) -> Option<Self> {
        Self::new(
            logging.maximum_file_count,
            logging.maximum_file_bytes,
            Self::ABSOLUTE_MAXIMUM_TOTAL_BYTES,
        )
    }
}

impl Default for DiagnosticLogOptions {
    fn default() -> Self {
        Self {
            maximum_file_count: 5,
            maximum_file_bytes: 1_048_576,
            maximum_total_bytes: Self::ABSOLUTE_MAXIMUM_TOTAL_BYTES,
        }
    }
}

/// A property value. The .NET logger wrote any other type as `[UNSUPPORTED]`; here it cannot be
/// passed. An enum is passed as its name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiagnosticValue<'a> {
    Null,
    Bool(bool),
    Integer(i64),
    Unsigned(u64),
    /// NaN and the infinities are written as `[NON_FINITE]`.
    Float(f64),
    /// Written with credentials removed and cut to 2,048 UTF-16 units.
    Text(&'a str),
}

impl<'a> From<&'a str> for DiagnosticValue<'a> {
    fn from(text: &'a str) -> Self {
        Self::Text(text)
    }
}

impl From<bool> for DiagnosticValue<'_> {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i32> for DiagnosticValue<'_> {
    fn from(value: i32) -> Self {
        Self::Integer(value.into())
    }
}

impl From<i64> for DiagnosticValue<'_> {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<u32> for DiagnosticValue<'_> {
    fn from(value: u32) -> Self {
        Self::Unsigned(value.into())
    }
}

impl From<u64> for DiagnosticValue<'_> {
    fn from(value: u64) -> Self {
        Self::Unsigned(value)
    }
}

impl From<f64> for DiagnosticValue<'_> {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl<'a, T: Into<DiagnosticValue<'a>>> From<Option<T>> for DiagnosticValue<'a> {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Null, Into::into)
    }
}

#[derive(Debug)]
pub enum DiagnosticLogError {
    /// Empty, longer than 128 characters or outside `[A-Za-z0-9._-]`.
    InvalidEventName,
    /// Empty, longer than 64 characters, outside `[A-Za-z0-9._-]` or given twice.
    InvalidPropertyName,
    /// The directory or a log file could not be written; the entry may be lost.
    Io(io::Error),
}

impl std::fmt::Display for DiagnosticLogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEventName => f.write_str("invalid diagnostic event name"),
            Self::InvalidPropertyName => f.write_str("invalid diagnostic property name"),
            Self::Io(error) => write!(f, "the diagnostic log could not be written: {error}"),
        }
    }
}

impl std::error::Error for DiagnosticLogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

pub struct LocalDiagnosticLogger<'a> {
    directory: PathBuf,
    options: DiagnosticLogOptions,
    clock: &'a dyn Clock,
    active_file: Option<PathBuf>,
}

impl<'a> LocalDiagnosticLogger<'a> {
    /// The directory is created on the first write.
    pub fn new(
        directory: impl Into<PathBuf>,
        options: DiagnosticLogOptions,
        clock: &'a dyn Clock,
    ) -> Self {
        Self {
            directory: directory.into(),
            options,
            clock,
            active_file: None,
        }
    }

    /// Where the log files are, the directory [`DiagnosticExport`](super::DiagnosticExport)
    /// reads.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Appends one entry. No properties are written as `"Properties":null`, as the .NET callers
    /// that passed none did.
    pub fn write(
        &mut self,
        level: DiagnosticLogLevel,
        event: &str,
        properties: &[(&str, DiagnosticValue)],
    ) -> Result<(), DiagnosticLogError> {
        if !is_valid_name(event, MAXIMUM_EVENT_NAME_LENGTH) {
            return Err(DiagnosticLogError::InvalidEventName);
        }
        for (index, (name, _)) in properties.iter().enumerate() {
            if !is_valid_name(name, MAXIMUM_PROPERTY_NAME_LENGTH)
                || properties[..index].iter().any(|(other, _)| other == name)
            {
                return Err(DiagnosticLogError::InvalidPropertyName);
            }
        }
        let now = self.clock.now();
        let entry = self.serialize(now, level, event, properties);
        self.append(now, entry.as_bytes())
            .map_err(DiagnosticLogError::Io)
    }

    fn serialize(
        &self,
        now: Timestamp,
        level: DiagnosticLogLevel,
        event: &str,
        properties: &[(&str, DiagnosticValue)],
    ) -> String {
        let line = |write_properties: &dyn Fn(&mut JsonObject)| {
            JsonObject::write(|entry| {
                entry.name("TimestampUtc", &json_timestamp(now));
                entry.name("Level", level.name());
                entry.name("Event", event);
                write_properties(entry);
            }) + "\n"
        };
        let entry = line(&|entry| {
            if properties.is_empty() {
                entry.value("Properties", "null");
            } else {
                entry.object("Properties", |object| {
                    for (name, value) in properties {
                        write_property(object, name, *value);
                    }
                });
            }
        });
        if entry.len() as i64 <= self.options.maximum_file_bytes {
            return entry;
        }
        line(&|entry| {
            entry.object("Properties", |object| {
                object.name("loggingFailure", "entry_too_large");
            });
        })
    }

    fn append(&mut self, now: Timestamp, entry: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.directory)?;
        self.prune(now)?;
        let path = match self.active_file.take() {
            Some(path) if !self.should_rotate(&path, entry.len(), now) => path,
            _ => self.directory.join(format!(
                "worktime-{}-{:07}-{}.jsonl",
                now.strftime("%Y%m%d-%H%M%S"),
                now.subsec_nanosecond() / 100,
                unique_hex()
            )),
        };
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        self.active_file = Some(path);
        file.write_all(entry)?;
        file.flush()?;
        self.prune(now)
    }

    fn should_rotate(&self, path: &Path, entry_length: usize, now: Timestamp) -> bool {
        let Ok(metadata) = fs::metadata(path) else {
            return true;
        };
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        metadata.len() as i64 > self.options.maximum_file_bytes - entry_length as i64
            || file_start(name, &metadata) <= now - RETENTION
    }

    /// Deletes the files that started 14 days ago or earlier, then the oldest written beyond the
    /// file count or the total size.
    fn prune(&self, now: Timestamp) -> io::Result<()> {
        let cutoff = now - RETENTION;
        for (path, name, metadata) in self.log_files()? {
            if file_start(&name, &metadata) <= cutoff {
                remove(&path)?;
            }
        }

        let mut files = self.log_files()?;
        files.sort_by(|(_, left_name, left), (_, right_name, right)| {
            modified(right)
                .cmp(&modified(left))
                .then_with(|| right_name.cmp(left_name))
        });
        let mut total_bytes: i64 = files.iter().map(|(_, _, meta)| meta.len() as i64).sum();
        for (index, (path, _, metadata)) in files.iter().enumerate().rev() {
            if index < self.options.maximum_file_count as usize
                && total_bytes <= self.options.maximum_total_bytes
            {
                break;
            }
            total_bytes -= metadata.len() as i64;
            remove(path)?;
        }
        Ok(())
    }

    /// The files `worktime-*.jsonl`, matched without case as .NET matches on Windows.
    fn log_files(&self) -> io::Result<Vec<(PathBuf, String, fs::Metadata)>> {
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if !lower.starts_with("worktime-")
                || !lower.ends_with(".jsonl")
                || is_directory(&entry)?
            {
                continue;
            }
            files.push((entry.path(), name, entry.metadata()?));
        }
        Ok(files)
    }
}

/// Deleting a file that is already gone is not an error, as for `FileInfo.Delete`.
fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn modified(metadata: &fs::Metadata) -> Timestamp {
    metadata
        .modified()
        .ok()
        .and_then(|time| Timestamp::try_from(time).ok())
        .unwrap_or(Timestamp::MIN)
}

/// The UTC time in the 23 characters after `worktime-` (`yyyyMMdd-HHmmss-fffffff`), or the
/// last write time when they hold none.
fn file_start(name: &str, metadata: &fs::Metadata) -> Timestamp {
    let stamp = name.get(9..32).map(str::as_bytes);
    stamp
        .filter(|stamp| stamp[8] == b'-' && stamp[15] == b'-')
        .and_then(|stamp| {
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
        })
        .and_then(|civil| civil.to_zoned(TimeZone::UTC).ok())
        .map_or_else(|| modified(metadata), |zoned| zoned.timestamp())
}

/// Not blank, at most `maximum` characters, and only `[A-Za-z0-9._-]`. The .NET pattern also let
/// a final line feed through, which is refused here.
fn is_valid_name(name: &str, maximum: usize) -> bool {
    !name.is_empty()
        && name.len() <= maximum
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// `DateTimeOffset` as `System.Text.Json` writes a zero offset: seconds, then up to seven
/// fraction digits without trailing zeros, then `+00:00`.
fn json_timestamp(now: Timestamp) -> String {
    let ticks = now.subsec_nanosecond() / 100;
    let fraction = if ticks == 0 {
        String::new()
    } else {
        format!(".{ticks:07}").trim_end_matches('0').to_owned()
    };
    format!("{}{fraction}+00:00", now.strftime("%Y-%m-%dT%H:%M:%S"))
}

fn write_property(object: &mut JsonObject, name: &str, value: DiagnosticValue) {
    if is_sensitive(name) {
        object.name(name, REDACTED);
        return;
    }
    match value {
        DiagnosticValue::Null => object.value(name, "null"),
        DiagnosticValue::Bool(value) => object.value(name, value),
        DiagnosticValue::Integer(value) => object.value(name, value),
        DiagnosticValue::Unsigned(value) => object.value(name, value),
        DiagnosticValue::Float(value) if !value.is_finite() => object.name(name, NON_FINITE),
        DiagnosticValue::Float(value) => object.value(name, dotnet_double(value)),
        DiagnosticValue::Text(text) => object.string(name, &redact(text)),
    }
}

fn is_sensitive(name: &str) -> bool {
    let normalized: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect();
    [
        "apikey",
        "authorization",
        "bearer",
        "password",
        "secret",
        "token",
        "windowtitle",
        "applicationname",
    ]
    .iter()
    .any(|fragment| normalized.contains(fragment))
        || matches!(
            normalized.as_str(),
            "app" | "title" | "message" | "payload" | "eventdata"
        )
}

/// The .NET patterns `(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+`, replaced by `Bearer [REDACTED]`, then
/// `(?i)\b(api[_-]?key|password|secret|token)\s*[:=]\s*[^\s&;,]+`, replaced by the key and
/// `[REDACTED]`; then the cut to 2,048 UTF-16 units and `…`.
///
/// They are matched by hand as .NET matches them: `\b` uses the word characters .NET counts
/// (`word_characters.rs`), and the only non-ASCII letter `(?i)` folds is the Kelvin sign, to `k`.
/// `redacts_generated_texts_as_dotnet_does` replays what the .NET logger wrote for 3,000 texts,
/// since a looser match can start earlier and swallow the keyword that .NET's match starts at.
fn redact(text: &str) -> String {
    let text = replace_matches(text, |rest, start| {
        let mut end = start + letters(&rest[start..], "bearer")?;
        let spaces = run(&rest[end..], char::is_whitespace);
        if spaces == 0 {
            return None;
        }
        end += spaces;
        let token = run(&rest[end..], |character| {
            character.is_ascii_alphanumeric()
                || "._~+/=-".contains(character)
                || character == KELVIN
        });
        (token > 0).then(|| (end + token, format!("Bearer {REDACTED}")))
    });
    let text = replace_matches(&text, |rest, start| {
        let key = ["password", "secret", "token"]
            .iter()
            .find_map(|key| letters(&rest[start..], key))
            .or_else(|| {
                let api = letters(&rest[start..], "api")?;
                let separator = rest[start + api..]
                    .starts_with(['_', '-'])
                    .then_some(1)
                    .and_then(|separator| {
                        letters(&rest[start + api + separator..], "key").map(|key| separator + key)
                    });
                separator
                    .or_else(|| letters(&rest[start + api..], "key"))
                    .map(|key| api + key)
            })?;
        let mut end = start + key;
        end += run(&rest[end..], char::is_whitespace);
        if !rest[end..].starts_with([':', '=']) {
            return None;
        }
        end += 1;
        end += run(&rest[end..], char::is_whitespace);
        let value = run(&rest[end..], |character| {
            !character.is_whitespace() && !"&;,".contains(character)
        });
        (value > 0).then(|| {
            (
                end + value,
                format!("{}{REDACTED}", &rest[start..start + key]),
            )
        })
    });
    truncate(text)
}

/// Replaces, left to right, each match that `matcher` finds where `\b` holds: given the whole
/// text and a start, it returns the match's end and its replacement.
fn replace_matches(text: &str, matcher: impl Fn(&str, usize) -> Option<(usize, String)>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = 0;
    while let Some(character) = text[start..].chars().next() {
        let previous = text[..start].chars().next_back();
        let at_boundary = !previous.is_some_and(is_word_character);
        match at_boundary.then(|| matcher(text, start)).flatten() {
            Some((end, replacement)) => {
                out.push_str(&replacement);
                start = end;
            }
            None => {
                out.push(character);
                start += character.len_utf8();
            }
        }
    }
    out
}

/// The byte length of `literal` (lower-case ASCII letters) at the start of `text`, ignoring case.
fn letters(text: &str, literal: &str) -> Option<usize> {
    let mut length = 0;
    let mut characters = text.chars();
    for expected in literal.chars() {
        let character = characters.next()?;
        if character.to_ascii_lowercase() != expected && !(character == KELVIN && expected == 'k') {
            return None;
        }
        length += character.len_utf8();
    }
    Some(length)
}

const KELVIN: char = '\u{212A}';

fn is_word_character(character: char) -> bool {
    let code_point = u32::from(character);
    WORD_CHARACTERS
        .binary_search_by(|&(first, last)| {
            if last < code_point {
                std::cmp::Ordering::Less
            } else if first > code_point {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// The byte length of the run of characters at the start of `text` that `accept` accepts.
fn run(text: &str, accept: impl Fn(char) -> bool) -> usize {
    text.find(|character| !accept(character))
        .unwrap_or(text.len())
}

/// At most 2,048 UTF-16 units, then `…`. A cut through a surrogate pair keeps its first half,
/// which `System.Text.Json` writes as U+FFFD.
fn truncate(text: String) -> String {
    let mut units = 0;
    for (index, character) in text.char_indices() {
        let width = character.len_utf16();
        if units + width > MAXIMUM_STRING_LENGTH {
            let mut cut = text[..index].to_owned();
            if units < MAXIMUM_STRING_LENGTH {
                cut.push('\u{FFFD}');
            }
            cut.push('…');
            return cut;
        }
        units += width;
    }
    text
}

/// A finite double as .NET writes it: the shortest digits that read back the same value, in
/// fixed notation unless the decimal exponent is below -4 or reaches the larger of 15 and the
/// digit count, where it switches to `d.dddE+XX`.
fn dotnet_double(value: f64) -> String {
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("LowerExp writes an exponent");
    let exponent: i32 = exponent
        .parse()
        .expect("LowerExp writes an integer exponent");
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(mantissa) => ("-", mantissa),
        None => ("", mantissa),
    };
    let digits = mantissa.replace('.', "");
    let count = digits.len() as i32;
    let scale = exponent + 1;
    if !(-3..=17).contains(&scale) {
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() { "" } else { "." };
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        format!(
            "{sign}{first}{point}{rest}E{exponent_sign}{:02}",
            exponent.unsigned_abs()
        )
    } else if scale <= 0 {
        format!(
            "{sign}0.{}{digits}",
            "0".repeat(scale.unsigned_abs() as usize)
        )
    } else if scale >= count {
        format!("{sign}{digits}{}", "0".repeat((scale - count) as usize))
    } else {
        let (whole, fraction) = digits.split_at(scale as usize);
        format!("{sign}{whole}.{fraction}")
    }
}

#[cfg(test)]
mod tests;
mod word_characters;
