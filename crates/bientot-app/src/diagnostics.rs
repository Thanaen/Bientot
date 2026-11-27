//! « Exporter le diagnostic… » of the tray and of the startup failure, ported from
//! `ExportDiagnosticsAsync` of `App.xaml.cs`: the user picks where to save the ZIP, the archive
//! of the snapshot and of the local log is written off the main thread, then the window that
//! asked says whether it was saved.

use std::fmt::Display;
use std::io;
use std::path::{Path, PathBuf};

use bientot_core::diagnostics::{
    DiagnosticArchitecture, DiagnosticExport, DiagnosticExportError, DiagnosticExportResult,
    DiagnosticInitializationState, DiagnosticInstallKind, DiagnosticName, DiagnosticNumericVersion,
    DiagnosticRuntimeSnapshot, DiagnosticSessionType, DiagnosticSnapshot, DiagnosticSystemSnapshot,
    create_snapshot,
};
use bientot_domain::configuration::Settings;
use bientot_domain::time::SystemClock;
use bientot_platform::system::{InstallKind, SessionType, SystemInfo};
use gpui::{App, AsyncApp, Global};
use jiff::Timestamp;

use crate::host::Today;
use crate::platform::{self, Backends, lock_reporting_name};
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
        let backends = cx.try_global::<Backends>().copied().unwrap_or_default();
        Self {
            snapshot: DiagnosticSnapshot {
                system: platform::system().map(|system| system_snapshot(&system, backends)),
                ..today.diagnostic_snapshot(runtime())
            },
            logs: today.log_directory().map(ToOwned::to_owned),
        }
    }

    /// A start that failed with `settings` read, as `ObserveInitializationAsync` leaves the
    /// snapshot: no day, so no summary, and no backend named.
    pub fn of_failed_start(settings: Option<&Settings>, logs: Option<PathBuf>) -> Self {
        let failed = create_snapshot(
            DiagnosticInitializationState::Failed,
            settings,
            None,
            None,
            Timestamp::now(),
            runtime(),
        );
        Self {
            snapshot: DiagnosticSnapshot {
                system: platform::system()
                    .map(|system| system_snapshot(&system, Backends::default())),
                ..failed
            },
            logs,
        }
    }

    /// Writes the ZIP at `destination`. .NET dates the records it keeps by the system clock,
    /// even in a demo.
    pub fn write(
        &self,
        destination: &Path,
    ) -> Result<DiagnosticExportResult, DiagnosticExportError> {
        DiagnosticExport::new(&SystemClock, platform::final_paths()).export(
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
            match platform::pick_save(SUGGESTED_FILE_NAME.to_owned(), file_type, "zip", cx).await {
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
        if let Some(host) = live {
            host.log_failure("window_command.failed", error);
        }
    });
    Outcome::Failed
}

/// What `DiagnosticSnapshotFactory` reads of the process and the system. The Rust app runs on
/// no .NET runtime: its version is all zeros.
fn runtime() -> DiagnosticRuntimeSnapshot {
    let process_architecture = architecture(std::env::consts::ARCH);
    DiagnosticRuntimeSnapshot {
        application_version: application_version(env!("CARGO_PKG_VERSION")),
        operating_system_version: numeric_version(platform::operating_system_version()),
        dot_net_version: numeric_version([0; 4]),
        process_architecture,
        operating_system_architecture: platform::native_machine()
            .map_or(process_architecture, machine_architecture),
    }
}

/// What the export names of `system` and of the `backends` in use, and whether the desktop
/// reports locks. A name that is not a [`DiagnosticName`] is left out.
fn system_snapshot(system: &impl SystemInfo, backends: Backends) -> DiagnosticSystemSnapshot {
    let description = system.describe();
    let name = |text: &String| DiagnosticName::new(text);
    DiagnosticSystemSnapshot {
        install_kind: match system.install_kind() {
            InstallKind::Store => DiagnosticInstallKind::Store,
            InstallKind::Direct => DiagnosticInstallKind::Direct,
            InstallKind::Portable => DiagnosticInstallKind::Portable,
            InstallKind::Flatpak => DiagnosticInstallKind::Flatpak,
            InstallKind::AppImage => DiagnosticInstallKind::AppImage,
            InstallKind::Development => DiagnosticInstallKind::Development,
        },
        distribution: name(&description.distribution),
        distribution_version: description.distribution_version.as_ref().and_then(name),
        desktops: description.desktops.iter().filter_map(name).collect(),
        session_type: match description.session_type {
            Some(SessionType::X11) => DiagnosticSessionType::X11,
            Some(SessionType::Wayland) => DiagnosticSessionType::Wayland,
            None => DiagnosticSessionType::Unknown,
        },
        backends: backends.named(),
        lock_reporting: backends.lock_reporting.map(lock_reporting_name),
    }
}

/// An I/O error's kind as the diagnostic log writes it, from a fixed list: the error's text can
/// name the user's session, bus or folder.
pub fn error_kind(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::AddrInUse => "address_in_use",
        io::ErrorKind::InvalidInput => "invalid_input",
        _ => "other",
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
