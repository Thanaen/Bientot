//! Plugin ids. An id becomes a folder name, a settings key and a log file name, so only a narrow
//! form is accepted: `^[a-z0-9]+(\.[a-z0-9-]+){1,4}$`, at most 64 characters.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PluginId(String);

/// Ids under `dev.` are kept for the developer mode of a later step.
pub const RESERVED_PREFIX: &str = "dev.";

impl PluginId {
    pub fn parse(text: &str) -> Option<PluginId> {
        if text.len() > 64 {
            return None;
        }
        let mut parts = text.split('.');
        let first = parts.next()?;
        if first.is_empty()
            || !first
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            return None;
        }
        let mut rest = 0;
        for part in parts {
            let valid = !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !valid {
                return None;
            }
            rest += 1;
        }
        (1..=4).contains(&rest).then(|| PluginId(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_reserved(&self) -> bool {
        self.0.starts_with(RESERVED_PREFIX)
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for PluginId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PluginId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        PluginId::parse(&text).ok_or_else(|| serde::de::Error::custom("invalid plugin id"))
    }
}

#[cfg(test)]
mod tests {
    use super::PluginId;

    #[test]
    fn ids_follow_the_contract() {
        for good in [
            "dev.bientot.focus",
            "com.example.pomodoro",
            "fr.jane-doe.cafe",
            "a.b",
            "a1.b-2.c.d.e",
        ] {
            assert!(PluginId::parse(good).is_some(), "{good}");
        }
        let long = format!("a.{}", "b".repeat(63));
        for bad in [
            "",
            "focus",
            "Com.example.x",
            "com..x",
            "com.example.",
            ".com.x",
            "-a.b",
            "a.b.c.d.e.f",
            "a.b_c",
            "a.b/c",
            "a.b\\c",
            "a.b c",
            "a.é",
            long.as_str(),
        ] {
            assert!(PluginId::parse(bad).is_none(), "{bad}");
        }
        assert!(PluginId::parse("dev.bientot.focus").unwrap().is_reserved());
        assert!(!PluginId::parse("com.example.x").unwrap().is_reserved());
    }
}
