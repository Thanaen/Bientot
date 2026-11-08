//! Plugin log files (FR-034): `logs\plugins\<id>.jsonl`, one JSON line per entry with the
//! plugin's words in `text`, at most 1 MiB before the file turns into `<id>.1.jsonl`, and none
//! older than 14 days. They sit in a folder the diagnostic export does not read.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use bientot_plugin_api::PluginId;
use serde_json::json;

use crate::host::{Clock, LogLevel, LogSink};

pub const MAX_FILE_BYTES: u64 = 1 << 20;
pub const RETENTION: Duration = Duration::from_secs(14 * 24 * 3600);

pub struct LogFiles {
    directory: PathBuf,
    clock: Clock,
    lock: Mutex<()>,
}

impl LogFiles {
    /// `directory` is `logs\plugins\`.
    pub fn new(directory: PathBuf, clock: Clock) -> LogFiles {
        LogFiles {
            directory,
            clock,
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self, plugin: &PluginId) -> PathBuf {
        self.directory.join(format!("{}.jsonl", plugin.as_str()))
    }

    /// Deletes the files nobody wrote to for 14 days.
    pub fn prune(&self) {
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return;
        };
        let now = SystemTime::now();
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|modified| now.duration_since(modified).unwrap_or_default() > RETENTION);
            if old {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    fn append(&self, plugin: &PluginId, mut entry: serde_json::Value) {
        entry["time"] = json!((self.clock)().to_string());
        let mut line = entry.to_string();
        line.push('\n');
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        // A log that cannot be written is not worth failing the plugin for.
        let _ = self.write_line(&self.path(plugin), line.as_bytes());
    }

    fn write_line(&self, path: &Path, line: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.directory)?;
        let size = fs::metadata(path).map_or(0, |m| m.len());
        if size + line.len() as u64 > MAX_FILE_BYTES {
            fs::rename(path, path.with_extension("1.jsonl"))?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?
            .write_all(line)
    }
}

impl LogSink for LogFiles {
    fn write(&self, plugin: &PluginId, level: LogLevel, text: &str) {
        let level = match level {
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        };
        self.append(plugin, json!({ "level": level, "text": text }));
    }

    fn dropped(&self, plugin: &PluginId, count: u32) {
        self.append(plugin, json!({ "source": "host", "dropped": count }));
    }

    fn host(&self, plugin: &PluginId, text: &str) {
        self.append(plugin, json!({ "source": "host", "text": text }));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn files(directory: &Path) -> LogFiles {
        LogFiles::new(
            directory.join("plugins"),
            Arc::new(|| "2026-11-08T12:00:00Z".parse().unwrap()),
        )
    }

    #[test]
    fn lines_are_json_with_the_text_kept() {
        let dir = tempfile::tempdir().unwrap();
        let logs = files(dir.path());
        let id = PluginId::parse("bientot.focus").unwrap();
        logs.write(&id, LogLevel::Warn, "token=abc \"quoted\"");
        logs.dropped(&id, 3);
        logs.host(&id, "trap: unreachable");
        let text = fs::read_to_string(logs.path(&id)).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["text"], "token=abc \"quoted\"");
        assert_eq!(lines[0]["level"], "warn");
        assert_eq!(lines[1]["dropped"], 3);
        assert_eq!(lines[2]["source"], "host");
        assert_eq!(lines[2]["time"], "2026-11-08T12:00:00Z");
    }

    #[test]
    fn a_full_file_rotates() {
        let dir = tempfile::tempdir().unwrap();
        let logs = files(dir.path());
        let id = PluginId::parse("bientot.focus").unwrap();
        let text = "x".repeat(2000);
        for _ in 0..600 {
            logs.write(&id, LogLevel::Info, &text);
        }
        let current = fs::metadata(logs.path(&id)).unwrap().len();
        let previous = fs::metadata(logs.path(&id).with_extension("1.jsonl"))
            .unwrap()
            .len();
        assert!(current <= MAX_FILE_BYTES && previous <= MAX_FILE_BYTES);
        assert!(current + previous > MAX_FILE_BYTES);
    }
}
