//! The Windows side of « Exporter le diagnostic… », ported from `Diagnostics/DiagnosticExportService.cs`
//! and `App.xaml.cs`: the final paths the export checks the log directory and each log file
//! against and the system the snapshot describes. The save dialog is in [`crate::dialogs`].

#[cfg(windows)]
mod native;

use worktime_store::data_directory::upper_invariant;

#[cfg(windows)]
pub use native::{directory_is_final, file_is_in, native_machine, operating_system_version};

const SEPARATORS: [char; 2] = ['\\', '/'];

/// `GetFinalPathNameByHandleW` answers a verbatim path, `\\?\C:\…`, or `\\?\UNC\server\share\…`
/// on a network share. .NET strips the prefix before it compares the path.
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "only the Windows final paths strip it outside the tests"
    )
)]
fn without_verbatim_prefix(path: &str) -> String {
    const UNC_PREFIX: &str = r"\\?\UNC\";
    const DRIVE_PREFIX: &str = r"\\?\";
    match path.get(..UNC_PREFIX.len()) {
        Some(prefix) if prefix.eq_ignore_ascii_case(UNC_PREFIX) => {
            format!(r"\\{}", &path[UNC_PREFIX.len()..])
        }
        _ => path.strip_prefix(DRIVE_PREFIX).unwrap_or(path).to_owned(),
    }
}

/// Whether `final_path` names `directory`, compared as .NET does: ignoring case and trailing
/// separators. A `--data-dir` may itself be verbatim, so both sides lose the prefix.
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "only the Windows final paths compare them outside the tests"
    )
)]
fn names_directory(final_path: &str, directory: &str) -> bool {
    let normalized =
        |path: &str| upper_invariant(without_verbatim_prefix(path).trim_end_matches(SEPARATORS));
    normalized(final_path) == normalized(directory)
}

/// Whether `final_path` is directly inside `directory`: its parent, as `Path.GetDirectoryName`
/// cuts it, names `directory`.
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "only the Windows final paths compare them outside the tests"
    )
)]
fn is_directly_inside(final_path: &str, directory: &str) -> bool {
    final_path
        .rfind(SEPARATORS)
        .is_some_and(|name_start| names_directory(&final_path[..name_start], directory))
}

#[cfg(test)]
mod tests;
