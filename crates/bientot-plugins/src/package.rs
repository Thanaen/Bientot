//! Reading a `.bientot-plugin` file (contract `package.md`): the accepted entry names, the
//! manifest and locale files, and the signature over the canonical file list.

use std::collections::BTreeMap;

use bientot_plugin_api::locale::LocaleError;
use bientot_plugin_api::manifest::is_locale_tag;
use bientot_plugin_api::{Language, LocaleTexts, Manifest, ManifestError};
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::base64;
use crate::zip::{self, ZipError};

pub const MANIFEST: &str = "plugin.json";
pub const MODULE: &str = "plugin.wasm";
pub const SIGNATURE: &str = "SIGNATURE";
pub const MAX_MODULE_BYTES: u64 = 4 << 20;
pub const MAX_TEXT_BYTES: u64 = 64 << 10;
const MAX_SIGNATURE_BYTES: u64 = 16 << 10;
const MAX_ANNOUNCEMENTS: usize = 4;

pub type Hash = [u8; 32];

/// A package that passed every rule of the reader. Its signature, if any, verified.
#[derive(Debug, Clone)]
pub struct Package {
    pub manifest: Manifest,
    /// Every entry except `SIGNATURE`, by path.
    pub files: BTreeMap<String, Vec<u8>>,
    pub locales: BTreeMap<String, LocaleTexts>,
    pub signature: Option<SignatureBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureBlock {
    pub key: VerifyingKey,
    /// `announce` lines: a new key and a signature over it by some older key, checked when the
    /// package is matched against what the id is bound to.
    pub announcements: Vec<(VerifyingKey, Signature)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    Zip(ZipError),
    UnknownEntry(String),
    DuplicateEntry(String),
    MissingManifest,
    Manifest(ManifestError),
    MissingModule,
    MissingLocale(String),
    UndeclaredLocale(String),
    Locale(String, LocaleError),
    BadSignatureFile,
    SignatureMismatch,
}

impl PackageError {
    pub fn message(&self, language: Language) -> String {
        let fr = language == Language::Fr;
        match self {
            PackageError::Zip(error) => zip_message(error, fr),
            PackageError::UnknownEntry(name) if fr => {
                format!("le paquet contient « {name} », qui n’est pas un fichier de plugin")
            }
            PackageError::UnknownEntry(name) => {
                format!("the package holds `{name}`, which is not a plugin file")
            }
            PackageError::DuplicateEntry(name) if fr => {
                format!("le paquet contient deux fois « {name} »")
            }
            PackageError::DuplicateEntry(name) => format!("the package holds `{name}` twice"),
            PackageError::MissingManifest if fr => "le paquet n’a pas de plugin.json".into(),
            PackageError::MissingManifest => "the package has no plugin.json".into(),
            PackageError::Manifest(error) => error.message(language),
            PackageError::MissingModule if fr => "le paquet n’a pas de plugin.wasm".into(),
            PackageError::MissingModule => "the package has no plugin.wasm".into(),
            PackageError::MissingLocale(tag) if fr => {
                format!("la langue « {tag} » est déclarée mais locales/{tag}.json manque")
            }
            PackageError::MissingLocale(tag) => {
                format!("the locale `{tag}` is declared but locales/{tag}.json is missing")
            }
            PackageError::UndeclaredLocale(tag) if fr => {
                format!("locales/{tag}.json n’est pas déclaré dans `locales`")
            }
            PackageError::UndeclaredLocale(tag) => {
                format!("locales/{tag}.json is not declared in `locales`")
            }
            PackageError::Locale(tag, error) => format!("locales/{tag}.json: {error}"),
            PackageError::BadSignatureFile if fr => "le fichier SIGNATURE est illisible".into(),
            PackageError::BadSignatureFile => "the SIGNATURE file cannot be read".into(),
            PackageError::SignatureMismatch if fr => {
                "la signature ne correspond pas aux fichiers du paquet".into()
            }
            PackageError::SignatureMismatch => {
                "the signature does not match the package's files".into()
            }
        }
    }
}

fn zip_message(error: &ZipError, fr: bool) -> String {
    let detail = match error {
        ZipError::TooLarge => {
            return pick(
                fr,
                "le paquet dépasse 8 Mio",
                "the package is larger than 8 MiB",
            );
        }
        ZipError::TooManyEntries => {
            return pick(
                fr,
                "le paquet a plus de 256 fichiers",
                "the package has more than 256 files",
            );
        }
        ZipError::EntryTooLarge(name) | ZipError::Ratio(name) => {
            return if fr {
                format!("« {name} » est trop grand")
            } else {
                format!("`{name}` is too large")
            };
        }
        other => format!("{other:?}"),
    };
    if fr {
        format!("ce n’est pas un paquet de plugin valide ({detail})")
    } else {
        format!("this is not a valid plugin package ({detail})")
    }
}

fn pick(fr: bool, french: &str, english: &str) -> String {
    if fr { french } else { english }.to_owned()
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message(Language::En))
    }
}

impl std::error::Error for PackageError {}

/// The largest size of an entry the package accepts, or `None` for a name it refuses. Only these
/// exact names pass, which also rules out `..`, absolute and UNC paths, drive letters, `\`, reserved
/// Windows names and names that differ only by case.
fn limit(name: &str) -> Option<u64> {
    match name {
        MANIFEST => Some(MAX_TEXT_BYTES),
        MODULE => Some(MAX_MODULE_BYTES),
        SIGNATURE => Some(MAX_SIGNATURE_BYTES),
        _ => {
            if let Some(tag) = name
                .strip_prefix("locales/")
                .and_then(|rest| rest.strip_suffix(".json"))
            {
                return is_locale_tag(tag).then_some(MAX_TEXT_BYTES);
            }
            let asset = name
                .strip_prefix("assets/")
                .and_then(|rest| rest.strip_suffix(".svg"))?;
            let valid = (1..=32).contains(&asset.len())
                && asset
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            valid.then_some(MAX_TEXT_BYTES)
        }
    }
}

impl Package {
    /// Reads and checks a package. `reserved_titles` go to [`Manifest::parse`].
    pub fn read(bytes: &[u8], reserved_titles: &[&str]) -> Result<Package, PackageError> {
        let entries = zip::read(bytes, limit).map_err(PackageError::Zip)?;
        let mut files = BTreeMap::new();
        let mut signature_text = None;
        for (name, data) in entries {
            if limit(&name).is_none() {
                return Err(PackageError::UnknownEntry(name));
            }
            if name == SIGNATURE {
                if signature_text.replace(data).is_some() {
                    return Err(PackageError::DuplicateEntry(name));
                }
            } else if files.insert(name.clone(), data).is_some() {
                return Err(PackageError::DuplicateEntry(name));
            }
        }
        let manifest_bytes = files.get(MANIFEST).ok_or(PackageError::MissingManifest)?;
        let manifest =
            Manifest::parse(manifest_bytes, reserved_titles).map_err(PackageError::Manifest)?;
        if !files.contains_key(MODULE) {
            return Err(PackageError::MissingModule);
        }
        let mut locales = BTreeMap::new();
        for (name, data) in &files {
            let Some(tag) = name
                .strip_prefix("locales/")
                .and_then(|rest| rest.strip_suffix(".json"))
            else {
                continue;
            };
            if !manifest.locales.iter().any(|declared| declared == tag) {
                return Err(PackageError::UndeclaredLocale(tag.to_owned()));
            }
            let texts = LocaleTexts::parse(data)
                .map_err(|error| PackageError::Locale(tag.to_owned(), error))?;
            locales.insert(tag.to_owned(), texts);
        }
        if let Some(missing) = manifest.locales.iter().find(|t| !locales.contains_key(*t)) {
            return Err(PackageError::MissingLocale(missing.clone()));
        }
        let signature = match signature_text {
            None => None,
            Some(text) => {
                let block = parse_signature(&text)?;
                block
                    .0
                    .key
                    .verify_strict(canonical_list(&hashes(&files)).as_bytes(), &block.1)
                    .map_err(|_| PackageError::SignatureMismatch)?;
                Some(block.0)
            }
        };
        Ok(Package {
            manifest,
            files,
            locales,
            signature,
        })
    }

    pub fn hashes(&self) -> BTreeMap<String, Hash> {
        hashes(&self.files)
    }

    /// The SHA-256 of the canonical file list: what an unsigned id is bound to.
    pub fn list_hash(&self) -> Hash {
        Sha256::digest(canonical_list(&self.hashes()).as_bytes()).into()
    }

    pub fn module(&self) -> &[u8] {
        &self.files[MODULE]
    }
}

pub fn hash(bytes: &[u8]) -> Hash {
    Sha256::digest(bytes).into()
}

pub fn hex(hash: &Hash) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

fn hashes(files: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Hash> {
    files
        .iter()
        .map(|(name, data)| (name.clone(), hash(data)))
        .collect()
}

/// The signed message: `<sha256 hex> <path>\n` per entry other than `SIGNATURE`, sorted by path in
/// byte order (a `BTreeMap` of `String` iterates in that order).
pub fn canonical_list(hashes: &BTreeMap<String, Hash>) -> String {
    hashes
        .iter()
        .map(|(path, hash)| format!("{} {path}\n", hex(hash)))
        .collect()
}

/// The fingerprint shown to users: the first 16 bytes of the SHA-256 of the key, in groups of four
/// hex digits.
pub fn fingerprint(key: &VerifyingKey) -> String {
    let digest = hash(key.as_bytes());
    digest[..16]
        .chunks(2)
        .map(|pair| format!("{:02x}{:02x}", pair[0], pair[1]))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn parse_key(text: &str) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = base64::decode(text)?.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

fn parse_sig(text: &str) -> Option<Signature> {
    let bytes: [u8; 64] = base64::decode(text)?.try_into().ok()?;
    Some(Signature::from_bytes(&bytes))
}

fn parse_signature(text: &[u8]) -> Result<(SignatureBlock, Signature), PackageError> {
    let bad = || PackageError::BadSignatureFile;
    let text = std::str::from_utf8(text).map_err(|_| bad())?;
    let mut lines = text.strip_suffix('\n').unwrap_or(text).split('\n');
    if lines.next() != Some("bientot-plugin-signature 1") {
        return Err(bad());
    }
    let key = lines
        .next()
        .and_then(|line| line.strip_prefix("key ed25519 "))
        .and_then(parse_key)
        .ok_or_else(bad)?;
    let signature = lines
        .next()
        .and_then(|line| line.strip_prefix("sig "))
        .and_then(parse_sig)
        .ok_or_else(bad)?;
    let mut announcements = Vec::new();
    for line in lines {
        let mut words = line
            .strip_prefix("announce ed25519 ")
            .ok_or_else(bad)?
            .split(' ');
        let (Some(new), Some(sig), None) = (words.next(), words.next(), words.next()) else {
            return Err(bad());
        };
        announcements.push((
            parse_key(new).ok_or_else(bad)?,
            parse_sig(sig).ok_or_else(bad)?,
        ));
        if announcements.len() > MAX_ANNOUNCEMENTS {
            return Err(bad());
        }
    }
    Ok((SignatureBlock { key, announcements }, signature))
}

/// Writes the `SIGNATURE` text. Used by the SDK's signing tool and by tests.
pub fn signature_text(
    key: &VerifyingKey,
    signature: &Signature,
    announcements: &[(VerifyingKey, Signature)],
) -> String {
    let mut text = format!(
        "bientot-plugin-signature 1\nkey ed25519 {}\nsig {}\n",
        base64::encode(key.as_bytes()),
        base64::encode(&signature.to_bytes())
    );
    for (new, sig) in announcements {
        text.push_str(&format!(
            "announce ed25519 {} {}\n",
            base64::encode(new.as_bytes()),
            base64::encode(&sig.to_bytes())
        ));
    }
    text
}

#[cfg(test)]
pub(crate) mod tests;
