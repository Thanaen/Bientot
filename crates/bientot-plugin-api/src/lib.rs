//! The types Bientôt and its plugins exchange, in api 1.0: the manifest, the events, the day
//! snapshot and the view tree. The host and the SDK both build on this crate, so a type changes
//! for both at once. Contracts: `specs/003-plugin-runtime/contracts/`.
//!
//! Every enum a plugin receives has an `Unknown` variant and no struct refuses unknown fields,
//! so that a host adding to api 1.x never breaks a plugin built for 1.0.

#![forbid(unsafe_code)]

pub mod event;
pub mod id;
pub mod locale;
pub mod manifest;
pub mod snapshot;
pub mod text;
pub mod view;

pub use event::{ActivateReason, CommandSource, Event};
pub use id::PluginId;
pub use locale::LocaleTexts;
pub use manifest::{Capability, CommandDecl, EventKind, Manifest, ManifestError, Window};
pub use snapshot::DaySnapshot;
pub use text::{Language, Localized};
pub use view::{Card, Node, PreparedCard, ViewError};

/// The api this host implements. A manifest asks for `1.x` with x at most this minor.
pub const API_MAJOR: u32 = 1;
pub const API_MINOR: u32 = 0;

/// The value `bt_abi` returns: the envelope of the exports and imports (contract `host-abi.md`).
pub const ABI_VERSION: i32 = 1;

/// A version `major.minor.patch`, as plugins and the api declare it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// Reads `1.2.3`, or `1.2` and `1` with the missing parts at zero. Leading zeros, signs,
    /// spaces, pre-release and build suffixes are refused.
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = [0u32; 3];
        for (index, part) in text.split('.').enumerate() {
            if index == 3 || part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            if part.len() > 1 && part.starts_with('0') {
                return None;
            }
            parts[index] = part.parse().ok()?;
        }
        Some(Version {
            major: parts[0],
            minor: parts[1],
            patch: parts[2],
        })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::Version;

    #[test]
    fn versions_parse_strictly() {
        assert_eq!(
            Version::parse("1.2.3"),
            Some(Version {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
        assert_eq!(Version::parse("1.0"), Version::parse("1.0.0"));
        assert_eq!(Version::parse("2"), Version::parse("2.0.0"));
        for bad in [
            "",
            "1.",
            ".1",
            "1.2.3.4",
            "01.2",
            "1.-2",
            " 1.2",
            "1.2.3-beta",
            "a.b",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert_eq!(Version::parse("0.10.2").unwrap().to_string(), "0.10.2");
    }
}
