//! The system the screenshots are taken for (`--platform`): what its ports answer the scenes,
//! and which scenes show something that differs from Windows. Windows takes every scene; Linux
//! only those of [`LINUX`], which the comparison in CI puts beside their Windows twin.

use std::collections::HashSet;

use bientot_app::strings::Wording;
use bientot_platform::startup::SystemPage;
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
/// words apart for Linux, named beside each. A scene whose state Linux never reaches, as a
/// startup turned off in the system's page of startup apps, stays out.
const LINUX: &[Scene] = &[
    // SettingsStartupCard.Header, SettingsStartupInput.AutomationProperties.Name.
    Scene {
        group: "settings::capture_pages",
        name: "settings-general",
    },
    // SettingsStartupChangeFailed.
    Scene {
        group: "settings::capture_pages",
        name: "settings-general-startup-change-failed",
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
    // OnboardingLanguageIntro.Text, OnboardingLanguageSystem.
    Scene {
        group: "onboarding::capture_steps",
        name: "onboarding-language",
    },
    // SettingsStartupCard.Header, the title of the onboarding's startup row.
    Scene {
        group: "onboarding::capture_steps",
        name: "onboarding-connections",
    },
    // NotificationRegistrationFailed.
    Scene {
        group: "flyout::capture_notices",
        name: "flyout-notifications-unavailable",
    },
    // FlyoutRebuiltReviewText.
    Scene {
        group: "flyout::capture_rebuilt_review",
        name: "flyout-rebuilt-review",
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
