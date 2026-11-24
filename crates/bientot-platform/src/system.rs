//! The system Bientôt runs on, as the diagnostic export describes it.

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemDescription {
    /// Such as « Windows 11 26100 » or « Ubuntu 24.04.1 LTS ».
    pub os_name: String,
    pub architecture: String,
    /// `XDG_CURRENT_DESKTOP`; `None` on Windows.
    pub desktop: Option<String>,
    pub session_type: Option<SessionType>,
}

pub trait SystemInfo {
    fn install_kind(&self) -> InstallKind;

    fn describe(&self) -> SystemDescription;

    /// The backend in use for each concern, such as `("idle", "ext-idle-notify-v1")`.
    fn backends(&self) -> Vec<(&'static str, String)>;
}

/// A system that gives the answers it was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InMemorySystem {
    pub install_kind: InstallKind,
    pub description: SystemDescription,
    pub backends: Vec<(&'static str, String)>,
}

impl SystemInfo for InMemorySystem {
    fn install_kind(&self) -> InstallKind {
        self.install_kind
    }

    fn describe(&self) -> SystemDescription {
        self.description.clone()
    }

    fn backends(&self) -> Vec<(&'static str, String)> {
        self.backends.clone()
    }
}
