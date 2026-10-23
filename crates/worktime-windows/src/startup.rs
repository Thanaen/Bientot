//! Start with Windows, ported from `Startup/WindowsStartupService.cs` and
//! `Startup/WindowsLaunchIntent.cs`. The portable build writes a value under the user's `Run` key,
//! which starts WorkTime at sign-in with [`WINDOWS_STARTUP_ARGUMENT`]; the MSIX build turns on the
//! [`PACKAGED_TASK_ID`] startup task its manifest declares instead. Either start launches WorkTime
//! hidden in the notification area.

#[cfg(windows)]
mod native;
#[cfg(windows)]
mod packaged;

use std::ffi::OsStr;
use std::io;
use std::path::Path;

use worktime_store::data_directory::upper_invariant;

#[cfg(windows)]
pub use native::RegistryRunKey;
#[cfg(windows)]
pub use packaged::{WindowsStartupTask, current_activation};

/// The `Run` value the .NET app registers.
pub const REGISTRATION_NAME: &str = "WorkTime";

/// Marks a launch started by the portable (non-MSIX) build's start-with-Windows `Run` value.
/// Matched exactly and case-sensitively.
pub const WINDOWS_STARTUP_ARGUMENT: &str = "--worktime-windows-startup";

/// The `TaskId` of the startup task in `packaging/msix/AppxManifest.xml`, which ships it disabled.
pub const PACKAGED_TASK_ID: &str = "WorkTimeStartupTask";

/// Development hosts that may run WorkTime's code but must never be registered to start with
/// Windows.
const HOST_EXECUTABLE_NAMES: [&str; 5] = [
    "dotnet.exe",
    "testhost.exe",
    "vstest.console.exe",
    "msbuild.exe",
    "devenv.exe",
];

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LaunchIntent {
    Manual,
    WindowsStartup,
}

/// What Windows activated this launch for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Activation {
    /// An ordinary launch, the only kind the portable build gets.
    Launch,
    /// The packaged startup task with this id.
    StartupTask(String),
    /// Anything else, such as a click on a notification.
    Other,
}

impl LaunchIntent {
    /// How a launch started, `windows_startup` when its command line holds
    /// [`WINDOWS_STARTUP_ARGUMENT`]. Only an ordinary launch honours the marker, so that a click on
    /// a notification always shows the window.
    pub fn resolve(windows_startup: bool, activation: &Activation) -> Self {
        let started_by_windows = match activation {
            Activation::Launch => windows_startup,
            Activation::StartupTask(task_id) => task_id == PACKAGED_TASK_ID,
            Activation::Other => false,
        };
        if started_by_windows {
            Self::WindowsStartup
        } else {
            Self::Manual
        }
    }
}

/// What the Startup page of Windows Settings or Task Manager recorded for a `Run` value, in
/// `Explorer\StartupApproved\Run`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StartupApproval {
    /// No record: the user never toggled it.
    None,
    Enabled,
    DisabledByUser,
    /// A record WorkTime does not understand.
    Unknown,
}

/// The `Run` key and its approvals, under `HKEY_CURRENT_USER`.
pub trait RunKey {
    /// The text of the value `name`, unexpanded. `None` when it is missing or not a string.
    fn read_run_value(&self, name: &str) -> io::Result<Option<String>>;

    fn set_run_value(&self, name: &str, command: &str) -> io::Result<()>;

    /// Deleting a missing value succeeds.
    fn delete_run_value(&self, name: &str) -> io::Result<()>;

    fn read_approval(&self, name: &str) -> io::Result<StartupApproval>;
}

/// A stored approval record, as the registry holds it.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the registry reads it outside the tests")
)]
enum ApprovalValue {
    Binary(Vec<u8>),
    OtherType,
}

/// The first four bytes of the record are a little-endian state; Windows writes 2 or 6 for
/// enabled and 3 for disabled. Anything else is [`StartupApproval::Unknown`], which never
/// allows a rewrite.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the registry decodes outside the tests")
)]
fn decode_approval(value: Option<ApprovalValue>) -> StartupApproval {
    let Some(value) = value else {
        return StartupApproval::None;
    };
    let ApprovalValue::Binary(bytes) = value else {
        return StartupApproval::Unknown;
    };
    let Some(state) = bytes.first_chunk::<4>() else {
        return StartupApproval::Unknown;
    };
    match u32::from_le_bytes(*state) {
        0x02 | 0x06 => StartupApproval::Enabled,
        0x03 => StartupApproval::DisabledByUser,
        _ => StartupApproval::Unknown,
    }
}

/// The command WorkTime registers, and the one it registered before launches from the `Run` key
/// were marked.
#[derive(Clone, Debug, PartialEq, Eq)]
struct StartupCommands {
    /// `"<executable>" --worktime-windows-startup`.
    canonical: String,
    /// `"<executable>"`.
    legacy: String,
}

impl StartupCommands {
    /// `None` unless `executable` is a fully qualified path to an existing `.exe` that is not a
    /// development host.
    fn new(executable: &Path) -> Option<Self> {
        let is_exe = executable
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| same_text(extension, "exe"));
        let is_host = executable
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| {
                HOST_EXECUTABLE_NAMES
                    .iter()
                    .any(|host| same_text(name, host))
            });
        if !executable.is_absolute() || !is_exe || is_host || !executable.is_file() {
            return None;
        }
        let legacy = format!("\"{}\"", std::path::absolute(executable).ok()?.to_str()?);
        Some(Self {
            canonical: format!("{legacy} {WINDOWS_STARTUP_ARGUMENT}"),
            legacy,
        })
    }
}

/// .NET's `StringComparison.OrdinalIgnoreCase`, as Windows compares paths.
fn same_text(left: &str, right: &str) -> bool {
    upper_invariant(left) == upper_invariant(right)
}

/// The `Run` value of one executable. Every failure to reach the registry reads as
/// [`StartupRegistrationState::Unavailable`], as in the .NET `WindowsStartupService`.
pub struct PortableStartupRegistration<K> {
    key: K,
    name: String,
    commands: Option<StartupCommands>,
}

impl<K: RunKey> PortableStartupRegistration<K> {
    /// The registration of `executable` under the value `name`. Without a registrable executable
    /// every state is [`StartupRegistrationState::Unavailable`] and nothing is ever written.
    pub fn new(key: K, name: impl Into<String>, executable: Option<&Path>) -> Self {
        Self {
            key,
            name: name.into(),
            commands: executable.and_then(StartupCommands::new),
        }
    }

    /// Reads the state without changing anything, even a mismatch.
    pub fn state(&self) -> StartupRegistrationState {
        self.try_state()
            .unwrap_or(StartupRegistrationState::Unavailable)
    }

    /// Enabling writes the canonical command unless it is already there, repairing a mismatch.
    /// Disabling deletes only this executable's own value, canonical or legacy, and reports
    /// another executable's as a mismatch.
    pub fn set_enabled(&self, enabled: bool) -> StartupRegistrationState {
        self.try_set_enabled(enabled)
            .unwrap_or(StartupRegistrationState::Unavailable)
    }

    /// Rewrites this executable's legacy command to the canonical one, unless the user disabled
    /// it in Windows or its approval is not understood. Failures are ignored: the next start
    /// tries again.
    pub fn migrate_owned_legacy_registration(&self) {
        let _ = self.try_migrate();
    }

    fn try_state(&self) -> io::Result<StartupRegistrationState> {
        let Some(commands) = &self.commands else {
            return Ok(StartupRegistrationState::Unavailable);
        };
        let Some(registered) = self.key.read_run_value(&self.name)? else {
            return Ok(StartupRegistrationState::Disabled);
        };
        let canonical = same_text(&registered, &commands.canonical);
        let approval = self.key.read_approval(&self.name)?;
        Ok(
            if approval == StartupApproval::DisabledByUser
                && (canonical || same_text(&registered, &commands.legacy))
            {
                StartupRegistrationState::DisabledByUser
            } else if !canonical {
                StartupRegistrationState::RegistrationMismatch
            } else if approval == StartupApproval::Unknown {
                StartupRegistrationState::Unavailable
            } else {
                StartupRegistrationState::Enabled
            },
        )
    }

    fn try_set_enabled(&self, enabled: bool) -> io::Result<StartupRegistrationState> {
        let Some(commands) = &self.commands else {
            return Ok(StartupRegistrationState::Unavailable);
        };
        let registered = self.key.read_run_value(&self.name)?;
        if enabled {
            if !registered.is_some_and(|registered| same_text(&registered, &commands.canonical)) {
                self.key.set_run_value(&self.name, &commands.canonical)?;
            }
        } else {
            let Some(registered) = registered else {
                return Ok(StartupRegistrationState::Disabled);
            };
            if !same_text(&registered, &commands.canonical)
                && !same_text(&registered, &commands.legacy)
            {
                return Ok(StartupRegistrationState::RegistrationMismatch);
            }
            self.key.delete_run_value(&self.name)?;
        }
        self.try_state()
    }

    fn try_migrate(&self) -> io::Result<()> {
        let Some(commands) = &self.commands else {
            return Ok(());
        };
        let owned_legacy = self
            .key
            .read_run_value(&self.name)?
            .is_some_and(|registered| same_text(&registered, &commands.legacy));
        if owned_legacy
            && matches!(
                self.key.read_approval(&self.name)?,
                StartupApproval::None | StartupApproval::Enabled
            )
        {
            self.key.set_run_value(&self.name, &commands.canonical)?;
        }
        Ok(())
    }
}

#[cfg(windows)]
impl PortableStartupRegistration<RegistryRunKey> {
    /// The [`REGISTRATION_NAME`] value of the running executable.
    pub fn for_current_process() -> Self {
        let executable = std::env::current_exe().ok();
        Self::new(RegistryRunKey, REGISTRATION_NAME, executable.as_deref())
    }
}

/// The state of a packaged startup task, as WinRT's `StartupTaskState` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StartupTaskState {
    Disabled,
    DisabledByUser,
    DisabledByPolicy,
    Enabled,
    EnabledByPolicy,
    /// A value newer than this code.
    Unrecognized,
}

impl From<StartupTaskState> for StartupRegistrationState {
    fn from(state: StartupTaskState) -> Self {
        match state {
            StartupTaskState::Disabled => Self::Disabled,
            StartupTaskState::DisabledByUser => Self::DisabledByUser,
            StartupTaskState::DisabledByPolicy => Self::DisabledByPolicy,
            StartupTaskState::Enabled => Self::Enabled,
            StartupTaskState::EnabledByPolicy => Self::EnabledByPolicy,
            StartupTaskState::Unrecognized => Self::Unavailable,
        }
    }
}

/// The startup task the package manifest declares.
pub trait PackagedStartupTask {
    fn state(&self) -> io::Result<StartupTaskState>;

    /// Asks Windows to enable the task and returns the state it settled on, which stays
    /// [`StartupTaskState::DisabledByUser`] or a policy state when Windows refuses.
    fn request_enable(&self) -> io::Result<StartupTaskState>;

    fn disable(&self) -> io::Result<StartupTaskState>;
}

/// The startup task of the MSIX build. Every failure to reach it reads as
/// [`StartupRegistrationState::Unavailable`], as in the .NET `WindowsStartupService`.
pub struct PackagedStartupRegistration<T> {
    task: T,
}

impl<T: PackagedStartupTask> PackagedStartupRegistration<T> {
    pub fn new(task: T) -> Self {
        Self { task }
    }

    pub fn state(&self) -> StartupRegistrationState {
        self.task
            .state()
            .map_or(StartupRegistrationState::Unavailable, Into::into)
    }

    /// Enabling asks Windows only from a plain [`StartupTaskState::Disabled`], and disabling
    /// changes only a plain [`StartupTaskState::Enabled`]: what the user turned off in Windows
    /// Settings and what a policy decides are not WorkTime's to override.
    pub fn set_enabled(&self, enabled: bool) -> StartupRegistrationState {
        self.try_set_enabled(enabled)
            .map_or(StartupRegistrationState::Unavailable, Into::into)
    }

    fn try_set_enabled(&self, enabled: bool) -> io::Result<StartupTaskState> {
        match (enabled, self.task.state()?) {
            (true, StartupTaskState::Disabled) => self.task.request_enable(),
            (false, StartupTaskState::Enabled) => self.task.disable(),
            (_, state) => Ok(state),
        }
    }
}

/// Start with Windows for whichever build is running: the startup task with package identity,
/// the `Run` key without.
pub enum StartupRegistration<K, T> {
    Portable(PortableStartupRegistration<K>),
    Packaged(PackagedStartupRegistration<T>),
}

impl<K: RunKey, T: PackagedStartupTask> StartupRegistration<K, T> {
    pub fn state(&self) -> StartupRegistrationState {
        match self {
            Self::Portable(registration) => registration.state(),
            Self::Packaged(registration) => registration.state(),
        }
    }

    pub fn set_enabled(&self, enabled: bool) -> StartupRegistrationState {
        match self {
            Self::Portable(registration) => registration.set_enabled(enabled),
            Self::Packaged(registration) => registration.set_enabled(enabled),
        }
    }

    /// Only the portable build ever registered a legacy command.
    pub fn migrate_owned_legacy_registration(&self) {
        if let Self::Portable(registration) = self {
            registration.migrate_owned_legacy_registration();
        }
    }
}

#[cfg(windows)]
impl StartupRegistration<RegistryRunKey, WindowsStartupTask> {
    /// The registration of the running executable. With package identity, reading and changing
    /// the state wait for WinRT, so call them off the UI thread.
    pub fn for_current_process() -> Self {
        if crate::has_package_identity() {
            Self::Packaged(PackagedStartupRegistration::new(WindowsStartupTask::new(
                PACKAGED_TASK_ID,
            )))
        } else {
            Self::Portable(PortableStartupRegistration::for_current_process())
        }
    }
}

#[cfg(test)]
mod tests;
