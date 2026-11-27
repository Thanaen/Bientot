//! The system the screenshots are taken for (`--platform`): what its ports answer the scenes,
//! and which scenes show something that differs from Windows. Windows takes every scene but those
//! of a state only Linux has, as the AppImage's row « Menu des applications »; Linux only those of
//! [`LINUX`], which the comparison in CI puts beside their Windows twin when there is one.

use std::collections::HashSet;

use bientot_app::strings::Wording;
use bientot_platform::startup::SystemPage;
use bientot_platform::system::InstallKind;
use gpui::Global;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Linux,
}

impl Platform {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "windows" => Some(Self::Windows),
            "linux" => Some(Self::Linux),
            _ => None,
        }
    }

    /// The words of its texts, whichever system renders them.
    pub fn wording(self) -> Wording {
        match self {
            Self::Windows => Wording::Windows,
            Self::Linux => Wording::Linux,
        }
    }

    /// The page of startup apps its `StartupPort` names, which the screenshots install none of.
    pub fn startup_page(self) -> Option<SystemPage> {
        match self {
            Self::Windows => Some(SystemPage {
                uri: "ms-settings:startupapps",
            }),
            Self::Linux => None,
        }
    }

    /// How its `SystemInfo` says the app was installed: an AppImage on Linux, whose « Paramètres »
    /// keep the update setting the screenshots show and add « Menu des applications »; nothing on
    /// Windows.
    pub fn install_kind(self) -> Option<InstallKind> {
        match self {
            Self::Windows => None,
            Self::Linux => Some(InstallKind::AppImage),
        }
    }

    /// Whether it takes a scene of `group`, a name of `SCENES`.
    pub fn takes_group(self, group: &str) -> bool {
        match self {
            Self::Windows => true,
            Self::Linux => LINUX.iter().any(|scene| scene.group == group),
        }
    }

    /// Whether its set holds the screenshot `name`, a file name without `.png`.
    fn takes(self, name: &str) -> bool {
        match self {
            Self::Windows => true,
            Self::Linux => LINUX.iter().any(|scene| scene.name == name),
        }
    }
}

/// A scene Linux takes.
struct Scene {
    /// The group of `SCENES` that takes it.
    group: &'static str,
    name: &'static str,
}

/// The scenes Linux takes: those that show a text `contracts/copy.md` of specs/005-linux-support
/// words apart for Linux, named beside each, and those of a drawing only Linux shows. A scene
/// whose state Linux never reaches, as a startup turned off in the system's page of startup apps,
/// stays out.
const LINUX: &[Scene] = &[
    // SettingsStartupCard.Header, SettingsStartupInput.AutomationProperties.Name; the row of an
    // AppImage above it, LauncherEntry.Header and .Description, its switch on.
    Scene {
        group: "settings::capture_pages",
        name: "settings-general",
    },
    // The AppImage's switch off, then LauncherEntry.ChangeFailed: states only Linux has.
    Scene {
        group: "settings::capture_pages",
        name: "settings-general-launcher-off",
    },
    Scene {
        group: "settings::capture_pages",
        name: "settings-general-launcher-failed",
    },
    // SettingsStartupChangeFailed.
    Scene {
        group: "settings::capture_pages",
        name: "settings-general-startup-change-failed",
    },
    // InstallKind.Header and InstallKind.AppImage, under the version in « À propos ».
    Scene {
        group: "settings::capture_pages",
        name: "settings-general-end",
    },
    // « Diagnostic » under it, which only Linux shows: DiagnosticSection.Header and
    // .Detection with the idle backend's name, .Locks with .LocksNotReported and
    // LockStateUnknown.Text; then .ActivityNone with .ActivityNoneText, and .LocksReported.
    Scene {
        group: "settings::capture_pages",
        name: "settings-general-diagnostic",
    },
    Scene {
        group: "settings::capture_pages",
        name: "settings-general-diagnostic-undetectable",
    },
    // StartupMismatchAppImage.Text and StartupMismatch.Action: a state only an AppImage has.
    Scene {
        group: "settings::capture_pages",
        name: "settings-general-startup-moved",
    },
    // SettingsStartupCard.Header, in the two other states Linux reaches.
    Scene {
        group: "settings::capture_pages",
        name: "settings-general-startup-mismatch",
    },
    Scene {
        group: "settings::capture_pages",
        name: "settings-general-startup-unavailable",
    },
    // OnboardingLanguageIntro.Text, and the system's language: OnboardingLanguageSystemFrench or
    // OnboardingLanguageSystemEnglish.
    Scene {
        group: "onboarding::capture_steps",
        name: "onboarding-language",
    },
    // SettingsStartupCard.Header, the title of the onboarding's startup row.
    Scene {
        group: "onboarding::capture_steps",
        name: "onboarding-connections",
    },
    // StartupMismatchAppImage.Text as the row's status, its switch off: a state only an AppImage
    // has.
    Scene {
        group: "onboarding::capture_steps",
        name: "onboarding-connections-startup-moved",
    },
    // NotificationRegistrationFailed.
    Scene {
        group: "flyout::capture_notices",
        name: "flyout-notifications-unavailable",
    },
    // ActivityUndetectable.Title, .Text and .Action: a state only Linux has.
    Scene {
        group: "flyout::capture_notices",
        name: "flyout-activity-undetectable",
    },
    // ActivityUndetectable.ForecastPaused.
    Scene {
        group: "flyout::capture_notices",
        name: "flyout-activity-undetectable-paused",
    },
    // WindowModeNotice.Title, .Text, .Link and .Dismiss: window mode, a state only Linux has;
    // then the notice above the day with the alerts below it.
    Scene {
        group: "flyout::capture_notices",
        name: "flyout-window-mode",
    },
    Scene {
        group: "flyout::capture_notices",
        name: "flyout-window-mode-alerts",
    },
    // No words: the frame the app draws where no system frames the flyout's window, rounded
    // with its shadow where a compositor blends it, square without one; Windows draws its own.
    Scene {
        group: "flyout::capture_notices",
        name: "flyout-frame-drawn",
    },
    Scene {
        group: "flyout::capture_notices",
        name: "flyout-frame-square",
    },
    // FlyoutRebuiltReviewText.
    Scene {
        group: "flyout::capture_rebuilt_review",
        name: "flyout-rebuilt-review",
    },
    // No words: the tray icon for a panel whose colour Linux does not tell, which Windows never
    // draws, its panels captioned with their colour and the sizes in pixels, alike in every
    // language.
    Scene {
        group: "tray::capture_icons",
        name: "tray-icons-any-panel",
    },
];

/// The screenshots of one run: its platform, and the scenes taken so far.
pub struct Shots {
    pub platform: Platform,
    taken: HashSet<String>,
}

impl Global for Shots {}

impl Shots {
    pub fn new(platform: Platform) -> Self {
        Self {
            platform,
            taken: HashSet::new(),
        }
    }

    /// Whether to take the screenshot `name`, then counted as taken.
    pub fn take(&mut self, name: &str) -> bool {
        let takes = self.platform.takes(name);
        if takes {
            self.taken.insert(name.to_owned());
        }
        takes
    }

    /// The scenes of Linux that no capture took: renamed or gone since [`LINUX`] named them.
    pub fn missed(&self) -> Vec<&'static str> {
        match self.platform {
            Platform::Windows => Vec::new(),
            Platform::Linux => LINUX
                .iter()
                .map(|scene| scene.name)
                .filter(|name| !self.taken.contains(*name))
                .collect(),
        }
    }
}
