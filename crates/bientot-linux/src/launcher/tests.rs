use std::ffi::OsString;
use std::path::{Path, PathBuf};

use bientot_platform::launcher::LauncherPort;
use tempfile::TempDir;

use super::*;
use crate::desktop_entry::Entry;

const APPIMAGE: &str = "/home/ana/Applications/Bientôt.AppImage";
/// What the AppImage holds as `.DirIcon`; any bytes do.
const DIR_ICON: &[u8] = b"\x89PNG\r\n\x1a\n the icon of the AppImage";

/// A data folder of its own, and an AppImage mount that holds the icon.
struct Home {
    data: TempDir,
    mount: TempDir,
}

impl Home {
    fn new() -> Self {
        let mount = TempDir::new().unwrap();
        std::fs::write(mount.path().join(".DirIcon"), DIR_ICON).unwrap();
        Self {
            data: TempDir::new().unwrap(),
            mount,
        }
    }

    /// The entry of the AppImage at `appimage`, mounted here.
    fn launcher(&self, appimage: &str) -> LauncherEntry {
        LauncherEntry::new(
            self.data.path(),
            Path::new(appimage),
            Some(self.mount.path()),
        )
        .unwrap()
    }

    fn entry_file(&self) -> PathBuf {
        self.data
            .path()
            .join("applications/dev.bientot.Bientot.desktop")
    }

    fn icon_file(&self) -> PathBuf {
        self.data
            .path()
            .join("icons/hicolor/256x256/apps/dev.bientot.Bientot.png")
    }

    fn entry(&self) -> Option<String> {
        std::fs::read_to_string(self.entry_file()).ok()
    }

    fn icon(&self) -> Option<Vec<u8>> {
        std::fs::read(self.icon_file()).ok()
    }

    /// The program the entry starts, as the desktop reads it.
    fn program(&self) -> Option<String> {
        let exec = Entry::parse(&self.entry()?).exec?;
        exec.into_iter().next()
    }
}

#[test]
fn the_first_run_writes_the_entry_of_the_contract_and_the_icon_of_the_appimage() {
    let home = Home::new();
    let launcher = home.launcher(APPIMAGE);
    assert!(!launcher.is_added());

    launcher.settle(true).unwrap();

    assert_eq!(
        home.entry().unwrap(),
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Bientôt\n\
         Exec=\"/home/ana/Applications/Bientôt.AppImage\"\n\
         TryExec=/home/ana/Applications/Bientôt.AppImage\n\
         Icon=dev.bientot.Bientot\n\
         Categories=Office;Utility;\n\
         StartupWMClass=dev.bientot.Bientot\n"
    );
    assert_eq!(home.icon().unwrap(), DIR_ICON);
    assert!(launcher.is_added());
}

#[test]
fn a_later_start_leaves_the_entry_of_this_appimage_as_the_user_left_it() {
    let home = Home::new();
    let launcher = home.launcher(APPIMAGE);
    launcher.settle(true).unwrap();
    // As a menu editor hides an entry the user owns.
    let edited = format!("{}NoDisplay=true\n", home.entry().unwrap());
    std::fs::write(home.entry_file(), &edited).unwrap();

    for first_run in [true, false] {
        launcher.settle(first_run).unwrap();
        assert_eq!(home.entry().unwrap(), edited, "first run: {first_run}");
    }
    assert_eq!(home.icon().unwrap(), DIR_ICON);
}

#[test]
fn a_start_from_another_place_points_the_entry_at_the_moved_appimage() {
    let home = Home::new();
    home.launcher("/home/ana/Downloads/Bientot.AppImage")
        .settle(true)
        .unwrap();
    std::fs::remove_file(home.icon_file()).unwrap();

    let moved = home.launcher(APPIMAGE);
    assert!(!moved.is_added(), "the entry starts the old place");
    moved.settle(false).unwrap();

    assert_eq!(home.program().as_deref(), Some(APPIMAGE));
    assert_eq!(home.icon().unwrap(), DIR_ICON);
    assert!(moved.is_added());
}

#[test]
fn removing_deletes_the_entry_and_the_icon_and_later_starts_leave_them_out() {
    let home = Home::new();
    let launcher = home.launcher(APPIMAGE);
    launcher.settle(true).unwrap();

    launcher.set_added(false).unwrap();
    assert_eq!(home.entry(), None);
    assert_eq!(home.icon(), None);
    assert!(!launcher.is_added());

    launcher.settle(false).unwrap();
    assert_eq!(home.entry(), None, "a later start does not add it back");
    assert_eq!(home.icon(), None);
    launcher.set_added(false).unwrap();

    launcher.set_added(true).unwrap();
    assert_eq!(home.program().as_deref(), Some(APPIMAGE));
    assert_eq!(home.icon().unwrap(), DIR_ICON);
    assert!(launcher.is_added());
}

#[test]
fn removing_leaves_the_entry_of_another_appimage_and_its_icon() {
    let home = Home::new();
    let other = "/home/ana/Downloads/Bientot.AppImage";
    home.launcher(other).settle(true).unwrap();

    home.launcher(APPIMAGE).set_added(false).unwrap();

    assert_eq!(home.program().as_deref(), Some(other));
    assert_eq!(home.icon().unwrap(), DIR_ICON);
}

#[test]
fn a_path_with_spaces_and_reserved_characters_is_started_as_it_is() {
    let home = Home::new();
    let appimage = r#"/home/ana/Mes "apps" $HOME 100% \ `x`/Bientôt.AppImage"#;
    let launcher = home.launcher(appimage);

    launcher.settle(true).unwrap();

    let text = home.entry().unwrap();
    assert!(
        text.contains(
            r#"Exec="/home/ana/Mes \\"apps\\" \\$HOME 100%% \\\\ \\`x\\`/Bientôt.AppImage""#
        ),
        "{text}"
    );
    assert!(
        text.contains(r#"TryExec=/home/ana/Mes "apps" $HOME 100% \\ `x`/Bientôt.AppImage"#),
        "{text}"
    );
    assert_eq!(home.program().as_deref(), Some(appimage));
    assert!(launcher.is_added());
}

#[test]
fn an_entry_hidden_as_deleted_is_not_in_the_menu_and_a_start_leaves_it() {
    let home = Home::new();
    let launcher = home.launcher(APPIMAGE);
    launcher.settle(true).unwrap();
    let hidden = format!("{}Hidden=true\n", home.entry().unwrap());
    std::fs::write(home.entry_file(), &hidden).unwrap();

    assert!(!launcher.is_added());
    launcher.settle(false).unwrap();
    assert_eq!(home.entry().unwrap(), hidden);
}

#[test]
fn without_the_mount_the_entry_is_written_without_an_icon() {
    let home = Home::new();
    let launcher = LauncherEntry::new(home.data.path(), Path::new(APPIMAGE), None).unwrap();

    launcher.settle(true).unwrap();

    assert_eq!(home.program().as_deref(), Some(APPIMAGE));
    assert_eq!(home.icon(), None);
}

#[test]
fn a_change_the_folder_refuses_is_an_error_and_nothing_is_added() {
    let home = Home::new();
    // A file where the folder of the entries should be.
    std::fs::write(home.data.path().join("applications"), "").unwrap();
    let launcher = home.launcher(APPIMAGE);

    assert!(launcher.settle(true).is_err());
    assert!(launcher.set_added(true).is_err());
    assert!(!launcher.is_added());
}

fn environment(variables: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
    let variables: Vec<(String, OsString)> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).into()))
        .collect();
    move |name| {
        variables
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.clone())
    }
}

#[test]
fn only_an_appimage_has_an_entry_to_write() {
    let home = Home::new();
    let data_home = home.data.path().to_str().unwrap();
    let appimage = [("APPIMAGE", APPIMAGE), ("XDG_DATA_HOME", data_home)];

    let launcher = LauncherEntry::of(environment(&appimage), false).unwrap();
    launcher.settle(true).unwrap();
    assert_eq!(home.program().as_deref(), Some(APPIMAGE));

    let flatpak = [
        ("FLATPAK_ID", "dev.bientot.Bientot"),
        appimage[0],
        appimage[1],
    ];
    assert!(LauncherEntry::of(environment(&flatpak), false).is_none());
    assert!(LauncherEntry::of(environment(&appimage), true).is_none());
    let development = [("XDG_DATA_HOME", data_home)];
    assert!(LauncherEntry::of(environment(&development), false).is_none());
}

#[test]
fn the_entry_goes_under_an_absolute_xdg_data_home_else_under_dot_local_share() {
    let launcher = |variables: &[(&str, &str)]| LauncherEntry::of(environment(variables), false);
    let entry = |variables: &[(&str, &str)]| launcher(variables).map(|launcher| launcher.entry);
    let applications = |folder: &str| Some(Path::new(folder).join("dev.bientot.Bientot.desktop"));

    assert_eq!(
        entry(&[
            ("APPIMAGE", APPIMAGE),
            ("XDG_DATA_HOME", "/tmp/data"),
            ("HOME", "/home/ana")
        ]),
        applications("/tmp/data/applications")
    );
    assert_eq!(
        entry(&[
            ("APPIMAGE", APPIMAGE),
            ("XDG_DATA_HOME", "data"),
            ("HOME", "/home/ana")
        ]),
        applications("/home/ana/.local/share/applications")
    );
    assert_eq!(entry(&[("APPIMAGE", APPIMAGE)]), None);
    assert_eq!(
        entry(&[("APPIMAGE", "Bientot.AppImage"), ("HOME", "/home/ana")]),
        None,
        "a relative AppImage path names nothing to start"
    );
    let mounted = launcher(&[
        ("APPIMAGE", APPIMAGE),
        ("APPDIR", "/tmp/.mount_BientoXYZ"),
        ("HOME", "/home/ana"),
    ])
    .unwrap();
    assert_eq!(
        mounted.dir_icon.as_deref(),
        Some(Path::new("/tmp/.mount_BientoXYZ/.DirIcon"))
    );
}
