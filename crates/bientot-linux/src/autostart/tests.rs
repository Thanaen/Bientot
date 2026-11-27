use std::ffi::OsString;
use std::path::{Path, PathBuf};

use bientot_platform::startup::StartupRegistrationState::{
    Disabled, Enabled, RegistrationMismatch, RegistrationMoved, Unavailable,
};
use bientot_platform::startup::{StartupPort, StartupRegistrationState};
use tempfile::TempDir;

use super::*;

const EXECUTABLE: &str = "/opt/Bientôt/worktime";

/// A configuration folder of its own and the registration of `executable` in it.
struct Config {
    home: TempDir,
    autostart: XdgAutostart,
}

impl Config {
    fn new(executable: &str) -> Self {
        let home = TempDir::new().unwrap();
        let autostart = XdgAutostart::new(Some(home.path()), Some(Path::new(executable)));
        Self { home, autostart }
    }

    fn file(&self) -> PathBuf {
        self.home
            .path()
            .join("autostart/dev.bientot.Bientot.desktop")
    }

    fn put(&self, text: &str) {
        std::fs::create_dir_all(self.file().parent().unwrap()).unwrap();
        std::fs::write(self.file(), text).unwrap();
    }

    fn text(&self) -> Option<String> {
        std::fs::read_to_string(self.file()).ok()
    }

    fn state(&self) -> StartupRegistrationState {
        self.autostart.state()
    }
}

/// An entry as a desktop's own tool may write it, starting `exec`.
fn entry_starting(exec: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName=Bientôt\nExec={exec}\n")
}

#[test]
fn no_entry_is_disabled_and_enabling_writes_the_entry_of_the_contract() {
    let config = Config::new(EXECUTABLE);
    assert_eq!(config.state(), Disabled);

    assert_eq!(config.autostart.set_enabled(true), Enabled);
    assert_eq!(
        config.text().unwrap(),
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Bientôt\n\
         Exec=\"/opt/Bientôt/worktime\" --autostart\n\
         Icon=dev.bientot.Bientot\n\
         X-GNOME-Autostart-enabled=true\n"
    );
    assert_eq!(config.state(), Enabled);

    assert_eq!(config.autostart.set_enabled(false), Disabled);
    assert_eq!(config.text(), None, "the entry is removed");
    assert_eq!(config.autostart.set_enabled(false), Disabled);
}

#[test]
fn an_entry_turned_off_is_disabled_and_enabling_turns_it_back_on() {
    for turned_off in ["Hidden=true", "X-GNOME-Autostart-enabled=false"] {
        let config = Config::new(EXECUTABLE);
        config.put(&format!("{}{turned_off}\n", entry_starting(EXECUTABLE)));
        assert_eq!(config.state(), Disabled, "{turned_off}");

        assert_eq!(config.autostart.set_enabled(true), Enabled, "{turned_off}");
        assert!(!config.text().unwrap().contains(turned_off));
    }
}

#[test]
fn turned_on_again_by_its_tool_an_entry_starts_this_install() {
    let config = Config::new(EXECUTABLE);
    config.put(&format!(
        "{}Hidden=false\nX-GNOME-Autostart-enabled=true\n",
        entry_starting(EXECUTABLE)
    ));
    assert_eq!(config.state(), Enabled);
}

#[test]
fn an_entry_of_another_path_is_a_mismatch_that_only_enabling_replaces() {
    let other = entry_starting("\"/home/ana/Downloads/Bientot.AppImage\" --autostart");
    let config = Config::new(EXECUTABLE);
    config.put(&other);
    assert_eq!(config.state(), RegistrationMismatch);

    assert_eq!(config.autostart.set_enabled(false), RegistrationMismatch);
    assert_eq!(
        config.text().unwrap(),
        other,
        "another install's entry stays"
    );

    assert_eq!(config.autostart.set_enabled(true), Enabled);
    assert_eq!(config.state(), Enabled);
}

#[test]
fn the_flatpaks_entry_is_a_mismatch_and_no_other_copy_that_starts_too() {
    let flatpak = "[Desktop Entry]\nType=Application\nName=dev.bientot.Bientot\n\
                   Exec=flatpak run --command=bientot dev.bientot.Bientot --autostart\n\
                   X-Flatpak=dev.bientot.Bientot\n";
    let config = Config::new(EXECUTABLE);
    config.put(flatpak);
    assert_eq!(config.state(), RegistrationMismatch);
    assert!(!config.autostart.other_copy_starts());

    assert_eq!(config.autostart.set_enabled(false), RegistrationMismatch);
    assert_eq!(config.text().unwrap(), flatpak, "the Flatpak's entry stays");
}

const APPIMAGE: &str = "/home/ana/Applications/Bientot.AppImage";

/// The registration of the AppImage file [`APPIMAGE`], under a configuration folder of its own,
/// whose entry is `entry`.
fn appimage_reading(entry: &str) -> Config {
    let home = TempDir::new().unwrap();
    let autostart = XdgAutostart::for_appimage(Some(home.path()), Path::new(APPIMAGE));
    let config = Config { home, autostart };
    config.put(entry);
    config
}

#[test]
fn an_appimage_reads_an_entry_of_another_file_as_moved_and_enabling_repairs_it() {
    let old = entry_starting("\"/home/ana/Downloads/Bientot.AppImage\" --autostart");
    let config = appimage_reading(&old);
    assert_eq!(config.state(), RegistrationMoved);

    assert_eq!(config.autostart.set_enabled(false), RegistrationMoved);
    assert_eq!(config.text().unwrap(), old, "the old entry stays");

    assert_eq!(config.autostart.set_enabled(true), Enabled);
    assert!(
        config
            .text()
            .unwrap()
            .contains("Exec=\"/home/ana/Applications/Bientot.AppImage\" --autostart\n")
    );
}

#[test]
fn an_appimage_reads_the_flatpaks_entry_and_one_it_cannot_read_as_a_mismatch() {
    for exec in [
        "flatpak run --command=bientot dev.bientot.Bientot --autostart",
        "bientot --autostart",
        "\"/home/ana/Downloads/Bientot.AppImage",
        "",
    ] {
        let config = appimage_reading(&entry_starting(exec));
        assert_eq!(config.state(), RegistrationMismatch, "{exec:?}");
    }
    let config = appimage_reading(&entry_starting(APPIMAGE));
    assert_eq!(config.state(), Enabled);
}

#[test]
fn an_entry_of_another_path_turned_off_is_disabled_and_left_alone() {
    let other = format!("{}Hidden=true\n", entry_starting("/usr/bin/bientot"));
    let config = Config::new(EXECUTABLE);
    config.put(&other);
    assert_eq!(config.state(), Disabled);
    assert_eq!(config.autostart.set_enabled(false), Disabled);
    assert_eq!(config.text().unwrap(), other);
}

#[test]
fn a_path_with_spaces_and_reserved_characters_starts_this_install_once_written() {
    let executable = r#"/home/ana/My Apps/100% "fresh" $HOME `x` \ Bientôt/worktime"#;
    let config = Config::new(executable);
    assert_eq!(config.autostart.set_enabled(true), Enabled);
    let text = config.text().unwrap();
    assert!(
        text.contains(
            r#"Exec="/home/ana/My Apps/100%% \\"fresh\\" \\$HOME \\`x\\` \\\\ Bientôt/worktime" --autostart"#
        ),
        "{text}"
    );
    assert_eq!(config.state(), Enabled);
    assert_eq!(
        Entry::parse(&text).exec.unwrap(),
        [executable, "--autostart"]
    );
    let prefix = XdgAutostart::new(
        Some(config.home.path()),
        Some(Path::new("/home/ana/My Apps/100%")),
    );
    assert_eq!(prefix.state(), RegistrationMismatch, "another install");
}

#[test]
fn an_entry_quoted_or_not_by_another_tool_reads_as_the_desktop_reads_it() {
    for (exec, program) in [
        ("/opt/Bientôt/worktime --autostart", EXECUTABLE),
        ("/opt/Bientôt/worktime", EXECUTABLE),
        ("\"/opt/Bientôt/worktime\" %U", EXECUTABLE),
        (
            r#""/home/ana/My Apps/worktime" --autostart"#,
            "/home/ana/My Apps/worktime",
        ),
        (
            r#""/home/ana/My\sApps/worktime""#,
            "/home/ana/My Apps/worktime",
        ),
    ] {
        let config = Config::new(program);
        config.put(&entry_starting(exec));
        assert_eq!(config.state(), Enabled, "{exec}");
    }
    for exec in ["\"/opt/Bientôt/worktime", r"/opt/Bientôt/worktime\q", ""] {
        let config = Config::new(EXECUTABLE);
        config.put(&entry_starting(exec));
        assert_eq!(config.state(), RegistrationMismatch, "{exec:?}");
    }
}

#[test]
fn only_the_keys_of_the_main_group_count() {
    let config = Config::new(EXECUTABLE);
    config.put(&format!(
        "# Hidden=true\n{}\n[Desktop Action quit]\nExec=/usr/bin/other\nHidden=true\n",
        entry_starting(EXECUTABLE)
    ));
    assert_eq!(config.state(), Enabled);
}

#[test]
fn without_an_executable_or_a_folder_nothing_is_read_or_written() {
    let home = TempDir::new().unwrap();
    for autostart in [
        XdgAutostart::new(Some(home.path()), None),
        XdgAutostart::new(Some(home.path()), Some(Path::new("worktime"))),
        XdgAutostart::new(None, Some(Path::new(EXECUTABLE))),
    ] {
        assert_eq!(autostart.state(), Unavailable);
        assert_eq!(autostart.set_enabled(true), Unavailable);
    }
    assert!(!home.path().join("autostart").exists());
}

#[test]
fn an_unreadable_entry_is_unavailable() {
    let config = Config::new(EXECUTABLE);
    std::fs::create_dir_all(config.file()).unwrap();
    assert_eq!(config.state(), Unavailable);
    assert_eq!(config.autostart.set_enabled(true), Unavailable);
    assert_eq!(config.autostart.set_enabled(false), Unavailable);
}

fn environment(variables: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
    let variables: Vec<(String, OsString)> = variables
        .iter()
        .map(|(name, value)| (name.to_string(), value.into()))
        .collect();
    move |name| {
        variables
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.clone())
    }
}

#[test]
fn the_entry_goes_under_an_absolute_xdg_config_home_else_under_dot_config() {
    let config_home = |variables: &[(&str, &str)]| config_home(environment(variables));
    assert_eq!(
        config_home(&[("XDG_CONFIG_HOME", "/tmp/config"), ("HOME", "/home/ana")]),
        Some(PathBuf::from("/tmp/config"))
    );
    for ignored in ["", "config"] {
        assert_eq!(
            config_home(&[("XDG_CONFIG_HOME", ignored), ("HOME", "/home/ana")]),
            Some(PathBuf::from("/home/ana/.config"))
        );
    }
    assert_eq!(config_home(&[]), None);
}

#[test]
fn under_a_temporary_xdg_config_home_the_entry_is_written_there_and_removed() {
    let home = TempDir::new().unwrap();
    let variables = environment(&[
        ("XDG_CONFIG_HOME", home.path().to_str().unwrap()),
        ("HOME", "/nonexistent"),
    ]);
    let autostart = XdgAutostart::new(
        config_home(variables).as_deref(),
        Some(Path::new(EXECUTABLE)),
    );
    let file = home.path().join("autostart/dev.bientot.Bientot.desktop");

    assert_eq!(autostart.set_enabled(true), Enabled);
    assert!(file.is_file());
    assert_eq!(
        std::fs::read_dir(file.parent().unwrap()).unwrap().count(),
        1,
        "no partial file is left"
    );
    assert_eq!(autostart.set_enabled(false), Disabled);
    assert!(!file.exists());
}

#[test]
fn an_appimage_is_known_by_its_absolute_file() {
    assert_eq!(
        appimage_file(Some("/home/ana/Bientot.AppImage".into())),
        Some(PathBuf::from("/home/ana/Bientot.AppImage"))
    );
    assert_eq!(appimage_file(None), None);
    assert_eq!(appimage_file(Some("Bientot.AppImage".into())), None);
}

#[test]
fn a_flatpak_starts_through_the_background_portal_and_every_other_install_through_an_entry() {
    let through_the_portal: Vec<_> = InstallKind::ALL
        .into_iter()
        .filter(|kind| Autostart::for_install(*kind) == Autostart::BackgroundPortal)
        .collect();
    assert_eq!(through_the_portal, [InstallKind::Flatpak]);
}
