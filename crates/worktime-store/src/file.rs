//! File replacement shared by `settings.json` and the secrets, which .NET both write the same way.

use std::ffi::OsString;
use std::fs;
use std::hash::{BuildHasher, RandomState};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Replaces `path` atomically: `contents` are written and flushed to
/// `<path>.<32 hex digits>.tmp`, which is then renamed over `path`. The temporary file is removed
/// when any step fails.
pub(crate) fn replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    let temporary = sibling(path, &format!(".{}.tmp", random_hex()));
    let written = write_new(&temporary, contents).and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// `path` with `suffix` appended to its file name.
pub(crate) fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut sibling = OsString::from(path);
    sibling.push(suffix);
    PathBuf::from(sibling)
}

/// 128 random bits as 32 lowercase hex digits, the shape of a .NET `Guid.ToString("N")`.
pub(crate) fn random_hex() -> String {
    let state = RandomState::new();
    format!("{:016x}{:016x}", state.hash_one(1u8), state.hash_one(2u8))
}

fn write_new(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = fs::File::create_new(path)?;
    file.write_all(contents)?;
    file.sync_all()
}
