//! When the shell sends the anonymous daily usage signal, and what it logs. The sender and the
//! rule for a signal due are `worktime_core::usage_ping`; the HTTPS transport exists only in a
//! build with the `usage-ping` feature, so that no other build can send a signal.

use std::time::{Duration, Instant};

use gpui::App;
use jiff::civil::Date;
use worktime_core::diagnostics::{DiagnosticLogLevel, DiagnosticValue};
use worktime_core::usage_ping::UsagePingOutcome;

use crate::live::LiveHost;

#[cfg(feature = "usage-ping")]
mod https;

/// The longest wait between two attempts while the date stays the same.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// When to ask the sender: at startup, when the local date changes, then once an hour. The
/// sender sends only while a signal is due, so once it went the hourly attempts send nothing.
#[derive(Debug, Default)]
pub struct Schedule {
    last_attempt: Option<(Date, Instant)>,
}

impl Schedule {
    /// Whether to attempt a send now, on the local date `today`. Records the attempt if so.
    pub fn attempt(&mut self, today: Date, now: Instant) -> bool {
        let due = self.last_attempt.is_none_or(|(date, at)| {
            date != today || now.saturating_duration_since(at) >= RETRY_INTERVAL
        });
        if due {
            self.last_attempt = Some((today, now));
        }
        due
    }
}

/// Logs `outcome` as `usage_ping.sent` or `usage_ping.failed`, with the status code and never
/// the body. A skipped attempt logs nothing.
pub fn log(host: &mut LiveHost, outcome: UsagePingOutcome) {
    let (level, event, status) = match outcome {
        UsagePingOutcome::Skipped => return,
        UsagePingOutcome::Sent => (DiagnosticLogLevel::Information, SENT, Some(204)),
        UsagePingOutcome::SentUnsaved(error) => {
            host.log_failure("app_state.save_failed", &error);
            (DiagnosticLogLevel::Information, SENT, Some(204))
        }
        // The signal itself is wrong: the sender sends nothing more until the next launch.
        UsagePingOutcome::Rejected { status } => (DiagnosticLogLevel::Error, FAILED, Some(status)),
        UsagePingOutcome::Failed { status } => (DiagnosticLogLevel::Warning, FAILED, status),
    };
    let status = status.map_or(DiagnosticValue::Null, |status| u32::from(status).into());
    host.log(level, event, &[("status", status)]);
}

const SENT: &str = "usage_ping.sent";
const FAILED: &str = "usage_ping.failed";

/// Sends the signal from the user's data at startup, then whenever [`Schedule`] says, one send
/// at a time on a background thread. Reads the `usage_ping` setting at each attempt, so that
/// turning it off stops the sends without a restart. Does nothing in a build without the
/// `usage-ping` feature, in a debug build, nor for a demo.
pub fn install(cx: &mut App) {
    #[cfg(feature = "usage-ping")]
    if !cfg!(debug_assertions) {
        sending::install(cx);
    }
    #[cfg(not(feature = "usage-ping"))]
    let _ = cx;
}

#[cfg(feature = "usage-ping")]
mod sending {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui::App;
    use worktime_core::usage_ping::UsagePingSender;

    use super::{Schedule, https::HttpsTransport, log};
    use crate::host::Today;
    use crate::settings_view;

    /// How often the schedule is asked: a new day is seen within it.
    const CHECK_INTERVAL: Duration = Duration::from_secs(30);

    pub fn install(cx: &mut App) {
        let Some(host) = cx.global_mut::<Today>().live() else {
            return;
        };
        let sender = match UsagePingSender::new(
            HttpsTransport,
            host.app_state().clone(),
            settings_view::build(),
        ) {
            Ok(sender) => Arc::new(sender),
            Err(error) => {
                host.log_failure("usage_ping.failed", &error);
                return;
            }
        };
        cx.spawn(async move |cx| {
            let mut schedule = Schedule::default();
            loop {
                let attempt = cx.update(|cx| {
                    let host = cx.global_mut::<Today>().live()?;
                    let today = host.today();
                    schedule
                        .attempt(today, Instant::now())
                        .then(|| (host.settings().usage_ping, today))
                });
                if let Some((enabled, today)) = attempt {
                    let sender = Arc::clone(&sender);
                    let outcome = cx
                        .background_executor()
                        .spawn(async move { sender.send_if_due(enabled, today) })
                        .await;
                    cx.update(|cx| {
                        if let Some(host) = cx.global_mut::<Today>().live() {
                            log(host, outcome);
                        }
                    });
                }
                cx.background_executor().timer(CHECK_INTERVAL).await;
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests;
