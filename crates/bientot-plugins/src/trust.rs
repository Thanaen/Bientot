//! What an installed id is bound to, and whether a new package for it may replace it
//! (contract `package.md`, « Trust on install and update »).

use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

use crate::base64;
use crate::package::{self, Package};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Trust {
    /// Bound to `key`; a package signed by one of `announced` is also accepted.
    Signed {
        key: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        announced: Vec<String>,
    },
    /// Bound to the hash of the canonical file list.
    Unsigned { package_hash: String },
}

impl Trust {
    /// The author fingerprint shown to users, or `None` for an unsigned plugin.
    pub fn fingerprint(&self) -> Option<String> {
        match self {
            Trust::Signed { key, .. } => package::parse_key(key).map(|k| package::fingerprint(&k)),
            Trust::Unsigned { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The id is new: bind it.
    FirstInstall(Trust),
    /// Signed by the bound key or by one it announced.
    Update(Trust),
    /// The id was unsigned: the old plugin and its state go, after the user confirms.
    ReplaceUnsigned(Trust),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Signed by a key the bound one never announced.
    OtherAuthor { bound: String, offered: String },
    /// The id is signed and this package is not.
    NowUnsigned { bound: String },
}

/// Matches `package` against what its id is bound to, if anything.
pub fn decide(bound: Option<&Trust>, package: &Package) -> Result<Decision, Refusal> {
    let offered = binding(package, None);
    match bound {
        None => Ok(Decision::FirstInstall(offered)),
        Some(Trust::Unsigned { .. }) => Ok(Decision::ReplaceUnsigned(offered)),
        Some(Trust::Signed { key, announced }) => {
            let bound_key = package::parse_key(key);
            let bound_print = bound_key
                .map(|k| package::fingerprint(&k))
                .unwrap_or_default();
            let Some(signature) = &package.signature else {
                return Err(Refusal::NowUnsigned { bound: bound_print });
            };
            let new = base64::encode(signature.key.as_bytes());
            let handed_over = bound_key.is_some_and(|old| {
                signature.announcements.iter().any(|(announced_key, sig)| {
                    *announced_key == signature.key
                        && old.verify_strict(announced_key.as_bytes(), sig).is_ok()
                })
            });
            if new == *key || announced.contains(&new) || handed_over {
                let keep = if new == *key {
                    announced.as_slice()
                } else {
                    &[]
                };
                Ok(Decision::Update(binding(package, Some(keep))))
            } else {
                Err(Refusal::OtherAuthor {
                    bound: bound_print,
                    offered: package::fingerprint(&signature.key),
                })
            }
        }
    }
}

/// The binding a package gives its id. A signed package also carries the keys it announces for
/// later, each signed by its own key.
fn binding(package: &Package, keep: Option<&[String]>) -> Trust {
    let Some(signature) = &package.signature else {
        return Trust::Unsigned {
            package_hash: package::hex(&package.list_hash()),
        };
    };
    let own = signature.key;
    let mut announced: Vec<String> = keep.unwrap_or_default().to_vec();
    for (new, sig) in &signature.announcements {
        if *new != own && own.verify_strict(new.as_bytes(), sig).is_ok() {
            let text = base64::encode(new.as_bytes());
            if !announced.contains(&text) {
                announced.push(text);
            }
        }
    }
    Trust::Signed {
        key: base64::encode(own.as_bytes()),
        announced,
    }
}

/// Signs `new` with `old`, for an `announce` line.
pub fn announce(
    old: &ed25519_dalek::SigningKey,
    new: &VerifyingKey,
) -> (VerifyingKey, ed25519_dalek::Signature) {
    use ed25519_dalek::Signer;
    (*new, old.sign(new.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::tests::{build, entries, key, manifest_json};

    fn package(signer: Option<u8>, announce_by: &[(u8, u8)], module: &[u8]) -> Package {
        let signer = signer.map(key);
        let lines: Vec<_> = announce_by
            .iter()
            .map(|&(old, new)| announce(&key(old), &key(new).verifying_key()))
            .collect();
        Package::read(
            &build(&entries(&manifest_json(), module), signer.as_ref(), &lines),
            &[],
        )
        .unwrap()
    }

    fn bound(decision: Decision) -> Trust {
        match decision {
            Decision::FirstInstall(t) | Decision::Update(t) | Decision::ReplaceUnsigned(t) => t,
        }
    }

    #[test]
    fn first_install_binds_the_key_or_the_hash() {
        let signed = decide(None, &package(Some(1), &[], b"a")).unwrap();
        assert!(
            matches!(&signed, Decision::FirstInstall(Trust::Signed { announced, .. }) if announced.is_empty())
        );
        assert!(bound(signed).fingerprint().is_some());
        let unsigned = decide(None, &package(None, &[], b"a")).unwrap();
        let Decision::FirstInstall(Trust::Unsigned { package_hash }) = unsigned else {
            panic!()
        };
        assert_eq!(package_hash.len(), 64);
    }

    #[test]
    fn the_bound_key_updates() {
        let trust = bound(decide(None, &package(Some(1), &[], b"a")).unwrap());
        assert!(matches!(
            decide(Some(&trust), &package(Some(1), &[], b"b")),
            Ok(Decision::Update(_))
        ));
    }

    #[test]
    fn another_key_or_no_signature_is_refused() {
        let trust = bound(decide(None, &package(Some(1), &[], b"a")).unwrap());
        assert!(matches!(
            decide(Some(&trust), &package(Some(2), &[], b"b")),
            Err(Refusal::OtherAuthor { .. })
        ));
        assert!(matches!(
            decide(Some(&trust), &package(None, &[], b"b")),
            Err(Refusal::NowUnsigned { .. })
        ));
        // A self-made announcement by the new key proves nothing.
        assert!(matches!(
            decide(Some(&trust), &package(Some(2), &[(2, 2)], b"b")),
            Err(Refusal::OtherAuthor { .. })
        ));
    }

    #[test]
    fn a_key_announced_ahead_of_time_takes_over() {
        // Version 1, signed by key 1, announces key 2.
        let trust = bound(decide(None, &package(Some(1), &[(1, 2)], b"a")).unwrap());
        let Trust::Signed { announced, .. } = &trust else {
            panic!()
        };
        assert_eq!(announced.len(), 1);
        // Version 2 is signed by key 2, which is now the bound key.
        let next = bound(decide(Some(&trust), &package(Some(2), &[], b"b")).unwrap());
        assert_eq!(
            next.fingerprint(),
            Trust::Signed {
                key: base64::encode(key(2).verifying_key().as_bytes()),
                announced: vec![],
            }
            .fingerprint()
        );
        // Key 1 is no longer accepted.
        assert!(decide(Some(&next), &package(Some(1), &[], b"c")).is_err());
    }

    #[test]
    fn a_handover_in_the_new_package_takes_over() {
        let trust = bound(decide(None, &package(Some(1), &[], b"a")).unwrap());
        assert!(matches!(
            decide(Some(&trust), &package(Some(2), &[(1, 2)], b"b")),
            Ok(Decision::Update(_))
        ));
    }

    #[test]
    fn an_unsigned_id_is_replaced_after_confirmation() {
        let trust = bound(decide(None, &package(None, &[], b"a")).unwrap());
        assert!(matches!(
            decide(Some(&trust), &package(None, &[], b"b")),
            Ok(Decision::ReplaceUnsigned(Trust::Unsigned { .. }))
        ));
        assert!(matches!(
            decide(Some(&trust), &package(Some(1), &[], b"b")),
            Ok(Decision::ReplaceUnsigned(Trust::Signed { .. }))
        ));
    }
}
