//! The system Bientôt runs on, as « À propos » and the diagnostic export describe it.

/// How this copy of Bientôt was installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InstallKind {
    Store,
    Direct,
    Portable,
    Flatpak,
    AppImage,
    Development,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SessionType {
    X11,
    Wayland,
}

/// What the diagnostic export names of the system, beside its version and architecture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemDescription {
    /// The distribution's `ID` in os-release, such as `ubuntu`.
    pub distribution: String,
    /// Its `VERSION_ID`, such as `24.04`: `None` for a distribution that names no version.
    pub distribution_version: Option<String>,
    /// The desktops `XDG_CURRENT_DESKTOP` names, lower-cased, most specific first:
    /// `["ubuntu", "gnome"]`.
    pub desktops: Vec<String>,
    /// `None` when the environment names no display.
    pub session_type: Option<SessionType>,
}

pub trait SystemInfo {
    fn install_kind(&self) -> InstallKind;

    fn describe(&self) -> SystemDescription;
}

/// A system that gives the answers it was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InMemorySystem {
    pub install_kind: InstallKind,
    pub description: SystemDescription,
}

impl SystemInfo for InMemorySystem {
    fn install_kind(&self) -> InstallKind {
        self.install_kind
    }

    fn describe(&self) -> SystemDescription {
        self.description.clone()
    }
}
