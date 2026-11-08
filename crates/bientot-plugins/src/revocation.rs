//! The revocation list (contract `revocations.md`): the plugins and signing keys Bientôt turns
//! off from a distance, signed with a key kept offline by the maintainer.
//!
//! The list is fetched by the app; this module checks it, keeps the last good one in
//! `plugins/revocations.json` and applies it to `state.json`.

use std::fmt;
use std::io;

use bientot_plugin_api::PluginId;
use ed25519_dalek::{Signature, VerifyingKey};
use jiff::Timestamp;
use serde_json::{Map, Value};

use crate::base64;
use crate::files::{self, PluginsDir};
use crate::state::StateFile;
use crate::trust::Trust;

/// The address the app fetches the list from.
pub const URL: &str = "https://bientot.dev/plugins/revocations.json";
/// A larger response is refused unread.
pub const MAX_BYTES: usize = 256 * 1024;
/// The last good list, beside `state.json`.
pub const FILE: &str = "revocations.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List {
    pub version: u64,
    pub expires: Timestamp,
    /// Stops every WebAssembly plugin until a later list says `false`.
    pub level2_disabled: bool,
    pub revoked: Vec<Revoked>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revoked {
    pub target: Target,
    pub reason_en: String,
    pub reason_fr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Plugin(PluginId),
    /// A signing key's fingerprint, as lower-case hex without separators.
    Key(String),
}

/// Why a list was not taken; the last good one stays (`plugins.revocations.unavailable`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    TooLarge,
    Malformed(String),
    BadSignature,
    Older { version: u64, last: u64 },
    Expired,
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unavailable::TooLarge => write!(f, "larger than {MAX_BYTES} bytes"),
            Unavailable::Malformed(why) => write!(f, "malformed: {why}"),
            Unavailable::BadSignature => write!(f, "bad signature"),
            Unavailable::Older { version, last } => {
                write!(f, "version {version} is older than {last}")
            }
            Unavailable::Expired => write!(f, "expired"),
        }
    }
}

impl List {
    /// Whether `plugin`, bound by `trust`, is revoked, and why.
    pub fn revokes(&self, plugin: &PluginId, trust: &Trust) -> Option<&Revoked> {
        let key = trust.fingerprint().map(|f| hex_only(&f));
        self.revoked.iter().find(|r| match &r.target {
            Target::Plugin(id) => id == plugin,
            Target::Key(fingerprint) => key.as_deref() == Some(fingerprint.as_str()),
        })
    }
}

/// Checks a fetched list: its size, its signature by `key`, its version against the `last` good
/// one, and its expiry at `now`.
pub fn verify(
    bytes: &[u8],
    key: &VerifyingKey,
    last: Option<&List>,
    now: Timestamp,
) -> Result<List, Unavailable> {
    let list = verify_signature(bytes, key)?;
    if let Some(last) = last.filter(|last| list.version < last.version) {
        return Err(Unavailable::Older {
            version: list.version,
            last: last.version,
        });
    }
    if list.expires <= now {
        return Err(Unavailable::Expired);
    }
    Ok(list)
}

/// The last good list, if one was kept and still carries a good signature. Its expiry no
/// longer matters: it stays in force until a newer one replaces it.
pub fn last_good(dir: &PluginsDir, key: &VerifyingKey) -> Option<List> {
    let bytes = std::fs::read(dir.root().join(FILE)).ok()?;
    verify_signature(&bytes, key).ok()
}

/// Takes a fetched list: verifies it against the last good one, keeps it, and applies it to
/// `state`. Returns the plugins whose status changed; `state` is the caller's to save.
pub fn accept(
    dir: &PluginsDir,
    state: &mut StateFile,
    bytes: &[u8],
    key: &VerifyingKey,
    now: Timestamp,
) -> Result<(List, Vec<PluginId>), Unavailable> {
    let last = last_good(dir, key);
    let list = verify(bytes, key, last.as_ref(), now)?;
    // Should it not be kept, the list still applies now and the next fetch brings it again.
    keep(dir, bytes).ok();
    let changed = apply(state, &list);
    Ok((list, changed))
}

fn keep(dir: &PluginsDir, bytes: &[u8]) -> io::Result<()> {
    std::fs::create_dir_all(dir.root())?;
    files::replace(&dir.root().join(FILE), bytes)
}

/// Revokes the plugins `list` names and turns off, never back on, those it no longer names.
/// Returns the plugins whose status changed.
pub fn apply(state: &mut StateFile, list: &List) -> Vec<PluginId> {
    let mut changed = Vec::new();
    for (id, plugin) in &mut state.plugins {
        let did = match list.revokes(id, &plugin.trust) {
            Some(revoked) => plugin.revoke(&revoked.reason_en, &revoked.reason_fr),
            None => plugin.unrevoke(),
        };
        if did {
            changed.push(id.clone());
        }
    }
    changed
}

fn verify_signature(bytes: &[u8], key: &VerifyingKey) -> Result<List, Unavailable> {
    if bytes.len() > MAX_BYTES {
        return Err(Unavailable::TooLarge);
    }
    let malformed = |why: &str| Unavailable::Malformed(why.to_owned());
    let mut object = match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(object)) => object,
        Ok(_) => return Err(malformed("not an object")),
        Err(e) => return Err(Unavailable::Malformed(e.to_string())),
    };
    let signature = match object.remove("signature") {
        Some(Value::String(text)) => text,
        _ => return Err(malformed("no signature")),
    };
    let signature = base64::decode(&signature)
        .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok())
        .map(|bytes| Signature::from_bytes(&bytes))
        .ok_or_else(|| malformed("signature is not 64 bytes of base64"))?;
    let signed = canonical(&Value::Object(object.clone())).map_err(Unavailable::Malformed)?;
    key.verify_strict(signed.as_bytes(), &signature)
        .map_err(|_| Unavailable::BadSignature)?;
    parse(&object).map_err(Unavailable::Malformed)
}

fn parse(object: &Map<String, Value>) -> Result<List, String> {
    let version = object
        .get("version")
        .and_then(Value::as_u64)
        .ok_or("`version` is not a whole number")?;
    let expires = object
        .get("expires")
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<Timestamp>().ok())
        .ok_or("`expires` is not a time")?;
    let level2_disabled = match object.get("level2_disabled") {
        None => false,
        Some(value) => value
            .as_bool()
            .ok_or("`level2_disabled` is not a boolean")?,
    };
    let revoked = match object.get("revoked") {
        None => Vec::new(),
        Some(Value::Array(entries)) => entries
            .iter()
            .enumerate()
            .map(|(at, entry)| revoked(entry).map_err(|why| format!("`revoked[{at}]`: {why}")))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("`revoked` is not a list".to_owned()),
    };
    Ok(List {
        version,
        expires,
        level2_disabled,
        revoked,
    })
}

fn revoked(entry: &Value) -> Result<Revoked, String> {
    let target = match (
        entry.get("id").and_then(Value::as_str),
        entry.get("key").and_then(Value::as_str),
    ) {
        (Some(id), None) => Target::Plugin(PluginId::parse(id).ok_or("`id` is not a plugin id")?),
        (None, Some(key)) => {
            let hex = hex_only(key);
            if hex.len() != 32 {
                return Err("`key` is not a fingerprint".to_owned());
            }
            Target::Key(hex)
        }
        _ => return Err("needs one of `id` and `key`".to_owned()),
    };
    let reason = |language: &str| {
        entry
            .get("reason")
            .and_then(|reason| reason.get(language))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let reason_en = reason("en").ok_or("`reason.en` is missing")?;
    let reason_fr = reason("fr").unwrap_or_else(|| reason_en.clone());
    Ok(Revoked {
        target,
        reason_en,
        reason_fr,
    })
}

/// A fingerprint as lower-case hex, whatever separates its groups.
fn hex_only(fingerprint: &str) -> String {
    fingerprint
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The text the signature covers: keys sorted, no insignificant whitespace, integers only.
pub fn canonical(value: &Value) -> Result<String, String> {
    let mut out = String::new();
    write_canonical(value, &mut out)?;
    Ok(out)
}

fn write_canonical(value: &Value, out: &mut String) -> Result<(), String> {
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => out.push_str(&value.to_string()),
        Value::Number(number) => {
            if !(number.is_u64() || number.is_i64()) {
                return Err(format!("{number} is not an integer"));
            }
            out.push_str(&number.to_string());
        }
        Value::Array(items) => {
            out.push('[');
            for (at, item) in items.iter().enumerate() {
                if at > 0 {
                    out.push(',');
                }
                write_canonical(item, out)?;
            }
            out.push(']');
        }
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            out.push('{');
            for (at, key) in keys.into_iter().enumerate() {
                if at > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(&object[key], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
