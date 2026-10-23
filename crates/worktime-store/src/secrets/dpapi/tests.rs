//! `DpapiSecretStoreTests`. The two cancellation tests have no counterpart: the Rust store is
//! synchronous.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;
use crate::secrets::tests::{
    SECRETS, id, replacement_and_deletion_leave_other_identifiers_intact,
    saved_secret_survives_reopening,
};

struct Directory {
    root: TempDir,
}

impl Directory {
    fn new() -> Self {
        Self {
            root: TempDir::new().unwrap(),
        }
    }

    fn path(&self) -> PathBuf {
        self.root.path().join("secrets")
    }

    fn open(&self) -> DpapiSecretStore {
        DpapiSecretStore::new(self.path()).unwrap()
    }

    fn files(&self) -> Vec<PathBuf> {
        fs::read_dir(self.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }

    fn single_file(&self) -> PathBuf {
        let files = self.files();
        assert_eq!(files.len(), 1, "{files:?}");
        files.into_iter().next().unwrap()
    }
}

#[test]
fn saved_secret_survives_reopening_the_store() {
    for secret in SECRETS {
        let directory = Directory::new();
        saved_secret_survives_reopening(|| directory.open(), secret);

        let ciphertext = fs::read(directory.single_file()).unwrap();
        assert!(!ciphertext.is_empty());
        if !secret.is_empty() {
            let utf16: Vec<u8> = secret.encode_utf16().flat_map(u16::to_le_bytes).collect();
            for clear in [secret.as_bytes(), &utf16] {
                assert!(
                    !ciphertext
                        .windows(clear.len())
                        .any(|window| window == clear),
                    "{secret:?}"
                );
            }
        }
    }
}

#[test]
fn files_carry_the_dotnet_names() {
    let directory = Directory::new();
    directory
        .open()
        .save(&id("activitywatch"), "fictional-private-token")
        .unwrap();

    assert_eq!(
        directory.single_file().file_name().unwrap(),
        "E9E99544FC3CE0B34BE49F4D2664C57ABFBD3F7E40B86D603C14952BCE87A669.secret"
    );
}

#[test]
fn replacement_and_deletion_leave_other_identifiers_intact_on_disk() {
    let directory = Directory::new();
    replacement_and_deletion_leave_other_identifiers_intact(|| directory.open());

    assert_eq!(directory.files().len(), 1);
}

#[test]
fn replacing_a_secret_leaves_one_file() {
    let directory = Directory::new();
    let store = directory.open();
    store.save(&id("first"), "original-secret").unwrap();
    store.save(&id("second"), "independent-secret").unwrap();
    store.save(&id("first"), "replacement-secret").unwrap();

    assert_eq!(directory.files().len(), 2);
}

#[test]
fn corrupted_ciphertext_fails_without_returning_or_disclosing_the_secret() {
    let directory = Directory::new();
    let store = directory.open();
    store
        .save(&id("activitywatch"), "fictional-private-token")
        .unwrap();
    assert_eq!(
        store.get(&id("activitywatch")).unwrap().as_deref(),
        Some("fictional-private-token")
    );
    let path = directory.single_file();
    let ciphertext = fs::read(&path).unwrap();
    let truncated = &ciphertext[..ciphertext.len() / 2];
    fs::write(&path, truncated).unwrap();

    let error = directory.open().get(&id("activitywatch")).unwrap_err();

    assert!(matches!(error, SecretError::Unprotect(_)), "{error:?}");
    assert!(!format!("{error} {error:?}").contains("fictional-private-token"));
    assert_eq!(fs::read(&path).unwrap(), truncated);
}

#[test]
fn ciphertext_copied_to_another_identifier_cannot_be_read() {
    let directory = Directory::new();
    let store = directory.open();
    store.save(&id("first"), "first-secret").unwrap();
    let first = directory.single_file();
    store.save(&id("second"), "second-secret").unwrap();
    let second = directory
        .files()
        .into_iter()
        .find(|path| *path != first)
        .unwrap();
    assert_eq!(
        store.get(&id("second")).unwrap().as_deref(),
        Some("second-secret")
    );
    fs::copy(&first, &second).unwrap();

    let error = store.get(&id("second")).unwrap_err();

    assert!(matches!(error, SecretError::Unprotect(_)), "{error:?}");
    assert_eq!(
        store.get(&id("first")).unwrap().as_deref(),
        Some("first-secret")
    );
}

#[test]
fn path_like_identifier_stays_inside_the_secrets_directory() {
    let identifier = id("../../outside/private-token");
    let directory = Directory::new();
    directory
        .open()
        .save(&identifier, "contained-secret")
        .unwrap();

    assert_eq!(
        directory.open().get(&identifier).unwrap().as_deref(),
        Some("contained-secret")
    );
    let files = all_files(directory.root.path());
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].parent(), Some(directory.path().as_path()));
    let name = files[0].file_name().unwrap().to_str().unwrap();
    assert!(!name.contains("private-token"), "{name}");
}

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(all_files(&path));
        } else {
            files.push(path);
        }
    }
    files
}
