//! One WorkTime per data directory, the role `AppInstance.FindOrRegisterForKey` plays in the .NET
//! `Program.cs`: the first launch for a folder owns it, and a later launch hands its activation to
//! that one, which shows its window, then exits before it opens the database, the log or a window.
//!
//! The owner holds the only instance of a named pipe, named by the key and the sign-in session.
//! The pipe exists only while the owner's handle is open, so it vanishes when the owner leaves,
//! even in a crash. A later launch connects, sends a request on its own connection and waits for
//! the owner's answer on that same connection; when the owner leaves first, the connection breaks
//! and the launch takes the folder over. A launch that connects before the owner listens waits,
//! as the .NET app queues early redirects. The .NET key is not visible to this one, so a .NET and
//! a Rust WorkTime on the same folder do not see each other.

#[cfg(windows)]
mod native;

use bientot_store::data_directory::DataDirectory;

#[cfg(windows)]
pub use native::{ActivationListener, Instance, PrimaryInstance, claim};

/// The name of the pipe that owns `directory` in the sign-in session `session`. Pipe names are
/// global, unlike the `Local\` namespace of the .NET key, so the session id keeps each sign-in
/// session to its own WorkTime.
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "only the Windows claim names the pipe outside the tests"
    )
)]
fn pipe_name(directory: &DataDirectory, session: u32) -> String {
    format!(r"\\.\pipe\{}-{session}", directory.instance_key())
}

#[cfg(test)]
mod tests;
