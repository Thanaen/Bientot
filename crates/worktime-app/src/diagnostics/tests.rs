use super::*;
use crate::widgets::NoticeKind;

#[test]
fn a_saved_zip_is_announced_as_a_success() {
    assert_eq!(
        Outcome::Saved.notice(Language::French),
        Some(Notice {
            message: "Le ZIP de diagnostic a été enregistré.".to_owned(),
            kind: NoticeKind::Success,
            undo: None,
        })
    );
    assert_eq!(
        Outcome::Saved.notice(Language::English),
        Some(Notice {
            message: "The diagnostic ZIP was saved.".to_owned(),
            kind: NoticeKind::Success,
            undo: None,
        })
    );
}

#[test]
fn a_failed_export_asks_for_another_location() {
    assert_eq!(
        Outcome::Failed.notice(Language::French),
        Some(Notice {
            message: "Bientôt n’a pas pu enregistrer le ZIP de diagnostic. Choisissez un autre \
                      emplacement et réessayez."
                .to_owned(),
            kind: NoticeKind::Error,
            undo: None,
        })
    );
    assert_eq!(
        Outcome::Failed.notice(Language::English),
        Some(Notice {
            message: "Bientôt could not save the diagnostic ZIP. Choose another location and \
                      try again."
                .to_owned(),
            kind: NoticeKind::Error,
            undo: None,
        })
    );
}

#[test]
fn a_closed_dialog_shows_nothing() {
    assert_eq!(Outcome::Cancelled.notice(Language::French), None);
    assert_eq!(Outcome::Cancelled.notice(Language::English), None);
}

#[test]
fn the_dialog_offers_a_zip_archive() {
    assert_eq!(SUGGESTED_FILE_NAME, "bientot-diagnostics.zip");
    assert_eq!(
        Language::French.text(Key::DiagnosticZipFileType),
        "Archive ZIP"
    );
    assert_eq!(
        Language::English.text(Key::DiagnosticZipFileType),
        "ZIP archive"
    );
}

#[test]
fn the_application_version_has_four_parts_as_in_dotnet() {
    let version = |major, minor, build, revision| DiagnosticNumericVersion {
        major,
        minor,
        build,
        revision,
    };
    assert_eq!(application_version("1.4.2"), version(1, 4, 2, 0));
    assert_eq!(application_version("0.1.0-beta.3"), version(0, 1, 0, 0));
    assert_eq!(application_version("2.0.1.7"), version(2, 0, 1, 7));
}

#[test]
fn architectures_are_named_as_in_dotnet() {
    assert_eq!(architecture("x86_64"), DiagnosticArchitecture::X64);
    assert_eq!(architecture("aarch64"), DiagnosticArchitecture::Arm64);
    assert_eq!(architecture("riscv64"), DiagnosticArchitecture::Unknown);
    assert_eq!(machine_architecture(0x8664), DiagnosticArchitecture::X64);
    assert_eq!(machine_architecture(0xaa64), DiagnosticArchitecture::Arm64);
    assert_eq!(machine_architecture(0x014c), DiagnosticArchitecture::X86);
    assert_eq!(machine_architecture(0), DiagnosticArchitecture::Unknown);
}
