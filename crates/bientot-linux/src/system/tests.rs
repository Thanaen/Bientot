use std::ffi::OsString;

use bientot_platform::system::{InstallKind, SessionType, SystemDescription};

use super::{describe, install_kind, os_release_paths, session_type};

/// Reads these variables only.
fn environment<'a>(variables: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name| {
        variables
            .iter()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| OsString::from(value))
    }
}

const UBUNTU: &str = r#"PRETTY_NAME="Ubuntu 24.04.1 LTS"
NAME="Ubuntu"
VERSION_ID="24.04"
VERSION="24.04.1 LTS (Noble Numbat)"
VERSION_CODENAME=noble
ID=ubuntu
ID_LIKE=debian
HOME_URL="https://www.ubuntu.com/"
"#;

#[test]
fn names_the_distribution_its_version_the_desktops_and_the_session() {
    let variables = [
        ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"),
        ("WAYLAND_DISPLAY", "wayland-0"),
        ("DISPLAY", ":0"),
    ];
    assert_eq!(
        describe(environment(&variables), Some(UBUNTU)),
        SystemDescription {
            distribution: "ubuntu".into(),
            distribution_version: Some("24.04".into()),
            desktops: vec!["ubuntu".into(), "gnome".into()],
            session_type: Some(SessionType::Wayland),
        }
    );
}

#[test]
fn reads_os_release_values_as_the_shell_does() {
    let text = "# ID=commented\n\
                ID='opensuse-tumbleweed'\n\
                VERSION_ID=\"2024\\\"1120\"\n\
                ID=\"fedora\"\n";
    let description = describe(environment(&[]), Some(text));
    assert_eq!(description.distribution, "fedora", "the last assignment");
    assert_eq!(
        description.distribution_version.as_deref(),
        Some("2024\"1120"),
        "a backslash takes the next character literally"
    );
    let single = describe(environment(&[]), Some("ID='arch'\n"));
    assert_eq!(single.distribution, "arch");
}

#[test]
fn a_missing_os_release_names_linux_without_a_version() {
    for text in [
        None,
        Some(""),
        Some("ID=\nVERSION_ID=\"\"\n"),
        Some("BUILD_ID=rolling\n"),
    ] {
        let description = describe(environment(&[]), text);
        assert_eq!(description.distribution, "linux", "{text:?}");
        assert_eq!(description.distribution_version, None, "{text:?}");
    }
}

#[test]
fn a_session_without_display_names_no_desktop_and_no_session_type() {
    assert_eq!(
        describe(environment(&[("XDG_CURRENT_DESKTOP", "")]), Some(UBUNTU)),
        SystemDescription {
            distribution: "ubuntu".into(),
            distribution_version: Some("24.04".into()),
            desktops: Vec::new(),
            session_type: None,
        }
    );
}

#[test]
fn the_session_is_wayland_then_x11_and_an_empty_variable_is_not_set() {
    let session = |variables: &[(&str, &str)]| session_type(environment(variables));
    assert_eq!(session(&[("DISPLAY", ":0")]), Some(SessionType::X11));
    assert_eq!(
        session(&[("WAYLAND_DISPLAY", ""), ("DISPLAY", ":1")]),
        Some(SessionType::X11)
    );
    assert_eq!(
        session(&[("WAYLAND_DISPLAY", "wayland-1"), ("DISPLAY", ":0")]),
        Some(SessionType::Wayland)
    );
    assert_eq!(session(&[]), None);
}

#[test]
fn the_install_kind_is_flatpak_then_appimage_then_a_development_build() {
    let kind = |variables: &[(&str, &str)], flatpak_info| {
        install_kind(environment(variables), flatpak_info)
    };
    assert_eq!(kind(&[], true), InstallKind::Flatpak);
    assert_eq!(
        kind(&[("FLATPAK_ID", "dev.bientot.Bientot")], false),
        InstallKind::Flatpak
    );
    assert_eq!(
        kind(&[("APPIMAGE", "/home/u/Bientot.AppImage")], false),
        InstallKind::AppImage
    );
    assert_eq!(
        kind(&[("APPIMAGE", "/home/u/Bientot.AppImage")], true),
        InstallKind::Flatpak
    );
    assert_eq!(
        kind(&[("APPIMAGE", ""), ("FLATPAK_ID", "")], false),
        InstallKind::Development
    );
}

#[test]
fn a_flatpak_reads_the_os_release_of_its_host_first() {
    assert_eq!(
        os_release_paths(InstallKind::Flatpak)[0],
        "/run/host/os-release"
    );
    assert_eq!(
        os_release_paths(InstallKind::AppImage),
        ["/etc/os-release", "/usr/lib/os-release"]
    );
}
