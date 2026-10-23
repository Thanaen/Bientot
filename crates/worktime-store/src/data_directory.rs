//! Where a launch keeps its data, and the single-instance key that follows from it, ported from
//! `Services/AppDataDirectory.cs` and the file names `App.xaml.cs` uses in it.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The flag that names the folder, followed by its path. `worktime-app` reads it and
/// `worktime-windows` writes it into the toast activator's command line: both depend on this
/// crate, and the flag is no more Windows' than the app's.
pub const ARGUMENT: &str = "--data-dir";

/// The folder that holds `settings.json`, `worktime.db`, the `secrets`, `app-state.json` and the
/// `backups` of imports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataDirectory(PathBuf);

impl DataDirectory {
    /// The folder `--data-dir` names, made absolute like .NET's `Path.GetFullPath`. Fails only
    /// when `path` is empty or the current directory is unavailable.
    pub fn named(path: &Path) -> io::Result<Self> {
        std::path::absolute(path).map(Self)
    }

    /// `<temporary_directory>/WorkTimeDemo/<throwaway_id as 32 hex digits>`, the folder of a demo
    /// launch without `--data-dir`. The .NET app passes the user's temporary folder and a new
    /// random GUID.
    pub fn throwaway(temporary_directory: &Path, throwaway_id: u128) -> Self {
        Self(
            temporary_directory
                .join("WorkTimeDemo")
                .join(format!("{throwaway_id:032x}")),
        )
    }

    /// `<local_app_data>/WorkTime`, the user's folder: the .NET app passes `%LOCALAPPDATA%`.
    pub fn user(local_app_data: &Path) -> Self {
        Self(local_app_data.join("WorkTime"))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn settings_file(&self) -> PathBuf {
        self.0.join("settings.json")
    }

    pub fn database_file(&self) -> PathBuf {
        self.0.join("worktime.db")
    }

    /// Where the diagnostic log is written and the export reads it.
    pub fn log_directory(&self) -> PathBuf {
        self.0.join("logs")
    }

    pub fn secrets_directory(&self) -> PathBuf {
        self.0.join("secrets")
    }

    /// An import chosen in Paramètres, which the next start applies.
    pub fn pending_import_file(&self) -> PathBuf {
        self.0.join("import.worktime")
    }

    /// Where an import first saves the ledger and settings it replaces.
    pub fn backups_directory(&self) -> PathBuf {
        self.0.join("backups")
    }

    /// `app-state.json`, which only the Rust app writes.
    pub fn app_state_file(&self) -> PathBuf {
        self.0.join("app-state.json")
    }

    /// `WorkTime-` and the lowercase hex SHA-256 of the folder path, upper-cased and without
    /// trailing separators. Windows paths are case-insensitive and a trailing separator names the
    /// same folder, so every spelling of one folder gets one key, the same as the .NET app's.
    pub fn instance_key(&self) -> String {
        let path = self.0.to_string_lossy();
        let normalized = upper_invariant(path.trim_end_matches(['\\', '/']));
        let mut key = String::from("WorkTime-");
        for byte in Sha256::digest(normalized.as_bytes()) {
            let _ = write!(key, "{byte:02x}");
        }
        key
    }
}

impl From<PathBuf> for DataDirectory {
    fn from(path: PathBuf) -> Self {
        Self(path)
    }
}

/// .NET's `string.ToUpperInvariant`: each character maps to its simple (single character) upper
/// case, where Rust's `to_uppercase` applies the full mapping that can lengthen the text.
pub fn upper_invariant(text: &str) -> String {
    text.chars().map(simple_upper).collect()
}

fn simple_upper(character: char) -> char {
    let shifted = |offset| char::from_u32(u32::from(character) + offset).unwrap_or(character);
    match character {
        // .NET keeps the dotless i and the long s, which would otherwise become ASCII letters.
        'ı' | 'ſ' => character,
        // Greek with ypogegrammeni: the full mapping appends a capital iota, the simple one moves
        // to the prosgegrammeni form.
        '\u{1F80}'..='\u{1F87}' | '\u{1F90}'..='\u{1F97}' | '\u{1FA0}'..='\u{1FA7}' => shifted(8),
        'ᾳ' | 'ῃ' | 'ῳ' => shifted(9),
        _ => {
            let mut upper = character.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(single), None) => single,
                _ => character,
            }
        }
    }
}

#[cfg(test)]
mod tests;
