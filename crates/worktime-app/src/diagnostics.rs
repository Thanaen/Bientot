//! « Exporter le diagnostic… » of the tray and of the startup failure, ported from
//! `ExportDiagnosticsAsync` of `App.xaml.cs`: the user picks where to save the ZIP, the archive
//! of the snapshot and of the local log is written off the main thread, then the window that
//! asked says whether it was saved.

use std::fmt::Display;
use std::path::{Path, PathBuf};

use gpui::{App, AsyncApp, Global};
use jiff::Timestamp;
use worktime_core::diagnostics::{
    DiagnosticArchitecture, DiagnosticExport, DiagnosticExportError, DiagnosticExportResult,
    DiagnosticInitializationState, DiagnosticNumericVersion, DiagnosticRuntimeSnapshot,
    DiagnosticSnapshot, create_snapshot,
};
use worktime_domain::configuration::WorkTimeSettings;
use worktime_domain::time::SystemClock;

use crate::file_dialogs;
use crate::host::Today;
use crate::shell;
use crate::strings::{Key, Language};
use crate::widgets::Notice;

/// The name the save dialog offers.
const SUGGESTED_FILE_NAME: &str = "bientot-diagnostics.zip";

/// How an export ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The user closed the save dialog: nothing to say.
    Cancelled,
    Saved,
    /// The dialog or the archive failed; the user may pick another place.
    Failed,
}

impl Outcome {
    /// The notice the flyout shows, as `MainWindow.ShowNotice` did.
    pub fn notice(self, language: Language) -> Option<Notice> {
        match self {
            Self::Cancelled => None,
            Self::Saved => Some(Notice::success(
                language.text(Key::DiagnosticExportSucceeded),
            )),
            Self::Failed => Some(Notice::error(language.text(Key::DiagnosticExportFailed))),
        }
    }
}

/// What the ZIP holds: the snapshot, and the local log when there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contents {
    pub snapshot: DiagnosticSnapshot,
    pub logs: Option<PathBuf>,
}

impl Contents {
    /// The day the shell shows, now.
    fn of_today(cx: &App) -> Self {
        let today = cx.global::<Today>();
        Self {
            snapshot: today.diagnostic_snapshot(runtime()),
            logs: today.log_directory().map(ToOwned::to_owned),
        }
    }

    /// A start that failed with `settings` read, as `ObserveInitializationAsync` leaves the
    /// snapshot: no day, so no summary.
    pub fn of_failed_start(settings: Option<&WorkTimeSettings>, logs: Option<PathBuf>) -> Self {
        Self {
            snapshot: create_snapshot(
                DiagnosticInitializationState::Failed,
                settings,
                None,
                None,
                Timestamp::now(),
                runtime(),
            ),
            logs,
        }
    }

    /// Writes the ZIP at `destination`. .NET dates the records it keeps by the system clock,
    /// even in a demo.
    pub fn write(
        &self,
        destination: &Path,
    ) -> Result<DiagnosticExportResult, DiagnosticExportError> {
        DiagnosticExport::new(&SystemClock, &final_paths::FINAL_PATHS).export(
            destination,
            self.logs.as_deref(),
            &self.snapshot,
        )
    }
}

/// An export is under way: the tray command does nothing until it ends.
#[derive(Default)]
struct Exporting(bool);

impl Global for Exporting {}

/// Asks where to save the ZIP of the day, writes it there, then shows the outcome in the flyout.
pub fn export(cx: &mut App) {
    let language = cx.try_global::<Language>().copied().unwrap_or_default();
    export_with(
        Contents::of_today,
        move |outcome, cx| {
            if let Some(notice) = outcome.notice(language) {
                shell::open_flyout_with(notice, cx);
            }
        },
        cx,
    );
}

/// Asks where to save the ZIP, writes `contents` there, then hands the outcome to `done`. The
/// dialog and the archive both run off the main thread, so the windows go on meanwhile. Whether
/// it started: while another export runs, nothing happens and `done` is never called.
pub fn export_with(
    contents: impl FnOnce(&App) -> Contents + 'static,
    done: impl FnOnce(Outcome, &mut App) + 'static,
    cx: &mut App,
) -> bool {
    let exporting = cx.default_global::<Exporting>();
    if exporting.0 {
        return false;
    }
    exporting.0 = true;
    let language = cx.try_global::<Language>().copied().unwrap_or_default();
    let file_type = language.text(Key::DiagnosticZipFileType).to_owned();
    cx.spawn(async move |cx| {
        let outcome =
            match file_dialogs::pick_save(SUGGESTED_FILE_NAME.to_owned(), file_type, "zip", cx)
                .await
            {
                Ok(Some(destination)) => {
                    let contents = cx.update(|cx| contents(cx));
                    save(destination, contents, cx).await
                }
                Ok(None) => Outcome::Cancelled,
                Err(error) => failed(&error, cx),
            };
        cx.update(|cx| {
            cx.global_mut::<Exporting>().0 = false;
            done(outcome, cx);
        });
    })
    .detach();
    true
}

async fn save(destination: PathBuf, contents: Contents, cx: &mut AsyncApp) -> Outcome {
    let saved = cx
        .background_executor()
        .spawn(async move { contents.write(&destination) })
        .await;
    match saved {
        Ok(_) => Outcome::Saved,
        Err(error) => failed(&error, cx),
    }
}

/// Logs the failure as `window_command.failed`, the event .NET logs it as. A demo, or a start
/// that failed, has no log.
fn failed(error: &impl Display, cx: &mut AsyncApp) -> Outcome {
    cx.update(|cx| {
        let live = if cx.has_global::<Today>() {
            cx.global_mut::<Today>().live()
        } else {
            None
        };
        match live {
            Some(host) => host.log_failure("window_command.failed", error),
            None => log::error!("cannot export the diagnostic ZIP: {error}"),
        }
    });
    Outcome::Failed
}

#[cfg(windows)]
mod final_paths {
    use std::fs::File;
    use std::path::Path;

    use worktime_core::diagnostics::FinalPaths;
    use worktime_windows::diagnostics::{directory_is_final, file_is_in};

    /// Asks Windows where the log directory and each log file really are.
    pub struct WindowsFinalPaths;

    pub static FINAL_PATHS: WindowsFinalPaths = WindowsFinalPaths;

    impl FinalPaths for WindowsFinalPaths {
        fn directory_is_final(&self, directory: &Path) -> bool {
            directory_is_final(directory)
        }

        fn file_is_in(&self, file: &File, directory: &Path) -> bool {
            file_is_in(file, directory)
        }
    }
}

#[cfg(not(windows))]
mod final_paths {
    use worktime_core::diagnostics::NoFinalPathCheck;

    pub static FINAL_PATHS: NoFinalPathCheck = NoFinalPathCheck;
}

/// What `DiagnosticSnapshotFactory` reads of the process and the system. The Rust app runs on
/// no .NET runtime: its version is all zeros.
fn runtime() -> DiagnosticRuntimeSnapshot {
    let process_architecture = architecture(std::env::consts::ARCH);
    #[cfg(windows)]
    let (operating_system_version, operating_system_architecture) = (
        numeric_version(worktime_windows::diagnostics::operating_system_version()),
        worktime_windows::diagnostics::native_machine()
            .map_or(process_architecture, machine_architecture),
    );
    #[cfg(not(windows))]
    let (operating_system_version, operating_system_architecture) =
        (numeric_version([0; 4]), process_architecture);
    DiagnosticRuntimeSnapshot {
        application_version: application_version(env!("CARGO_PKG_VERSION")),
        operating_system_version,
        dot_net_version: numeric_version([0; 4]),
        process_architecture,
        operating_system_architecture,
    }
}

/// `1.4.2` as .NET's `1.4.2.0`; a part that is not a number counts as 0.
fn application_version(text: &str) -> DiagnosticNumericVersion {
    let mut parts = text
        .split(['.', '-', '+'])
        .map(|part| part.parse().unwrap_or(0));
    numeric_version([(); 4].map(|()| parts.next().unwrap_or(0)))
}

fn numeric_version(parts: [u32; 4]) -> DiagnosticNumericVersion {
    let [major, minor, build, revision] = parts.map(|part| i32::try_from(part).unwrap_or(0));
    DiagnosticNumericVersion {
        major,
        minor,
        build,
        revision,
    }
}

/// The process architecture, from the target Rust compiled it for.
fn architecture(target: &str) -> DiagnosticArchitecture {
    match target {
        "x86" => DiagnosticArchitecture::X86,
        "x86_64" => DiagnosticArchitecture::X64,
        "arm" => DiagnosticArchitecture::Arm,
        "aarch64" => DiagnosticArchitecture::Arm64,
        "wasm32" => DiagnosticArchitecture::Wasm,
        "s390x" => DiagnosticArchitecture::S390x,
        "loongarch64" => DiagnosticArchitecture::LoongArch64,
        _ => DiagnosticArchitecture::Unknown,
    }
}

/// The architecture of an `IMAGE_FILE_MACHINE`.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only Windows names its machine outside the tests")
)]
fn machine_architecture(machine: u16) -> DiagnosticArchitecture {
    match machine {
        0x014c => DiagnosticArchitecture::X86,
        0x8664 => DiagnosticArchitecture::X64,
        0x01c4 => DiagnosticArchitecture::Arm,
        0xaa64 => DiagnosticArchitecture::Arm64,
        _ => DiagnosticArchitecture::Unknown,
    }
}

#[cfg(test)]
mod tests;
