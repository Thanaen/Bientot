//! The move of the user's folder from `WorkTime` to `Bientot`, the app's new name. Only the
//! default folder moves: a `--data-dir` folder and a demo's keep their path.
//!
//! The move is one rename on the same volume, so the folder is either all under its old name or
//! all under its new one, never split between them. Each launch decides again from which names
//! exist, so a launch whose rename failed tries again at the next one.

use std::io;
use std::path::{Path, PathBuf};

use crate::data_directory::DataDirectory;

/// Where a launch keeps its data, before the user's folder has moved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// A `--data-dir` folder or a demo's, opened as it is.
    Chosen(DataDirectory),
    /// The user's folder under `%LOCALAPPDATA%`, or the XDG data folder elsewhere.
    User { local_app_data: PathBuf },
}

/// What became of the move, for the diagnostic log.
#[derive(Debug)]
pub enum Migration {
    /// `WorkTime` was renamed `Bientot`.
    Moved,
    /// Both exist: `Bientot` is used and `WorkTime`, made again by an older build, is left alone.
    /// They are never merged.
    BothPresent,
    /// The rename failed, a file in `WorkTime` being in use perhaps: this launch stays on
    /// `WorkTime`.
    Failed(io::Error),
}

/// What a launch on the user's folder does, from which of its two names exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    /// Only `Bientot` exists, or neither, for a new install.
    UseNew,
    /// Both exist.
    UseNewBesideOld,
    /// Only `WorkTime` exists.
    Move,
}

fn plan(old_exists: bool, new_exists: bool) -> Plan {
    match (old_exists, new_exists) {
        (true, true) => Plan::UseNewBesideOld,
        (true, false) => Plan::Move,
        (false, _) => Plan::UseNew,
    }
}

impl Location {
    /// The folder this launch would rename: the user's, while only its old name exists. On
    /// Windows the launch claims that folder first, so that a WorkTime still running on it keeps
    /// it.
    pub fn moving_from(&self) -> Option<DataDirectory> {
        let Self::User { local_app_data } = self else {
            return None;
        };
        let (old, new) = names(local_app_data);
        (observe(&old, &new) == Plan::Move).then_some(old)
    }

    /// The folder this launch opens, after renaming `WorkTime` with `rename` when only that name
    /// exists. A failed rename leaves `WorkTime` whole, and the launch opens it.
    pub fn settle(
        self,
        rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
    ) -> (DataDirectory, Option<Migration>) {
        let local_app_data = match self {
            Self::Chosen(directory) => return (directory, None),
            Self::User { local_app_data } => local_app_data,
        };
        let (old, new) = names(&local_app_data);
        let mut failed = None;
        if observe(&old, &new) == Plan::Move {
            match rename(old.path(), new.path()) {
                Ok(()) => return (new, Some(Migration::Moved)),
                Err(error) => failed = Some(error),
            }
        }
        // Looks again after a failed rename: another launch may have moved the folder meanwhile.
        match observe(&old, &new) {
            Plan::UseNew => (new, None),
            Plan::UseNewBesideOld => (new, Some(Migration::BothPresent)),
            Plan::Move => (old, failed.map(Migration::Failed)),
        }
    }
}

/// The user's folder under its name before the rename, then under its name now.
fn names(local_app_data: &Path) -> (DataDirectory, DataDirectory) {
    (
        DataDirectory::from(local_app_data.join("WorkTime")),
        DataDirectory::user(local_app_data),
    )
}

fn observe(old: &DataDirectory, new: &DataDirectory) -> Plan {
    plan(old.path().exists(), new.path().exists())
}

#[cfg(test)]
mod tests;
