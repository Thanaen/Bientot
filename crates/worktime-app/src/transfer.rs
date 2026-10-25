//! Moving WorkTime to another PC: the export and the import of « Paramètres › Données », the
//! restart that applies an import, and the start that applies it. The file is
//! [`worktime_store::transfer`]'s; the dialogs and the file work run off the main thread.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AsyncApp};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use worktime_store::data_directory::DataDirectory;
use worktime_store::settings::SettingsStore;
use worktime_store::transfer::{self, EXTENSION, EXTENSIONS, TransferError};

use crate::file_dialogs;
use crate::host::Today;
use crate::live::Announcement;
use crate::settings::data::{Backup, Chosen, Exported, Refusal};
use crate::strings::{Key, Language};

/// How long a restarted WorkTime waits for the one that started it to quit.
const RESTART_WAIT: Duration = Duration::from_secs(30);

/// The data folder of the day the shell shows, `None` in a demo.
pub fn data_directory(cx: &App) -> Option<DataDirectory> {
    cx.try_global::<Today>()
        .and_then(Today::data_directory)
        .cloned()
}

/// Asks where to save the export of `directory`, writes it there, then hands `done` how it
/// ended, or `None` when the dialog was cancelled.
pub fn export(
    directory: DataDirectory,
    done: impl FnOnce(Option<Exported>, &mut App) + 'static,
    cx: &mut App,
) {
    let file_type = file_type(cx);
    let suggested = format!("Bientot-{}.{EXTENSION}", jiff::Zoned::now().date());
    cx.spawn(async move |cx| {
        let saved = match file_dialogs::pick_save(suggested, file_type, EXTENSION, cx).await {
            Ok(None) => None,
            Ok(Some(destination)) => Some(
                cx.background_executor()
                    .spawn(async move {
                        transfer::export(&directory, &destination, Timestamp::now())
                            .map(|_| destination)
                    })
                    .await
                    .map_err(anyhow::Error::from),
            ),
            Err(error) => Some(Err(error)),
        };
        let exported = saved.map(|saved| match saved {
            Ok(destination) => Exported::Saved(destination),
            Err(error) => {
                failed(&error, cx);
                Exported::Failed
            }
        });
        cx.update(|cx| done(exported, cx));
    })
    .detach();
}

/// Asks which file to import, then hands `done` what [`read_import`] reads of it, or `None` when
/// the dialog was cancelled.
pub fn choose_import(
    directory: DataDirectory,
    done: impl FnOnce(Option<Result<Chosen, Refusal>>, &mut App) + 'static,
    cx: &mut App,
) {
    let file_type = file_type(cx);
    cx.spawn(async move |cx| {
        let read = match file_dialogs::pick_open(file_type, &EXTENSIONS, cx).await {
            Ok(None) => None,
            Ok(Some(file)) => Some(inspect(directory, file, cx).await),
            Err(error) => {
                failed(&error, cx);
                Some(Err(Refusal::Unreadable))
            }
        };
        cx.update(|cx| done(read, cx));
    })
    .detach();
}

/// Reads `file`, here the copy the last import saved, and the days the ledger of `directory`
/// holds off the main thread, then hands `done` what the import would replace.
pub fn read_import(
    directory: DataDirectory,
    file: PathBuf,
    done: impl FnOnce(Option<Result<Chosen, Refusal>>, &mut App) + 'static,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let read = inspect(directory, file, cx).await;
        cx.update(|cx| done(Some(read), cx));
    })
    .detach();
}

async fn inspect(
    directory: DataDirectory,
    file: PathBuf,
    cx: &mut AsyncApp,
) -> Result<Chosen, Refusal> {
    let inspected = cx
        .background_executor()
        .spawn(async move {
            let contents = transfer::inspect(&file)?;
            // This PC's days only add a line to the dialog.
            let this_pc = transfer::ledger_days(&directory).unwrap_or_else(|error| {
                log::warn!("cannot read the days of the ledger: {error}");
                None
            });
            Ok(Chosen {
                exported_on: contents.exported_at.to_zoned(TimeZone::system()).date(),
                file,
                contents,
                this_pc,
            })
        })
        .await;
    inspected.map_err(|error: TransferError| {
        if matches!(error, TransferError::Io(_) | TransferError::Sqlite(_)) {
            failed(&error, cx);
        }
        Refusal::from(&error)
    })
}

/// The copy the last import of `directory` saved, and the local day it was saved.
pub fn latest_backup(directory: &DataDirectory) -> Option<Backup> {
    let file = transfer::latest_backup(directory)?;
    let modified = std::fs::metadata(&file)
        .and_then(|metadata| metadata.modified())
        .ok()?;
    let saved_on = Timestamp::try_from(modified)
        .ok()?
        .to_zoned(TimeZone::system())
        .date();
    Some(Backup { file, saved_on })
}

/// Stages `file` in `directory`, then restarts WorkTime, whose start applies it. `failed` runs
/// when either step fails: this PC's data is then unchanged, or the import waits for the next
/// start.
pub fn import(
    directory: DataDirectory,
    file: PathBuf,
    failed_to_import: impl FnOnce(&mut App) + 'static,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let staged = cx
            .background_executor()
            .spawn({
                let directory = directory.clone();
                async move { transfer::stage(&file, &directory) }
            })
            .await;
        if let Err(error) = staged {
            failed(&error, cx);
            cx.update(failed_to_import);
            return;
        }
        let restarted = cx.update(restart);
        if let Err(error) = restarted {
            // The next start applies the import, unless it is undone first.
            std::fs::remove_file(directory.pending_import_file()).ok();
            failed(&error, cx);
            cx.update(failed_to_import);
        }
    })
    .detach();
}

/// Starts this WorkTime again with the same arguments, then quits. The new one waits for this
/// one to end before it claims the data folder.
fn restart(cx: &mut App) -> std::io::Result<()> {
    std::process::Command::new(std::env::current_exe()?)
        .args(crate::launch::restart_arguments(false))
        .arg(crate::launch::RESTARTED_FROM_ARGUMENT)
        .arg(std::process::id().to_string())
        .spawn()?;
    cx.quit();
    Ok(())
}

/// In a WorkTime a restart started, waits for the one that started it to quit, so that it gives
/// up the data folder and its ledger.
pub fn wait_for_restart(restarted_from: Option<u32>) {
    let Some(id) = restarted_from else {
        return;
    };
    #[cfg(windows)]
    if !worktime_windows::process::wait_for_exit(id, RESTART_WAIT) {
        log::warn!("the WorkTime that restarted this one still runs");
    }
    #[cfg(not(windows))]
    let _ = (id, RESTART_WAIT);
}

/// After a restart for an import that gave up waiting, the WorkTime that started it still owns
/// the folder: whether the import staged in `directory` now waits for the next start, and the
/// language of the settings it would replace, to tell the user so.
pub fn import_left_waiting(
    restarted_from: Option<u32>,
    directory: &DataDirectory,
    forced: Option<Language>,
) -> Option<Language> {
    if restarted_from.is_none() || !directory.pending_import_file().exists() {
        return None;
    }
    let saved = SettingsStore::new(directory.settings_file())
        .ok()
        .and_then(|store| store.peek());
    Some(Language::resolve(
        forced,
        saved
            .as_ref()
            .and_then(|settings| settings.language_override.as_deref()),
    ))
}

/// Applies the import staged in `directory`, before anything opens its ledger: what the first
/// flyout then tells, `None` when nothing was staged.
pub fn apply_staged(directory: &DataDirectory) -> Option<Announcement> {
    match transfer::apply_staged(directory, Timestamp::now(), &TimeZone::system()) {
        Ok(None) => None,
        Ok(Some(applied)) => {
            if let Some(backup) = applied.backup {
                log::info!("import applied, previous data in {}", backup.display());
            }
            Some(Announcement::Imported)
        }
        Err(error) => {
            log::error!("cannot apply the import: {error}");
            Some(Announcement::ImportFailed)
        }
    }
}

fn file_type(cx: &App) -> String {
    let language = cx.try_global::<Language>().copied().unwrap_or_default();
    language.text(Key::TransferFileType).to_owned()
}

/// Logs the failure as `window_command.failed`, as the diagnostic export does; a demo has no
/// log.
fn failed(error: &dyn std::fmt::Display, cx: &mut AsyncApp) {
    cx.update(|cx| {
        let live = if cx.has_global::<Today>() {
            cx.global_mut::<Today>().live()
        } else {
            None
        };
        match live {
            Some(host) => host.log_failure("window_command.failed", &error),
            None => log::error!("cannot move the data: {error}"),
        }
    });
}

#[cfg(test)]
mod tests;
