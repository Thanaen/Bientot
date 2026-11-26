//! One Bientôt per data folder (research.md, R11; contracts/linux-backends.md, « Single
//! instance »), the role the named pipe plays on Windows.
//!
//! The first launch for a folder holds an advisory lock (`flock`) on `<stem>.lock` for its life
//! and listens on the stream socket `<stem>.sock` beside it. A later launch that cannot take the
//! lock connects, sends a request and waits for the owner's answer on that connection, in the
//! framing of the Windows pipe: one byte each way. The kernel releases the lock when the owner
//! dies, even by `kill -9`, so the socket it leaves is known stale and replaced. Both files live
//! in the user's runtime folder, else in the data folder itself.

#[cfg(target_os = "linux")]
mod native;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
pub use native::{ActivationListener, PrimaryInstance};

/// The folder that holds the lock and the socket of the data folder `data_folder`:
/// `$XDG_RUNTIME_DIR/bientot`, private to the user and emptied at each sign-in, or
/// `<data folder>/run` when the session has no runtime folder, as under `su` or a bare `startx`.
/// The base directory specification has a relative value ignored.
#[cfg_attr(
    not(target_os = "linux"),
    allow(
        dead_code,
        reason = "only the Linux lock names its files outside the tests"
    )
)]
fn instance_folder(xdg_runtime_dir: Option<OsString>, data_folder: &Path) -> PathBuf {
    xdg_runtime_dir
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map_or_else(
            || data_folder.join("run"),
            |runtime| runtime.join("bientot"),
        )
}

/// The name of the lock and the socket of the instance key `key`: its first 41 characters,
/// `WorkTime-` and 128 bits of its hash. A socket's path must fit the 108 bytes of `sun_path`,
/// which the whole key under a data folder would not.
#[cfg_attr(
    not(target_os = "linux"),
    allow(
        dead_code,
        reason = "only the Linux lock names its files outside the tests"
    )
)]
fn file_stem(key: &str) -> &str {
    key.get(..41).unwrap_or(key)
}

#[cfg(test)]
mod tests;
