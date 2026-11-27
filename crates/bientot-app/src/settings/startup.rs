//! « Démarrage Windows », ported from `Services/StartupSettingsService.cs`: what the switch
//! shows for each state of the registration, and the change it asks Windows for.

use bientot_platform::startup::{StartupPort, StartupRegistrationState};

use crate::strings::Key;

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
            | StartupRegistrationState::RegistrationMoved
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
        StartupRegistrationState::RegistrationMoved => Key::StartupMismatchAppImageText,
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
fn applied(settled: StartupRegistrationState, requested: bool) -> bool {
    enabled(settled) == Some(requested)
}

/// What Windows does with start with Windows once it `settled` on a state for `requested`, or
/// `None` when it refused a change it may still take on a retry. Startup apps and policies
/// leave nothing to retry.
pub fn settled(settled: StartupRegistrationState, requested: bool) -> Option<bool> {
    if can_change(settled) && !applied(settled, requested) {
        None
    } else {
        enabled(settled)
    }
}

/// What the settings record once Windows `settled` on a state for `requested`: what Windows
/// does, or the request when that cannot be known. Ported from `ResolvePersistedValue`.
pub fn persisted(settled: StartupRegistrationState, requested: bool) -> bool {
    enabled(settled).unwrap_or(requested)
}

#[cfg(test)]
mod tests;
