use std::ffi::OsString;

use super::messages_language;

/// The language of the messages in an environment of these variables only.
fn language_of(environment: &[(&str, &str)]) -> Option<String> {
    messages_language(|name| {
        environment
            .iter()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| OsString::from(value))
    })
}

#[test]
fn reads_the_language_and_territory_of_a_locale_name() {
    assert_eq!(
        language_of(&[("LANG", "fr_FR.UTF-8@euro")]).as_deref(),
        Some("fr-FR")
    );
    assert_eq!(
        language_of(&[("LANG", "en_GB.utf8")]).as_deref(),
        Some("en-GB")
    );
    assert_eq!(language_of(&[("LANG", "fr")]).as_deref(), Some("fr"));
    assert_eq!(
        language_of(&[("LANG", "de_DE@euro")]).as_deref(),
        Some("de-DE")
    );
    assert_eq!(
        language_of(&[("LANG", "es_419.UTF-8")]).as_deref(),
        Some("es-419")
    );
}

#[test]
fn lc_all_beats_lc_messages_which_beats_lang() {
    let everything = [
        ("LANG", "de_DE.UTF-8"),
        ("LC_MESSAGES", "en_US.UTF-8"),
        ("LC_ALL", "fr_CA.UTF-8"),
    ];
    assert_eq!(language_of(&everything).as_deref(), Some("fr-CA"));
    assert_eq!(language_of(&everything[..2]).as_deref(), Some("en-US"));
    assert_eq!(language_of(&everything[..1]).as_deref(), Some("de-DE"));
}

#[test]
fn an_empty_variable_is_not_set() {
    assert_eq!(
        language_of(&[("LC_ALL", ""), ("LC_MESSAGES", ""), ("LANG", "fr_FR.UTF-8")]).as_deref(),
        Some("fr-FR")
    );
    assert_eq!(language_of(&[("LANG", "")]), None);
    assert_eq!(language_of(&[]), None);
}

#[test]
fn the_c_and_posix_locales_name_no_language_and_hide_the_later_variables() {
    for locale in ["C", "C.UTF-8", "POSIX"] {
        assert_eq!(
            language_of(&[("LC_ALL", locale), ("LANG", "fr_FR.UTF-8")]),
            None,
            "{locale}"
        );
    }
    assert_eq!(
        language_of(&[("LC_MESSAGES", "C"), ("LANG", "fr_FR.UTF-8")]),
        None
    );
}

#[test]
fn a_name_that_is_no_locale_names_no_language() {
    for locale in ["/usr/lib/locale/fr_FR", "français", "f_FR", "fr-FR-x"] {
        assert_eq!(language_of(&[("LANG", locale)]), None, "{locale}");
    }
}
