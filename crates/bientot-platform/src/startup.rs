//! The states a registration to start at sign-in reports, and how a launch started.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StartupRegistrationState {
    Unavailable,
    Disabled,
    DisabledByUser,
    DisabledByPolicy,
    Enabled,
    EnabledByPolicy,
    /// The `Run` value starts something other than this executable.
    RegistrationMismatch,
}

/// How a launch started: by the user, or by the system as the user signed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LaunchIntent {
    Manual,
    SignIn,
}
