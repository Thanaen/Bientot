use std::path::PathBuf;

use super::*;

fn folder(settings: Option<&str>, staged: bool) -> (tempfile::TempDir, DataDirectory) {
    let root = tempfile::tempdir().unwrap();
    let directory = DataDirectory::from(PathBuf::from(root.path()));
    if let Some(settings) = settings {
        std::fs::write(directory.settings_file(), settings).unwrap();
    }
    if staged {
        std::fs::write(directory.pending_import_file(), b"staged").unwrap();
    }
    (root, directory)
}

#[test]
fn a_restart_that_left_its_import_staged_tells_in_the_language_of_the_settings() {
    let (_root, directory) = folder(Some(r#"{"languageOverride":"en-US"}"#), true);

    assert_eq!(
        import_left_waiting(Some(4242), &directory, None),
        Some(Language::English)
    );
    assert_eq!(
        import_left_waiting(Some(4242), &directory, Some(Language::French)),
        Some(Language::French)
    );
}

#[test]
fn only_a_restart_with_a_staged_import_has_something_to_tell() {
    let (_root, staged) = folder(None, true);
    let (_other, nothing) = folder(None, false);

    assert_eq!(import_left_waiting(None, &staged, None), None);
    assert_eq!(import_left_waiting(Some(4242), &nothing, None), None);
}
