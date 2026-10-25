//! The version resource build.rs compiles into `worktime.exe`, which Windows reads for the app's
//! name in Task Manager and the Startup apps page. CI reads it back from the built exe on Windows.

#[path = "../build/version_resource.rs"]
mod version_resource;

use std::path::Path;

use version_resource::rc;

fn value<'a>(source: &'a str, key: &str) -> &'a str {
    let prefix = format!("VALUE \"{key}\", ");
    source
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("no {key}"))
}

#[test]
fn windows_names_the_exe_bientot_from_its_description_and_product() {
    let source = rc("0.2.0").unwrap();

    assert_eq!(value(&source, "FileDescription"), r#"L"Bient\x00F4t""#);
    assert_eq!(value(&source, "ProductName"), r#"L"Bient\x00F4t""#);
    assert_eq!(value(&source, "CompanyName"), r#"L"Thanaen""#);
    assert_eq!(value(&source, "OriginalFilename"), r#"L"worktime.exe""#);
}

#[test]
fn the_copyright_is_the_license_s() {
    let license =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../LICENSE"))
            .unwrap();
    let source = rc("0.2.0").unwrap();

    assert!(
        license
            .lines()
            .any(|line| line == version_resource::COPYRIGHT)
    );
    assert_eq!(
        value(&source, "LegalCopyright"),
        format!("L\"{}\"", version_resource::COPYRIGHT)
    );
}

#[test]
fn a_release_carries_its_commit_count_as_the_fourth_number() {
    let source = rc("0.2.0-alpha.412").unwrap();

    assert!(source.contains("\nFILEVERSION 0,2,0,412\n"));
    assert!(source.contains("\nPRODUCTVERSION 0,2,0,412\n"));
    assert_eq!(value(&source, "FileVersion"), r#"L"0.2.0-alpha.412""#);
    assert_eq!(value(&source, "ProductVersion"), r#"L"0.2.0-alpha.412""#);
    assert!(rc("0.2.0").unwrap().contains("\nFILEVERSION 0,2,0,0\n"));
}

#[test]
fn a_version_the_resource_cannot_carry_fails_the_build() {
    assert!(rc("0.2").is_err());
    assert!(rc("0.2.0-alpha").is_err());
    assert!(rc("0.2.0-alpha.70000").is_err());
}
