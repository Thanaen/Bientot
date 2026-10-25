//! Microsoft Store updates, ported from `Updates/StoreContextUpdatePort.cs` and the services
//! `Updates/StoreUpdateService.cs` and `Updates/UpdateCheck.cs` (FR-042). Only the MSIX build has
//! a Store to ask; on Windows [`StoreUpdates`] asks it through WinRT's `StoreContext`.
//!
//! The host checks at startup, then every [`RECHECK_INTERVAL`]; [`StoreUpdateService`] asks the
//! Store at most once a day of those.

#[cfg(windows)]
mod native;

use std::future::Future;
use std::time::Duration;

use bientot_domain::configuration::UpdateSettings;
use bientot_domain::time::Clock;
use jiff::{SignedDuration, Timestamp};

#[cfg(windows)]
pub use native::{StoreUpdateOffer, StoreUpdates};

/// How often the host calls [`StoreUpdateService::find_due_update`] after the startup check.
pub const RECHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);

const CHECK_INTERVAL: SignedDuration = SignedDuration::from_hours(24);

/// Whether the Store is due to be asked: checks are on, and the last one is a day old, missing,
/// or in the future. A last check in the future means the clock moved back; waiting for it would
/// block checks.
pub fn is_due(settings: &UpdateSettings, now: Timestamp) -> bool {
    settings.check_for_updates
        && settings
            .last_checked_at
            .is_none_or(|last| last > now || now.duration_since(last) >= CHECK_INTERVAL)
}

/// The Microsoft Store, asked about this package.
pub trait UpdateStore {
    type Offer;
    type Error;

    /// The updates to offer, or `None` when there are none.
    fn find_update(&self) -> impl Future<Output = Result<Option<Self::Offer>, Self::Error>>;
}

/// An update the Store found.
pub trait UpdateOffer {
    type Error;

    /// Whether a successful install needs WorkTime to quit: the updater of the installed build
    /// installs while WorkTime is closed, then starts it again.
    const RESTARTS: bool = false;

    /// Hands the update to the Store. Windows asks the user before downloading, and installing
    /// may close WorkTime. Completes without error when the user declines.
    fn download_and_install(&self) -> impl Future<Output = Result<(), Self::Error>>;
}

/// The settings the host keeps current: the update settings, and where the check is recorded.
pub trait UpdateCheckSettings {
    type Error;

    fn updates(&self) -> UpdateSettings;

    /// Saves `at` as `LastCheckedAtUtc`, then makes it current.
    fn record_check(&mut self, at: Timestamp) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateCheckError<S, T> {
    /// The check could not be recorded, so the Store was not asked.
    Settings(S),
    Store(T),
}

impl<S: std::fmt::Display, T: std::fmt::Display> std::fmt::Display for UpdateCheckError<S, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Settings(error) => write!(f, "the update check could not be recorded: {error}"),
            Self::Store(error) => write!(f, "the Microsoft Store could not be asked: {error}"),
        }
    }
}

impl<S, T> std::error::Error for UpdateCheckError<S, T>
where
    S: std::fmt::Debug + std::fmt::Display,
    T: std::fmt::Debug + std::fmt::Display,
{
}

/// Asks the Store for updates at most once a day.
pub struct StoreUpdateService<U, S, C> {
    /// `None` without package identity: the portable build has no Store to ask.
    store: Option<U>,
    settings: S,
    clock: C,
}

impl<U: UpdateStore, S: UpdateCheckSettings, C: Clock> StoreUpdateService<U, S, C> {
    pub fn new(store: Option<U>, settings: S, clock: C) -> Self {
        Self {
            store,
            settings,
            clock,
        }
    }

    /// The update to offer, or `None` when no check was due or nothing was found. A failed check
    /// still counts as the day's check; the host reports it and offers nothing. Dropping the
    /// future abandons the check without an error.
    pub async fn find_due_update(
        &mut self,
    ) -> Result<Option<U::Offer>, UpdateCheckError<S::Error, U::Error>> {
        let Some(store) = &self.store else {
            return Ok(None);
        };
        let now = self.clock.now();
        if !is_due(&self.settings.updates(), now) {
            return Ok(None);
        }
        // Recorded before asking, so a failing or hanging Store is still asked only once a day.
        self.settings
            .record_check(now)
            .map_err(UpdateCheckError::Settings)?;
        store.find_update().await.map_err(UpdateCheckError::Store)
    }
}

#[cfg(test)]
mod tests;
