//! Where a launch keeps its data, and the single-instance key that follows from it, ported from
//! `Services/AppDataDirectory.cs` and the file names `App.xaml.cs` uses in it.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The flag that names the folder, followed by its path. `bientot-app` reads it and
/// `bientot-windows` writes it into the toast activator's command line: both depend on this
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

    /// `<temporary_directory>/BientotDemo/<throwaway_id as 32 hex digits>`, the folder of a demo
    /// launch without `--data-dir`. The .NET app passes the user's temporary folder and a new
    /// random GUID.
    pub fn throwaway(temporary_directory: &Path, throwaway_id: u128) -> Self {
        Self(
            temporary_directory
                .join("BientotDemo")
                .join(format!("{throwaway_id:032x}")),
        )
    }

    /// `<local_app_data>/Bientot`, the user's folder under `%LOCALAPPDATA%`. It was `WorkTime`
    /// before the rename, which [`Location`](crate::data_migration::Location) moves.
    pub fn user(local_app_data: &Path) -> Self {
        Self(local_app_data.join("Bientot"))
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

    /// The key of the folder's resolved path (see [`resolved`]), so that a short (8.3) name, a
    /// junction, a mapped drive or a `..` segment names the same instance as the folder's usual
    /// spelling.
    pub fn instance_key(&self) -> String {
        key_of(&resolved(&self.0))
    }

    /// [`instance_key`](Self::instance_key) without folding case, for a file system where `Data`
    /// and `data` are two folders, as on Linux.
    pub fn case_sensitive_instance_key(&self) -> String {
        let path = resolved(&self.0);
        hashed(path.to_string_lossy().trim_end_matches(['\\', '/']))
    }
}

/// `WorkTime-` and the lowercase hex SHA-256 of `path`, upper-cased and without trailing
/// separators. Windows paths are case-insensitive and a trailing separator names the same folder,
/// so these spellings share a key, the same as the .NET app's.
fn key_of(path: &Path) -> String {
    let path = path.to_string_lossy();
    hashed(&upper_invariant(path.trim_end_matches(['\\', '/'])))
}

/// `WorkTime-` and the lowercase hex SHA-256 of `normalized`.
fn hashed(normalized: &str) -> String {
    let mut key = String::from("WorkTime-");
    for byte in Sha256::digest(normalized.as_bytes()) {
        let _ = write!(key, "{byte:02x}");
    }
    key
}

/// `path` with its deepest existing ancestor replaced by the canonical path of that ancestor,
/// without Windows' `\\?\` prefix. A folder that the first launch has yet to create therefore
/// resolves to the same path as once it exists. Falls back to `path` when no ancestor resolves.
fn resolved(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut missing = Vec::new();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(existing) {
            let mut resolved = without_verbatim_prefix(canonical);
            resolved.extend(missing.iter().rev());
            return resolved;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                existing = parent;
            }
            _ => return path.to_owned(),
        }
    }
}

/// `C:\folder` for `\\?\C:\folder` and `\\server\share` for `\\?\UNC\server\share`, the
/// spellings that `canonicalize` returns on Windows.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{share}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(disk) if disk.as_bytes().get(1) == Some(&b':') => PathBuf::from(disk),
        _ => path,
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
