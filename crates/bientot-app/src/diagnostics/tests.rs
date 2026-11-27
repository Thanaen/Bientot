use bientot_platform::lifecycle::LockReporting;
use bientot_platform::system::{InMemorySystem, SystemDescription};

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

fn ubuntu(install_kind: InstallKind) -> InMemorySystem {
    InMemorySystem {
        install_kind,
        description: SystemDescription {
            distribution: "ubuntu".into(),
            distribution_version: Some("24.04".into()),
            desktops: vec!["ubuntu".into(), "gnome".into()],
            session_type: Some(SessionType::Wayland),
        },
    }
}

#[test]
fn the_export_names_the_system_each_backend_in_use_and_unreported_locks() {
    let backends = Backends {
        tray: Some("status-notifier"),
        notifications: Some("notification-portal"),
        autostart: None,
        idle: Some("org.gnome.Mutter.IdleMonitor"),
        activity_undetectable: false,
        lifecycle: Some("logind"),
        lock_reporting: Some(LockReporting::Unavailable),
    };
    let name = |text| DiagnosticName::new(text);
    assert_eq!(
        system_snapshot(&ubuntu(InstallKind::Flatpak), backends),
        DiagnosticSystemSnapshot {
            install_kind: DiagnosticInstallKind::Flatpak,
            distribution: name("ubuntu"),
            distribution_version: name("24.04"),
            desktops: vec![name("ubuntu").unwrap(), name("gnome").unwrap()],
            session_type: DiagnosticSessionType::Wayland,
            backends: vec![
                ("tray", "status-notifier"),
                ("notifications", "notification-portal"),
                ("idle", "org.gnome.Mutter.IdleMonitor"),
                ("lifecycle", "logind"),
            ],
            lock_reporting: Some("unavailable"),
        }
    );
}

#[test]
fn a_name_the_export_may_not_write_is_left_out() {
    let system = InMemorySystem {
        install_kind: InstallKind::Development,
        description: SystemDescription {
            distribution: "Ubuntu on Zoé's laptop".into(),
            distribution_version: Some("24.04 LTS".into()),
            desktops: vec!["/home/zoe".into(), "gnome".into()],
            session_type: None,
        },
    };
    let snapshot = system_snapshot(&system, Backends::default());
    assert_eq!(snapshot.distribution, None);
    assert_eq!(snapshot.distribution_version, None);
    assert_eq!(snapshot.desktops, [DiagnosticName::new("gnome").unwrap()]);
    assert_eq!(snapshot.session_type, DiagnosticSessionType::Unknown);
    assert!(snapshot.backends.is_empty());
}

/// The text of every entry of the ZIP `contents` writes.
#[cfg(all(target_os = "linux", feature = "plugins"))]
fn exported_text(contents: &Contents) -> String {
    let folder = tempfile::tempdir().unwrap();
    let zip = folder.path().join("bientot-diagnostics.zip");
    contents.write(&zip).unwrap();
    let entries = bientot_plugins::zip::read(&std::fs::read(&zip).unwrap(), |_| Some(1 << 22))
        .expect("the export is a ZIP archive");
    entries
        .into_iter()
        .map(|(name, data)| format!("{name}\n{}\n", String::from_utf8(data).unwrap()))
        .collect()
}

/// The user, the computer and its id stand in every place of the environment and of os-release
/// that the export does not read; only the names it reads reach the archive.
#[cfg(all(target_os = "linux", feature = "plugins"))]
#[test]
fn the_export_holds_no_host_name_user_name_home_or_machine_id() {
    use std::ffi::OsString;

    use bientot_linux::system::{describe, install_kind};

    let (user, host, machine_id) = ("zoe-gauthier", "bureau-de-zoe", "5c1e0d2a9b8f4e3d");
    let home = format!("/home/{user}");
    let appimage = format!("{home}/Applications/Bientot.AppImage");
    let environment = [
        ("USER", user),
        ("LOGNAME", user),
        ("HOME", home.as_str()),
        ("HOSTNAME", host),
        ("APPIMAGE", appimage.as_str()),
        ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"),
        ("XDG_SESSION_ID", machine_id),
        ("WAYLAND_DISPLAY", "wayland-0"),
    ];
    let variable = |name: &str| {
        environment
            .iter()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| OsString::from(value))
    };
    let os_release = format!(
        "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu on {host}\"\nID=ubuntu\nVERSION_ID=\"24.04\"\n\
         VERSION=\"24.04 ({user})\"\nBUILD_ID={host}\nIMAGE_ID={machine_id}\nVARIANT=\"{home}\"\n"
    );
    let system = InMemorySystem {
        install_kind: install_kind(variable, false),
        description: describe(variable, Some(&os_release)),
    };
    let backends = Backends {
        tray: Some("status-notifier"),
        notifications: Some("notification-server"),
        autostart: Some("xdg-autostart"),
        idle: Some("MIT-SCREEN-SAVER"),
        activity_undetectable: false,
        lifecycle: Some("logind"),
        lock_reporting: Some(LockReporting::Live),
    };
    let failed = Contents::of_failed_start(None, None);
    let contents = Contents {
        snapshot: DiagnosticSnapshot {
            system: Some(system_snapshot(&system, backends)),
            ..failed.snapshot
        },
        logs: None,
    };

    let exported = exported_text(&contents);

    for personal in [user, "zoe", "gauthier", host, "bureau", machine_id, &home] {
        assert!(!exported.contains(personal), "{personal} in {exported}");
    }
    let system = r#""system":{"installKind":"AppImage","distribution":"ubuntu","distributionVersion":"24.04","desktops":["ubuntu","gnome"],"sessionType":"Wayland","backends":{"tray":"status-notifier","notifications":"notification-server","autostart":"xdg-autostart","idle":"MIT-SCREEN-SAVER","lifecycle":"logind"},"lockReporting":"live"}"#;
    assert!(exported.contains(system), "{exported}");
}

/// The same on the system the test runs on: the export of a failed start reads the real
/// environment and os-release. A value too short to tell apart from the archive's own words, or
/// that is one of the names the export holds, proves nothing and is skipped.
#[cfg(all(target_os = "linux", feature = "plugins"))]
#[test]
fn the_export_of_this_system_holds_none_of_its_personal_names() {
    let contents = Contents::of_failed_start(None, None);
    let system = contents
        .snapshot
        .system
        .clone()
        .expect("Linux describes its system");
    let exported = exported_text(&contents);
    assert!(
        exported.contains("\"system\":{\"installKind\":"),
        "{exported}"
    );

    let read = |path| std::fs::read_to_string(path).unwrap_or_default();
    let variable = |name| std::env::var(name).unwrap_or_default();
    let names: Vec<&str> = system
        .distribution
        .iter()
        .chain(&system.distribution_version)
        .chain(&system.desktops)
        .map(DiagnosticName::as_str)
        .collect();
    let personal = [
        read("/proc/sys/kernel/hostname"),
        read("/etc/machine-id"),
        variable("USER"),
        variable("LOGNAME"),
        variable("HOME"),
    ];
    for value in personal.iter().map(|value| value.trim()) {
        if value.len() < 4 || names.contains(&value) {
            continue;
        }
        assert!(!exported.contains(value), "{value} in {exported}");
    }
}
