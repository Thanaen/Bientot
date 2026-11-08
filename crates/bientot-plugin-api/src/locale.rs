//! A plugin's locale file, `locales/<tag>.json`: a flat object of keys to texts. Cards name texts
//! by key, so that the host can redraw a card in another language without asking the plugin.

use std::collections::BTreeMap;

use serde_json::Value;

pub const MAX_LOCALE_BYTES: usize = 64 * 1024;
pub const MAX_KEY_LEN: usize = 64;
pub const MAX_TEXT_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocaleTexts(BTreeMap<String, String>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocaleError {
    TooLarge,
    NotJson,
    NotFlatObject,
    BadKey(String),
    TextTooLong(String),
}

impl std::fmt::Display for LocaleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocaleError::TooLarge => write!(f, "the locale file is larger than 64 KiB"),
            LocaleError::NotJson => write!(f, "the locale file is not valid JSON"),
            LocaleError::NotFlatObject => write!(f, "the locale file must be an object of strings"),
            LocaleError::BadKey(key) => write!(
                f,
                "`{key}` is not a valid key: lower-case letters, digits, `.`, `_` and `-`, at most 64"
            ),
            LocaleError::TextTooLong(key) => write!(f, "`{key}` is longer than 200 characters"),
        }
    }
}

impl std::error::Error for LocaleError {}

impl LocaleTexts {
    pub fn parse(bytes: &[u8]) -> Result<LocaleTexts, LocaleError> {
        if bytes.len() > MAX_LOCALE_BYTES {
            return Err(LocaleError::TooLarge);
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|_| LocaleError::NotJson)?;
        let Value::Object(object) = value else {
            return Err(LocaleError::NotFlatObject);
        };
        let mut texts = BTreeMap::new();
        for (key, value) in object {
            if !is_key(&key) {
                return Err(LocaleError::BadKey(key));
            }
            let Value::String(text) = value else {
                return Err(LocaleError::NotFlatObject);
            };
            if text.chars().count() > MAX_TEXT_CHARS {
                return Err(LocaleError::TextTooLong(key));
            }
            texts.insert(key, text);
        }
        Ok(LocaleTexts(texts))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> LocaleTexts {
        LocaleTexts(
            pairs
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        )
    }
}

fn is_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY_LEN
        && key.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}

/// The texts of one plugin for the app language, with its first locale behind.
#[derive(Debug, Clone, Copy)]
pub struct TextLookup<'a> {
    pub preferred: Option<&'a LocaleTexts>,
    pub fallback: &'a LocaleTexts,
}

impl<'a> TextLookup<'a> {
    pub fn get(&self, key: &str) -> Option<&'a str> {
        self.preferred
            .and_then(|texts| texts.get(key))
            .or_else(|| self.fallback.get(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_object_reads() {
        let texts = LocaleTexts::parse(
            br#"{ "longest": "Longest focus", "since-break": "Since your break" }"#,
        )
        .unwrap();
        assert_eq!(texts.get("longest"), Some("Longest focus"));
        assert_eq!(texts.get("missing"), None);
    }

    #[test]
    fn bad_files_are_refused() {
        assert_eq!(LocaleTexts::parse(b"[]"), Err(LocaleError::NotFlatObject));
        assert_eq!(
            LocaleTexts::parse(br#"{ "a": { "b": "c" } }"#),
            Err(LocaleError::NotFlatObject)
        );
        assert_eq!(
            LocaleTexts::parse(br#"{ "A": "x" }"#),
            Err(LocaleError::BadKey("A".into()))
        );
        assert_eq!(LocaleTexts::parse(b"{"), Err(LocaleError::NotJson));
        let long = format!(r#"{{ "a": "{}" }}"#, "x".repeat(201));
        assert_eq!(
            LocaleTexts::parse(long.as_bytes()),
            Err(LocaleError::TextTooLong("a".into()))
        );
        assert_eq!(
            LocaleTexts::parse(&vec![b' '; MAX_LOCALE_BYTES + 1]),
            Err(LocaleError::TooLarge)
        );
    }

    #[test]
    fn lookup_falls_back_to_the_first_locale() {
        let en = LocaleTexts::from_pairs([("a", "A"), ("b", "B")]);
        let fr = LocaleTexts::from_pairs([("a", "À")]);
        let lookup = TextLookup {
            preferred: Some(&fr),
            fallback: &en,
        };
        assert_eq!(lookup.get("a"), Some("À"));
        assert_eq!(lookup.get("b"), Some("B"));
        assert_eq!(lookup.get("c"), None);
    }
}
