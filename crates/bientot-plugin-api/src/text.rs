//! Languages and texts given in several languages.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The app languages. A plugin may receive a language it does not know from a later host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    En,
    Fr,
    #[serde(other)]
    Unknown,
}

impl Language {
    /// The BCP 47 tag a plugin's locale file is named after.
    pub fn tag(self) -> &'static str {
        match self {
            Language::Fr => "fr",
            Language::En | Language::Unknown => "en",
        }
    }
}

/// A text keyed by BCP 47 tag, for example `{ "en": "Focus", "fr": "Concentration" }`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Localized(pub BTreeMap<String, String>);

impl Localized {
    /// The text for `language`, or for `fallback` (the plugin's first locale).
    pub fn get<'a>(&'a self, language: Language, fallback: &str) -> &'a str {
        self.0
            .get(language.tag())
            .or_else(|| self.0.get(fallback))
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn values(&self) -> impl Iterator<Item = &str> {
        self.0.values().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_language_falls_back_to_the_first_locale() {
        let text = Localized(BTreeMap::from([("en".to_owned(), "Focus".to_owned())]));
        assert_eq!(text.get(Language::Fr, "en"), "Focus");
        assert_eq!(text.get(Language::En, "en"), "Focus");
        let both = Localized(BTreeMap::from([
            ("en".to_owned(), "Focus".to_owned()),
            ("fr".to_owned(), "Concentration".to_owned()),
        ]));
        assert_eq!(both.get(Language::Fr, "en"), "Concentration");
    }

    #[test]
    fn unknown_languages_read_as_unknown() {
        let language: Language = serde_json::from_str("\"de\"").unwrap();
        assert_eq!(language, Language::Unknown);
        assert_eq!(serde_json::to_string(&Language::Fr).unwrap(), "\"fr\"");
    }
}
