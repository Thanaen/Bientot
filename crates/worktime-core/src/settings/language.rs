//! The language WorkTime starts in, ported from `Services/SettingsLanguageResolver.cs`.

pub const FRENCH: &str = "fr-FR";
pub const ENGLISH: &str = "en-US";

/// `--english`, else the language saved in the settings, else the first of the user's
/// `preferred` languages that is French or English, else English.
pub fn resolve<'a>(
    force_english: bool,
    persisted: Option<&str>,
    preferred: impl IntoIterator<Item = &'a str>,
) -> &'static str {
    if force_english {
        return ENGLISH;
    }
    match persisted {
        Some(FRENCH) => return FRENCH,
        Some(ENGLISH) => return ENGLISH,
        _ => {}
    }
    let starts_with = |language: &str, prefix: &str| {
        language
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
    };
    preferred
        .into_iter()
        .find_map(|language| {
            if starts_with(language, "fr") {
                Some(FRENCH)
            } else if starts_with(language, "en") {
                Some(ENGLISH)
            } else {
                None
            }
        })
        .unwrap_or(ENGLISH)
}
