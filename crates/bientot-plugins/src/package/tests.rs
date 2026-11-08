use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};

use super::*;

pub(crate) fn manifest_json() -> Value {
    json!({
        "id": "bientot.focus",
        "name": { "en": "Focus", "fr": "Concentration" },
        "version": "0.1.0",
        "api": "1.0",
        "locales": ["en", "fr"],
        "capabilities": ["read:day", "ui:flyout"],
        "events": ["day.refreshed"]
    })
}

pub(crate) fn entries(manifest: &Value, module: &[u8]) -> Vec<(String, Vec<u8>)> {
    vec![
        ("plugin.json".into(), manifest.to_string().into_bytes()),
        ("plugin.wasm".into(), module.to_vec()),
        ("locales/en.json".into(), br#"{"title":"Focus"}"#.to_vec()),
        (
            "locales/fr.json".into(),
            br#"{"title":"Concentration"}"#.to_vec(),
        ),
    ]
}

pub(crate) fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

/// Zips `entries`, signed by `signer` when given, with `announcements` added to `SIGNATURE`.
pub(crate) fn build(
    entries: &[(String, Vec<u8>)],
    signer: Option<&SigningKey>,
    announcements: &[(VerifyingKey, Signature)],
) -> Vec<u8> {
    let mut all: Vec<(String, Vec<u8>)> = entries.to_vec();
    if let Some(signer) = signer {
        let files: BTreeMap<String, Vec<u8>> = entries.iter().cloned().collect();
        let list = canonical_list(&hashes(&files));
        let text = signature_text(
            &signer.verifying_key(),
            &signer.sign(list.as_bytes()),
            announcements,
        );
        all.push((SIGNATURE.into(), text.into_bytes()));
    }
    let refs: Vec<(&str, &[u8])> = all
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    zip::write(&refs)
}

fn read(bytes: &[u8]) -> Result<Package, PackageError> {
    Package::read(bytes, &["Open settings"])
}

#[test]
fn a_valid_package_reads() {
    let package = read(&build(&entries(&manifest_json(), b"\0asm"), None, &[])).unwrap();
    assert_eq!(package.manifest.id.as_str(), "bientot.focus");
    assert_eq!(package.locales["fr"].get("title"), Some("Concentration"));
    assert_eq!(package.module(), b"\0asm");
    assert!(package.signature.is_none());
    assert_eq!(package.hashes().len(), 4);
}

#[test]
fn only_known_names_pass() {
    for bad in [
        "../plugin.json",
        "Plugin.json",
        "C:/x",
        "locales/EN.json",
        "locales/en.txt",
        "assets/Logo.svg",
        "assets/a/b.svg",
        "assets/.svg",
        "readme.md",
        "CON",
        "assets/x.svg ",
        "locales/en.json.",
    ] {
        let mut files = entries(&manifest_json(), b"\0asm");
        files.push((bad.into(), b"x".to_vec()));
        assert_eq!(
            read(&build(&files, None, &[])).unwrap_err(),
            PackageError::UnknownEntry(bad.into()),
            "{bad}"
        );
    }
    let mut files = entries(&manifest_json(), b"\0asm");
    files.push(("assets/logo-2.svg".into(), b"<svg/>".to_vec()));
    assert!(read(&build(&files, None, &[])).is_ok());
}

#[test]
fn duplicates_and_missing_files_are_refused() {
    let mut files = entries(&manifest_json(), b"\0asm");
    files.push(files[1].clone());
    assert_eq!(
        read(&build(&files, None, &[])).unwrap_err(),
        PackageError::DuplicateEntry("plugin.wasm".into())
    );
    let without = |name: &str| {
        let files: Vec<_> = entries(&manifest_json(), b"\0asm")
            .into_iter()
            .filter(|(n, _)| n != name)
            .collect();
        read(&build(&files, None, &[])).unwrap_err()
    };
    assert_eq!(without("plugin.json"), PackageError::MissingManifest);
    assert_eq!(without("plugin.wasm"), PackageError::MissingModule);
    assert_eq!(
        without("locales/fr.json"),
        PackageError::MissingLocale("fr".into())
    );
    let mut extra = entries(&manifest_json(), b"\0asm");
    extra.push(("locales/de.json".into(), b"{}".to_vec()));
    assert_eq!(
        read(&build(&extra, None, &[])).unwrap_err(),
        PackageError::UndeclaredLocale("de".into())
    );
}

#[test]
fn bad_manifests_and_locales_are_named() {
    let mut manifest = manifest_json();
    manifest["colour"] = json!("red");
    let error = read(&build(&entries(&manifest, b"\0asm"), None, &[])).unwrap_err();
    assert!(matches!(&error, PackageError::Manifest(e) if e.field == "colour"));
    assert!(error.message(Language::Fr).contains("colour"));

    let mut files = entries(&manifest_json(), b"\0asm");
    files[3].1 = br#"{"title": 3}"#.to_vec();
    assert!(matches!(
        read(&build(&files, None, &[])).unwrap_err(),
        PackageError::Locale(tag, LocaleError::NotFlatObject) if tag == "fr"
    ));
}

#[test]
fn caps_hold() {
    let big = vec![0u8; MAX_MODULE_BYTES as usize + 1];
    assert!(matches!(
        read(&build(&entries(&manifest_json(), &big), None, &[])).unwrap_err(),
        PackageError::Zip(ZipError::EntryTooLarge(_) | ZipError::Ratio(_) | ZipError::TooLarge)
    ));
    let mut files = entries(&manifest_json(), b"\0asm");
    files.push((
        "assets/big.svg".into(),
        vec![b'a'; MAX_TEXT_BYTES as usize + 1],
    ));
    assert!(read(&build(&files, None, &[])).is_err());
}

#[test]
fn signatures_verify_over_the_file_list() {
    let signer = key(1);
    let files = entries(&manifest_json(), b"\0asm");
    let package = read(&build(&files, Some(&signer), &[])).unwrap();
    assert_eq!(package.signature.unwrap().key, signer.verifying_key());

    // A signature made for other contents.
    let other = entries(&manifest_json(), b"\0asm2");
    let list = canonical_list(&hashes(&other.iter().cloned().collect()));
    let text = signature_text(&signer.verifying_key(), &signer.sign(list.as_bytes()), &[]);
    let mut forged = files.clone();
    forged.push((SIGNATURE.into(), text.into_bytes()));
    let refs: Vec<(&str, &[u8])> = forged
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    assert_eq!(
        read(&zip::write(&refs)).unwrap_err(),
        PackageError::SignatureMismatch
    );

    for text in [
        "",
        "bientot-plugin-signature 2\n",
        "bientot-plugin-signature 1\nkey ed25519 AAAA\nsig AAAA\n",
        "bientot-plugin-signature 1\nkey rsa x\n",
    ] {
        let mut files = files.clone();
        files.push((SIGNATURE.into(), text.as_bytes().to_vec()));
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        assert_eq!(
            read(&zip::write(&refs)).unwrap_err(),
            PackageError::BadSignatureFile,
            "{text:?}"
        );
    }
}

#[test]
fn announcements_parse() {
    let (old, new) = (key(1), key(2));
    let announcement = (
        new.verifying_key(),
        old.sign(new.verifying_key().as_bytes()),
    );
    let package = read(&build(
        &entries(&manifest_json(), b"\0asm"),
        Some(&new),
        &[announcement],
    ))
    .unwrap();
    assert_eq!(package.signature.unwrap().announcements, vec![announcement]);
}

#[test]
fn fingerprints_are_grouped() {
    let print = fingerprint(&key(1).verifying_key());
    assert_eq!(print.len(), 8 * 4 + 7);
    assert!(print.split(' ').all(|g| g.len() == 4));
}
