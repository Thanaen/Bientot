//! The app built for a system shows its texts as that system words them. The unit tests read
//! the base texts on every system, so only a build outside them can tell.

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
