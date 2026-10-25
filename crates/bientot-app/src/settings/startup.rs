//! « Démarrage Windows », ported from `Services/StartupSettingsService.cs`: what the switch
//! shows for each state of the registration, and the change it asks Windows for.

use bientot_windows::startup::{
    PackagedStartupTask, RunKey, StartupRegistration, StartupRegistrationState,
};

use crate::strings::Key;

/// The registration of this copy of WorkTime. Reading and changing it may wait for Windows,
/// so the window calls it off its thread.
pub trait StartupPort: Send + Sync {
    fn state(&self) -> StartupRegistrationState;

    /// The state Windows settled on.
    fn set_enabled(&self, enabled: bool) -> StartupRegistrationState;
}

impl<K, T> StartupPort for StartupRegistration<K, T>
where
    K: RunKey + Send + Sync,
    T: PackagedStartupTask + Send + Sync,
{
    fn state(&self) -> StartupRegistrationState {
        StartupRegistration::state(self)
    }

    fn set_enabled(&self, enabled: bool) -> StartupRegistrationState {
        StartupRegistration::set_enabled(self, enabled)
    }
}

/// The switch is on.
pub fn is_on(state: StartupRegistrationState) -> bool {
    matches!(
        state,
        StartupRegistrationState::Enabled | StartupRegistrationState::EnabledByPolicy
    )
}

/// What Windows Settings and policies decide is not WorkTime's to change.
pub fn can_change(state: StartupRegistrationState) -> bool {
    matches!(
        state,
        StartupRegistrationState::Disabled
            | StartupRegistrationState::Enabled
            | StartupRegistrationState::RegistrationMismatch
    )
}

/// Whether WorkTime starts at sign-in, or `None` when that cannot be known.
fn enabled(state: StartupRegistrationState) -> Option<bool> {
    match state {
        StartupRegistrationState::Unavailable => None,
        state => Some(is_on(state)),
    }
}

/// The line under the switch.
pub fn status(state: StartupRegistrationState) -> Key {
    match state {
        StartupRegistrationState::Unavailable => Key::StartupUnavailable,
        StartupRegistrationState::Disabled => Key::StartupDisabled,
        StartupRegistrationState::DisabledByUser => Key::StartupDisabledByUser,
        StartupRegistrationState::DisabledByPolicy => Key::StartupDisabledByPolicy,
        StartupRegistrationState::Enabled => Key::StartupEnabled,
        StartupRegistrationState::EnabledByPolicy => Key::StartupEnabledByPolicy,
        StartupRegistrationState::RegistrationMismatch => Key::StartupRegistrationMismatch,
    }
}

/// Asks Windows to turn start with Windows to `requested`, and returns the state it settled on.
/// The registration is read first, never taken from the window: the user may have lifted a
/// block in Windows Settings since. A switch already there is left as is; one the user or a
/// policy holds is left alone.
pub fn apply(port: &dyn StartupPort, requested: bool) -> StartupRegistrationState {
    let current = port.state();
    if is_on(current) == requested || !can_change(current) {
        current
    } else {
        port.set_enabled(requested)
    }
}

/// Windows took the change: the settings may record it.
pub fn applied(settled: StartupRegistrationState, requested: bool) -> bool {
    enabled(settled) == Some(requested)
}

/// What the settings record once Windows `settled` on a state for `requested`: what Windows
/// does, or the request when that cannot be known. Ported from `ResolvePersistedValue`.
pub fn persisted(settled: StartupRegistrationState, requested: bool) -> bool {
    enabled(settled).unwrap_or(requested)
}

#[cfg(test)]
mod tests;
