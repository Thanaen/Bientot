//! The states a registration to start at sign-in reports, how a launch started, and the port
//! that reads and changes the registration.

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
    /// Only an AppImage reports it: the entry starts a file at another absolute path, as after
    /// the file was moved. Another kind of copy's entry, as the Flatpak's, is a
    /// [`Self::RegistrationMismatch`].
    RegistrationMoved,
}

/// Marks a launch started at sign-in on every system but Windows, whose registrations keep
/// writing their own marker. Matched exactly and case-sensitively.
pub const AUTOSTART_ARGUMENT: &str = "--autostart";

/// How a launch started: by the user, or by the system as the user signed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LaunchIntent {
    Manual,
    SignIn,
}

/// The registration of this copy of Bientôt to start at sign-in. Reading and changing it may
/// wait for the system, so the window calls it off its thread.
pub trait StartupPort: Send + Sync {
    fn state(&self) -> StartupRegistrationState;

    /// The state the system settled on.
    fn set_enabled(&self, enabled: bool) -> StartupRegistrationState;

    /// Another copy of Bientôt starts at sign-in: on Windows, the Store build does, and so does
    /// an installer or portable copy through its `Run` value. Always `false` for those copies
    /// themselves.
    fn other_copy_starts(&self) -> bool;

    /// The page of the system's settings where the user manages the apps that start at sign-in,
    /// or `None` when the system has none.
    fn system_page(&self) -> Option<SystemPage> {
        None
    }
}

/// A page of the system's settings, opened by its URI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SystemPage {
    pub uri: &'static str,
}
