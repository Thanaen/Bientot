//! `ext_idle_notifier_v1`, the Wayland protocol that KWin, Sway, Hyprland, COSMIC and other
//! compositors offer, but not Mutter.
//!
//! The compositor never tells how long the user has been idle: one notification with a
//! [`TIMEOUT`] says `idled` once no input came for that long, then `resumed` at the next input.
//! [`Notification`] keeps the last of these and computes the idle time from it.

#[cfg(target_os = "linux")]
mod native;

#[cfg(target_os = "linux")]
pub(super) use native::open;

use std::time::Duration;

/// The timeout of the notification.
pub const TIMEOUT: Duration = Duration::from_millis(5_000);

/// The last event of the notification, at the monotonic instant it arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notification {
    /// `resumed`: input at that instant. The notification's creation counts as one, since what
    /// came before is not known.
    Resumed(Duration),
    /// `idled`: no input since [`TIMEOUT`] before that instant.
    Idled(Duration),
}

impl Notification {
    /// The idle time at the monotonic instant `now`. While no `idled` follows a `resumed`, input
    /// came within the timeout, but not earlier than the `resumed`: the idle time is the longest
    /// those two allow, so that an input followed by none gives its exact age, as Windows does.
    pub fn idle_at(self, now: Duration) -> Duration {
        match self {
            Self::Resumed(at) => now.saturating_sub(at).min(TIMEOUT),
            Self::Idled(at) => now.saturating_sub(at) + TIMEOUT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds(seconds: f64) -> Duration {
        Duration::from_secs_f64(seconds)
    }

    #[test]
    fn idled_dates_the_last_input_one_timeout_before_it() {
        let idled = Notification::Idled(seconds(100.0));

        assert_eq!(
            [100.0, 101.5, 400.0].map(|now| idled.idle_at(seconds(now))),
            [seconds(5.0), seconds(6.5), seconds(305.0)]
        );
    }

    #[test]
    fn resumed_counts_from_itself_until_the_timeout_says_input_went_on() {
        let resumed = Notification::Resumed(seconds(100.0));

        assert_eq!(
            [100.0, 103.25, 105.0, 3_600.0].map(|now| resumed.idle_at(seconds(now))),
            [seconds(0.0), seconds(3.25), seconds(5.0), seconds(5.0)]
        );
    }

    #[test]
    fn an_instant_before_the_event_reads_as_the_event() {
        assert_eq!(
            Notification::Idled(seconds(100.0)).idle_at(seconds(99.0)),
            seconds(5.0)
        );
        assert_eq!(
            Notification::Resumed(seconds(100.0)).idle_at(seconds(99.0)),
            Duration::ZERO
        );
    }
}
