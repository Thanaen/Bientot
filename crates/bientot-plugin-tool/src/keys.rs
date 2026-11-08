//! `keygen` and `sign` (contract `package.md`, « Signature » and « Trust on install and update »).
//!
//! A key file is text: `bientot-plugin-key 1` then `secret <base64 of the 32-byte seed>`. Its
//! public key is written beside it, in `<key file>.pub`, as one line of base64.

use std::path::Path;

use bientot_plugin_api::Language;
use bientot_plugins::base64;
use bientot_plugins::package::{self, Package, SIGNATURE};
use bientot_plugins::trust;
use bientot_plugins::zip;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};

use crate::{Args, read, write};

const HEADER: &str = "bientot-plugin-key 1";

pub fn keygen(args: &Args) -> Result<(), String> {
    let path = args.path(0, "key file")?;
    if path.exists() {
        return Err(format!(
            "{} exists; a new key would orphan the plugins it signed",
            path.display()
        ));
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| format!("no randomness: {e}"))?;
    let key = SigningKey::from_bytes(&seed);
    write_secret(
        &path,
        &format!("{HEADER}\nsecret {}\n", base64::encode(&seed)),
    )?;
    let public = base64::encode(key.verifying_key().as_bytes());
    let public_path = public_path(&path);
    write(&public_path, format!("{public}\n").as_bytes())?;
    println!("key        {}", path.display());
    println!("public key {public} ({})", public_path.display());
    println!("fingerprint {}", package::fingerprint(&key.verifying_key()));
    Ok(())
}

pub fn sign(args: &Args) -> Result<(), String> {
    let path = args.path(0, "package")?;
    let key = load(Path::new(
        args.option("--key").ok_or("sign needs --key <key file>")?,
    ))?;
    let bytes = read(&path)?;
    let package = Package::read(&bytes, &[]).map_err(|e| e.message(Language::En))?;
    let mut announcements = Vec::new();
    if let Some(next) = args.option("--announce") {
        let next = public_key(next)?;
        announcements.push(trust::announce(&key, &next));
    }
    if let Some(old) = args.option("--handover") {
        let old = load(Path::new(old))?;
        announcements.push(trust::announce(&old, &key.verifying_key()));
    }
    let list = package::canonical_list(&package.hashes());
    let signature = key.sign(list.as_bytes());
    let text = package::signature_text(&key.verifying_key(), &signature, &announcements);
    let mut entries: Vec<(&str, &[u8])> = package
        .files
        .iter()
        .map(|(name, data)| (name.as_str(), data.as_slice()))
        .collect();
    entries.push((SIGNATURE, text.as_bytes()));
    let signed = zip::write(&entries);
    Package::read(&signed, &[]).map_err(|e| e.message(Language::En))?;
    write(&path, &signed)?;
    println!(
        "{} signed by {}",
        path.display(),
        package::fingerprint(&key.verifying_key())
    );
    Ok(())
}

fn load(path: &Path) -> Result<SigningKey, String> {
    let text =
        String::from_utf8(read(path)?).map_err(|_| format!("{}: not text", path.display()))?;
    let mut lines = text.lines();
    let seed = (lines.next() == Some(HEADER))
        .then(|| lines.next()?.strip_prefix("secret "))
        .flatten()
        .and_then(base64::decode)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| {
            format!(
                "{} is not a key made by `bientot-plugin keygen`",
                path.display()
            )
        })?;
    Ok(SigningKey::from_bytes(&seed))
}

/// A public key given as base64, or as the path of a `.pub` file.
fn public_key(text: &str) -> Result<VerifyingKey, String> {
    let text = match std::fs::read_to_string(text) {
        Ok(file) => file,
        Err(_) => text.to_owned(),
    };
    package::parse_key(text.trim()).ok_or_else(|| format!("`{}` is not a public key", text.trim()))
}

fn public_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".pub");
    name.into()
}

/// Writes a new key file, and the folder it goes in when that is missing (`~/.bientot/` on a
/// first key), readable by its owner only.
fn write_secret(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write as _;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let mut folder = std::fs::DirBuilder::new();
        folder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            folder.mode(0o700);
        }
        folder
            .create(parent)
            .map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::{MODULE_WAT, folder};
    use bientot_plugins::trust::{Decision, decide};

    fn args(positional: &[&Path], options: &[(&str, &Path)]) -> Args {
        Args {
            positional: positional.iter().map(|p| p.display().to_string()).collect(),
            options: options
                .iter()
                .map(|(name, value)| (name.to_string(), Some(value.display().to_string())))
                .collect(),
        }
    }

    #[test]
    fn a_new_key_signs_and_hands_over_to_the_next() {
        let (dir, bytes, manifest) = folder(r#""read:day""#);
        let module = wat::parse_str(MODULE_WAT).unwrap();
        let package = dir.path().join("p.bientot-plugin");
        std::fs::write(
            &package,
            crate::pack::pack(dir.path(), &module, &bytes, &manifest).unwrap(),
        )
        .unwrap();
        let (old, new) = (dir.path().join("old.key"), dir.path().join("new.key"));
        keygen(&args(&[&old], &[])).unwrap();
        keygen(&args(&[&new], &[])).unwrap();
        assert!(
            keygen(&args(&[&old], &[])).is_err(),
            "a key is never overwritten"
        );

        sign(&args(&[&package], &[("--key", &old)])).unwrap();
        let first = Package::read(&std::fs::read(&package).unwrap(), &[]).unwrap();
        let Ok(Decision::FirstInstall(bound)) = decide(None, &first) else {
            panic!("a signed package installs");
        };
        // Signing again replaces the signature: the new key alone would be another author...
        sign(&args(&[&package], &[("--key", &new)])).unwrap();
        let alone = Package::read(&std::fs::read(&package).unwrap(), &[]).unwrap();
        assert!(decide(Some(&bound), &alone).is_err());
        // ...unless the old key hands over to it.
        sign(&args(&[&package], &[("--key", &new), ("--handover", &old)])).unwrap();
        let handed = Package::read(&std::fs::read(&package).unwrap(), &[]).unwrap();
        assert!(matches!(
            decide(Some(&bound), &handed),
            Ok(Decision::Update(_))
        ));
    }

    #[test]
    fn a_first_key_creates_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".bientot").join("plugin.key");
        keygen(&args(&[&path], &[])).unwrap();
        load(&path).unwrap();
        assert!(public_path(&path).exists());
    }

    #[test]
    fn a_file_that_is_not_a_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k");
        std::fs::write(&path, "secret abc").unwrap();
        assert!(load(&path).unwrap_err().contains("not a key"));
    }
}
