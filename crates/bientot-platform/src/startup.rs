//! The states a registration to start at sign-in reports.

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
