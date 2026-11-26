//! Waiting for the Bientôt that restarts this one to end, so that this one can hold the data
//! folder and open its ledger: `/proc/<pid>/stat`, read until the process is gone.

use std::time::{Duration, Instant};

/// How often the process is looked at again.
const POLL: Duration = Duration::from_millis(50);

/// Returns when process `pid` has exited or `timeout` passed; whether it exited.
pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
        if stat.as_deref().map_or(true, has_exited) {
            return true;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        std::thread::sleep(remaining.min(POLL));
    }
}

/// Whether the process of `stat`, a `/proc/<pid>/stat` line, has exited: a zombie waiting for
/// its parent, or dead. The state follows the command's name, which may hold `)`.
fn has_exited(stat: &str) -> bool {
    stat.rsplit_once(") ")
        .and_then(|(_, rest)| rest.chars().next())
        .is_none_or(|state| matches!(state, 'Z' | 'X' | 'x'))
}

#[cfg(test)]
mod tests;
