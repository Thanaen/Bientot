//! Sends the anonymous daily usage signal (issue #158). The service is `services/worktime`; the
//! rule for when a signal is due is `bientot_domain::usage_ping`.
//!
//! The caller runs [`UsagePingSender::send_if_due`] on a background thread at startup, when the
//! day changes and then at most once an hour while a signal is due, with the current value of the
//! `usage_ping` setting each time. The HTTP request itself goes through a
//! [`UsagePingTransport`], which the app provides.

use std::error::Error;
use std::io;
use std::sync::{Mutex, TryLockError};
use std::time::Duration;

use bientot_domain::usage_ping::{Build, UsagePing, usage_ping_due};
use bientot_store::app_state::{AppState, AppStateStore};
use jiff::civil::Date;

pub const ENDPOINT: &str = "https://worktime.thanaen.dev/v1/ping";

/// The whole request: a `POST` with these headers and body, and no cookie or other header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PingRequest {
    pub url: &'static str,
    pub headers: [(&'static str, &'static str); 2],
    pub body: String,
    pub timeout: Duration,
}

impl PingRequest {
    fn new(ping: UsagePing) -> Self {
        // `Build` only admits versions without characters that need escaping.
        let body = format!(
            r#"{{"version":"{}","channel":"{}","first_this_month":{}}}"#,
            ping.version,
            ping.channel.as_str(),
            ping.first_this_month
        );
        Self {
            url: ENDPOINT,
            headers: [
                ("Content-Type", "application/json"),
                ("User-Agent", "WorkTime"),
            ],
            body,
            timeout: Duration::from_secs(10),
        }
    }
}

/// Sends a [`PingRequest`] over HTTPS.
pub trait UsagePingTransport {
    /// The status code of the answer, or an error when none arrived.
    fn post(&self, request: &PingRequest) -> Result<u16, Box<dyn Error + Send + Sync>>;
}

/// What one call to [`UsagePingSender::send_if_due`] did. The caller logs a send as
/// `usage_ping.sent` and a rejection or failure as `usage_ping.failed`, with the status code and
/// never the body.
#[derive(Debug)]
pub enum UsagePingOutcome {
    /// Nothing was sent: the setting is off, a signal was already sent today, the service
    /// rejected one earlier in this launch, or another send is in flight.
    Skipped,
    /// The service recorded the signal and its date was saved.
    Sent,
    /// The service recorded the signal but its date could not be saved. This launch sends
    /// nothing more today.
    SentUnsaved(io::Error),
    /// `400` or `413`: the signal itself is wrong. Nothing more is sent until the next launch.
    Rejected { status: u16 },
    /// Any other status, or no answer (`None`): the next attempt tries again.
    Failed { status: Option<u16> },
}

pub struct UsagePingSender<T> {
    transport: T,
    store: AppStateStore,
    build: Build,
    state: Mutex<SenderState>,
}

struct SenderState {
    last_sent: Option<Date>,
    rejected: bool,
}

impl<T: UsagePingTransport> UsagePingSender<T> {
    /// Reads the date of the last signal from `store`.
    pub fn new(transport: T, store: AppStateStore, build: Build) -> io::Result<Self> {
        let AppState {
            usage_ping_last_sent,
            ..
        } = store.load()?;
        Ok(Self {
            transport,
            store,
            build,
            state: Mutex::new(SenderState {
                last_sent: usage_ping_last_sent,
                rejected: false,
            }),
        })
    }

    /// Sends the signal when `usage_ping_due` says one is due on the local date `today`, unless
    /// another call is already sending. The date is saved only after a `204`.
    pub fn send_if_due(&self, enabled: bool, today: Date) -> UsagePingOutcome {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return UsagePingOutcome::Skipped,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        if state.rejected {
            return UsagePingOutcome::Skipped;
        }
        let Some(ping) = usage_ping_due(enabled, state.last_sent, today, self.build) else {
            return UsagePingOutcome::Skipped;
        };
        match self.transport.post(&PingRequest::new(ping)) {
            Ok(204) => {
                state.last_sent = Some(today);
                let saved = self
                    .store
                    .update(|saved| saved.usage_ping_last_sent = Some(today));
                match saved {
                    Ok(()) => UsagePingOutcome::Sent,
                    Err(error) => UsagePingOutcome::SentUnsaved(error),
                }
            }
            Ok(status @ (400 | 413)) => {
                state.rejected = true;
                UsagePingOutcome::Rejected { status }
            }
            Ok(status) => UsagePingOutcome::Failed {
                status: Some(status),
            },
            Err(_) => UsagePingOutcome::Failed { status: None },
        }
    }
}

#[cfg(test)]
mod tests;
