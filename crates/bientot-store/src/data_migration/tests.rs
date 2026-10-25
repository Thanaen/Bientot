use std::fs;

use super::*;

struct Folders {
    local_app_data: tempfile::TempDir,
}

impl Folders {
    fn new() -> Self {
        Self {
            local_app_data: tempfile::tempdir().unwrap(),
        }
    }

    fn old(&self) -> PathBuf {
        self.local_app_data.path().join("WorkTime")
    }

    fn new_folder(&self) -> PathBuf {
        self.local_app_data.path().join("Bientot")
    }

    fn user(&self) -> Location {
        Location::User {
            local_app_data: self.local_app_data.path().to_owned(),
        }
    }

    /// A folder holding a ledger and a secret that read `text`.
    fn fill(&self, folder: &Path, text: &str) {
        fs::create_dir_all(folder.join("secrets")).unwrap();
        fs::write(folder.join("worktime.db"), text).unwrap();
        fs::write(folder.join("secrets").join("token.secret"), text).unwrap();
    }

    fn ledger(folder: &Path) -> Option<String> {
        fs::read_to_string(folder.join("worktime.db")).ok()
    }
}

fn never_renames(_: &Path, _: &Path) -> io::Result<()> {
    panic!("nothing should be renamed")
}

fn rename(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

#[test]
fn a_new_install_starts_in_bientot() {
    let folders = Folders::new();
    assert_eq!(folders.user().moving_from(), None);
    let (directory, migration) = folders.user().settle(never_renames);
    assert_eq!(directory.path(), folders.new_folder());
    assert!(migration.is_none());
    assert!(!folders.new_folder().exists() && !folders.old().exists());
}

#[test]
fn the_worktime_folder_moves_whole_to_bientot() {
    let folders = Folders::new();
    folders.fill(&folders.old(), "ledger");
    assert_eq!(folders.user().moving_from().unwrap().path(), folders.old());

    let (directory, migration) = folders.user().settle(rename);

    assert_eq!(directory.path(), folders.new_folder());
    assert!(matches!(migration, Some(Migration::Moved)));
    assert!(!folders.old().exists());
    assert_eq!(Folders::ledger(&folders.new_folder()).unwrap(), "ledger");
    assert_eq!(
        fs::read_to_string(directory.secrets_directory().join("token.secret")).unwrap(),
        "ledger"
    );
    // The next launch finds it moved.
    assert_eq!(folders.user().moving_from(), None);
    let (again, migration) = folders.user().settle(never_renames);
    assert_eq!(again, directory);
    assert!(migration.is_none());
}

#[test]
fn bientot_wins_over_a_worktime_folder_left_beside_it() {
    let folders = Folders::new();
    folders.fill(&folders.old(), "old");
    folders.fill(&folders.new_folder(), "new");
    assert_eq!(folders.user().moving_from(), None);

    let (directory, migration) = folders.user().settle(never_renames);

    assert_eq!(directory.path(), folders.new_folder());
    assert!(matches!(migration, Some(Migration::BothPresent)));
    assert_eq!(Folders::ledger(&folders.old()).unwrap(), "old");
    assert_eq!(Folders::ledger(&folders.new_folder()).unwrap(), "new");
}

#[test]
fn a_failed_move_stays_on_worktime_and_the_next_launch_tries_again() {
    let folders = Folders::new();
    folders.fill(&folders.old(), "ledger");

    let (directory, migration) = folders
        .user()
        .settle(|_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)));

    assert_eq!(directory.path(), folders.old());
    let Some(Migration::Failed(error)) = migration else {
        panic!("the failure is reported, got {migration:?}");
    };
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(Folders::ledger(&folders.old()).unwrap(), "ledger");
    assert!(!folders.new_folder().exists());

    let (directory, migration) = folders.user().settle(rename);
    assert_eq!(directory.path(), folders.new_folder());
    assert!(matches!(migration, Some(Migration::Moved)));
    assert_eq!(Folders::ledger(&folders.new_folder()).unwrap(), "ledger");
}

#[test]
fn a_move_another_launch_made_first_is_followed() {
    let folders = Folders::new();
    folders.fill(&folders.old(), "ledger");

    let (directory, migration) = folders.user().settle(|from, to| {
        fs::rename(from, to)?;
        Err(io::Error::from(io::ErrorKind::NotFound))
    });

    assert_eq!(directory.path(), folders.new_folder());
    assert!(migration.is_none());
    assert_eq!(Folders::ledger(&folders.new_folder()).unwrap(), "ledger");
}

#[test]
fn a_chosen_folder_never_moves_the_users() {
    let folders = Folders::new();
    folders.fill(&folders.old(), "ledger");
    let chosen = DataDirectory::from(folders.local_app_data.path().join("portable"));
    let location = Location::Chosen(chosen.clone());
    assert_eq!(location.moving_from(), None);

    let (directory, migration) = location.settle(never_renames);

    assert_eq!(directory, chosen);
    assert!(migration.is_none());
    assert_eq!(Folders::ledger(&folders.old()).unwrap(), "ledger");
    assert!(!folders.new_folder().exists());
}

/// The ledger held open as SQLite holds it, without sharing its deletion, by a WorkTime this
/// launch cannot claim, such as the .NET one.
#[cfg(windows)]
#[test]
fn a_ledger_held_open_keeps_the_folder_on_worktime_for_this_launch() {
    use std::os::windows::fs::OpenOptionsExt as _;
    const FILE_SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    let folders = Folders::new();
    folders.fill(&folders.old(), "ledger");
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ_WRITE)
        .open(folders.old().join("worktime.db"))
        .unwrap();

    let (directory, migration) = folders.user().settle(rename);

    assert_eq!(directory.path(), folders.old());
    assert!(
        matches!(migration, Some(Migration::Failed(_))),
        "{migration:?}"
    );
    assert!(!folders.new_folder().exists());
    drop(held);
    let (directory, migration) = folders.user().settle(rename);
    assert_eq!(directory.path(), folders.new_folder());
    assert!(matches!(migration, Some(Migration::Moved)));
}
