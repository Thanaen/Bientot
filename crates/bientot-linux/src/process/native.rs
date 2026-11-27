//! The pidfd the kernel wakes when the process ends (Linux 5.3).

use std::time::Instant;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use rustix::process::{Pid, PidfdFlags, pidfd_open};

/// Waits on a pidfd of `pid` until it has exited, a zombie included, or `deadline` passed;
/// whether it exited. `None` when no pidfd can be had or waited on, as before Linux 5.3 or under
/// a seccomp filter that refuses the call: the caller polls instead.
pub(super) fn wait_on_pidfd(pid: u32, deadline: Instant) -> Option<bool> {
    let pid = Pid::from_raw(i32::try_from(pid).ok()?)?;
    let pidfd = match pidfd_open(pid, PidfdFlags::empty()) {
        Ok(pidfd) => pidfd,
        Err(Errno::SRCH) => return Some(true),
        Err(_) => return None,
    };
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout = Timespec::try_from(remaining).ok()?;
        match poll(&mut [PollFd::new(&pidfd, PollFlags::IN)], Some(&timeout)) {
            Ok(0) => return Some(false),
            Ok(_) => return Some(true),
            Err(Errno::INTR) => {}
            Err(_) => return None,
        }
    }
}
