//! The app built for a system shows its texts as that system words them. The unit tests read
//! the base texts on every system, so only a build outside them can tell.

use bientot_app::live::Announcement;
use bientot_app::strings::{Key, Language};

#[test]
fn the_build_reads_the_wording_of_its_system() {
    let expected = match std::env::consts::OS {
        "linux" => "Démarrage automatique",
        _ => "Démarrage Windows",
    };
    assert_eq!(
        Language::French.text(Key::SettingsStartupCardHeader),
        expected
    );
}

/// The flyout's notice when no notification server answered at start, as each system words it.
#[test]
fn the_notifications_unavailable_notice_reads_the_wording_of_its_system() {
    let notice = |language| Announcement::NotificationsRefused.notice(language).message;
    let expected = match std::env::consts::OS {
        "linux" => [
            "Les notifications sont indisponibles sur ce bureau. Le suivi continue.",
            "Notifications are unavailable on this desktop. Workday tracking continues.",
        ],
        _ => [
            "Les notifications Windows sont indisponibles. Le suivi continue.",
            "Windows notifications are unavailable. Workday tracking continues.",
        ],
    };
    assert_eq!(
        [notice(Language::French), notice(Language::English)],
        expected
    );
}
