//! The « Données » page: exports the folder's history and settings to one file, or imports a
//! file exported on another PC, which replaces them once WorkTime restarts, and restores the
//! copy the last import saved the same way. Nothing here knows GPUI; [`crate::transfer`] does
//! the work.

use std::path::{Path, PathBuf};

use jiff::civil::Date;
use worktime_store::data_directory::DataDirectory;
use worktime_store::transfer::{Contents, TransferError};

use crate::settings::RowTone;
use crate::strings::{Key, Language};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Export,
    Import,
    /// Imports the copy the last import saved.
    Restore,
}

/// A row of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Export,
    Import,
    /// The copy of the data the last import replaced.
    Backup,
}

/// How an export ended, once the save dialog chose where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exported {
    Saved(PathBuf),
    Failed,
}

/// What the page is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Idle,
    /// The save dialog or the export runs.
    Exporting,
    /// The open dialog runs, or the chosen file is read: by [`Action::Import`] or
    /// [`Action::Restore`], as are the next two steps.
    Reading(Action),
    /// The dialog asks to replace this PC's data with the file's.
    Confirming(Action, Box<Chosen>),
    /// The file is staged and WorkTime restarts.
    Importing(Action),
}

/// A file read for an import, and the data it would replace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chosen {
    pub file: PathBuf,
    pub contents: Contents,
    /// The local day of the export.
    pub exported_on: Date,
    /// The first and the last workday this PC holds.
    pub this_pc: Option<(Date, Date)>,
}

/// How the last action ended, until the next one starts.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ending {
    Exported(Exported),
    Refused(Action, Refusal),
    ImportFailed(Action),
}

/// Why the chosen file cannot be imported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    NotATransfer,
    NewerVersion,
    Unreadable,
}

impl From<&TransferError> for Refusal {
    fn from(error: &TransferError) -> Self {
        match error {
            TransferError::NotATransfer => Self::NotATransfer,
            TransferError::NewerVersion => Self::NewerVersion,
            TransferError::Io(_) | TransferError::Sqlite(_) => Self::Unreadable,
        }
    }
}

/// The copy the last import saved before it replaced the data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backup {
    pub file: PathBuf,
    /// The local day it was saved.
    pub saved_on: Date,
}

pub struct DataPage {
    language: Language,
    /// The data folder moved, `None` in a demo, which has no data of its own.
    directory: Option<DataDirectory>,
    backup: Option<Backup>,
    step: Step,
    ending: Option<Ending>,
}

/// What the page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataView {
    pub title: &'static str,
    /// Under the title: what runs, or why nothing can run.
    pub status: Option<&'static str>,
    pub busy: bool,
    pub heading: &'static str,
    /// Export, import, then the copy of the last import when there is one.
    pub rows: Vec<ActionRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRow {
    pub row: Row,
    pub action: Action,
    pub title: &'static str,
    pub description: String,
    pub tone: RowTone,
    pub label: &'static str,
    pub enabled: bool,
    /// How the row's last action went.
    pub status: Option<String>,
    pub error: Option<&'static str>,
    /// The link under the status that shows the row's file in File Explorer.
    pub reveal: Option<&'static str>,
}

/// The dialog that asks before an import replaces this PC's data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportDialog {
    pub title: &'static str,
    /// When the file was exported and the days it holds, then the days this PC holds.
    pub paragraphs: Vec<String>,
    /// The days this PC would no longer show.
    pub loss: Option<String>,
    pub message: &'static str,
    pub cancel: &'static str,
    pub confirm: &'static str,
}

impl DataPage {
    pub fn new(
        language: Language,
        directory: Option<DataDirectory>,
        backup: Option<Backup>,
    ) -> Self {
        Self {
            language,
            directory,
            backup,
            step: Step::Idle,
            ending: None,
        }
    }

    /// The texts follow the language chosen since, as in the onboarding.
    pub fn set_language(&mut self, language: Language) {
        self.language = language;
    }

    fn available(&self) -> bool {
        self.directory.is_some()
    }

    /// Whether an export or an import may start now.
    pub fn ready(&self) -> bool {
        self.available() && self.step == Step::Idle
    }

    /// The copy « Restaurer… » imports, when the last import saved one.
    pub fn backup(&self) -> Option<&Backup> {
        self.backup.as_ref()
    }

    /// Starts exporting, importing or restoring: the data folder it works on, `None` when it may
    /// not start now.
    pub fn start(&mut self, action: Action) -> Option<DataDirectory> {
        if !self.ready() || (action == Action::Restore && self.backup.is_none()) {
            return None;
        }
        self.step = match action {
            Action::Export => Step::Exporting,
            Action::Import | Action::Restore => Step::Reading(action),
        };
        self.ending = None;
        self.directory.clone()
    }

    /// The export ended, `None` once the save dialog was cancelled.
    pub fn exported(&mut self, exported: Option<Exported>) {
        if self.step != Step::Exporting {
            return;
        }
        self.step = Step::Idle;
        self.ending = exported.map(Ending::Exported);
    }

    /// The chosen file was read: the dialog asks to import it. `None` once the open dialog was
    /// cancelled.
    pub fn read(&mut self, read: Option<Result<Chosen, Refusal>>) {
        let Step::Reading(action) = self.step else {
            return;
        };
        self.step = Step::Idle;
        match read {
            None => {}
            Some(Ok(chosen)) => self.step = Step::Confirming(action, Box::new(chosen)),
            Some(Err(refusal)) => self.ending = Some(Ending::Refused(action, refusal)),
        }
    }

    /// « Annuler », or Escape: nothing changes.
    pub fn cancel(&mut self) {
        if matches!(self.step, Step::Confirming(..)) {
            self.step = Step::Idle;
        }
    }

    /// « Remplacer et redémarrer »: the file to stage before the restart, and the data folder
    /// it replaces.
    pub fn confirm(&mut self) -> Option<(PathBuf, DataDirectory)> {
        let Step::Confirming(action, _) = self.step else {
            return None;
        };
        let directory = self.directory.clone()?;
        let Step::Confirming(_, chosen) =
            std::mem::replace(&mut self.step, Step::Importing(action))
        else {
            unreachable!();
        };
        Some((chosen.file, directory))
    }

    /// Staging the file, or the restart, failed: this PC's data has not changed.
    pub fn import_failed(&mut self) {
        if let Step::Importing(action) = self.step {
            self.step = Step::Idle;
            self.ending = Some(Ending::ImportFailed(action));
        }
    }

    /// The file « Afficher dans l'Explorateur » shows on `row`.
    pub fn revealed(&self, row: Row) -> Option<&Path> {
        match (row, &self.ending) {
            (Row::Backup, _) => self.backup.as_ref().map(|backup| backup.file.as_path()),
            (Row::Export, Some(Ending::Exported(Exported::Saved(path)))) => Some(path),
            _ => None,
        }
    }

    pub fn dialog(&self) -> Option<ImportDialog> {
        let Step::Confirming(action, chosen) = &self.step else {
            return None;
        };
        let language = self.language;
        // A copy was saved, not exported.
        let (period, empty) = if *action == Action::Restore {
            (
                Key::SettingsRestoreDialogPeriodFormat,
                Key::SettingsRestoreDialogEmptyFormat,
            )
        } else {
            (
                Key::SettingsImportDialogPeriodFormat,
                Key::SettingsImportDialogEmptyFormat,
            )
        };
        let mut paragraphs = vec![match chosen.contents.days {
            Some((first, last)) => {
                language.format_dates(period, &[chosen.exported_on, first, last])
            }
            None => language.format_dates(empty, &[chosen.exported_on]),
        }];
        if let Some((first, last)) = chosen.this_pc {
            paragraphs
                .push(language.format_dates(Key::SettingsImportDialogThisPcFormat, &[first, last]));
        }
        let loss = match (chosen.contents.days, chosen.this_pc) {
            (Some((file_first, _)), Some((pc_first, _))) if pc_first < file_first => Some(
                language
                    .text(Key::SettingsImportDialogLossOutside)
                    .to_owned(),
            ),
            (Some((_, file_last)), Some((_, pc_last))) if pc_last > file_last => {
                Some(language.format_dates(Key::SettingsImportDialogLossFormat, &[file_last]))
            }
            (None, Some(_)) => Some(language.text(Key::SettingsImportDialogLossAll).to_owned()),
            _ => None,
        };
        Some(ImportDialog {
            title: language.text(Key::SettingsImportDialogTitle),
            paragraphs,
            loss,
            message: language.text(Key::SettingsImportDialogMessage),
            cancel: language.text(Key::SettingsImportCancel),
            confirm: language.text(Key::SettingsImportConfirm),
        })
    }

    pub fn view(&self) -> DataView {
        let language = self.language;
        let (status, busy) = match self.step {
            _ if !self.available() => (Some(Key::SettingsTransferDemo), false),
            Step::Exporting => (Some(Key::SettingsExporting), true),
            Step::Reading(Action::Restore) => (Some(Key::SettingsRestoreReading), true),
            Step::Reading(_) => (Some(Key::SettingsImportReading), true),
            Step::Importing(_) => (Some(Key::SettingsImportRestarting), true),
            Step::Idle | Step::Confirming(..) => (None, false),
        };
        let error = |row: Row| {
            let key = match (&self.ending, row) {
                (Some(Ending::Exported(Exported::Failed)), Row::Export) => {
                    Key::SettingsExportFailed
                }
                (Some(Ending::Refused(Action::Import, refusal)), Row::Import) => match refusal {
                    Refusal::NotATransfer => Key::SettingsImportNotATransfer,
                    Refusal::NewerVersion => Key::SettingsImportNewer,
                    Refusal::Unreadable => Key::SettingsImportReadFailed,
                },
                (Some(Ending::Refused(Action::Restore, refusal)), Row::Backup) => match refusal {
                    Refusal::NewerVersion => Key::SettingsImportNewer,
                    Refusal::NotATransfer | Refusal::Unreadable => Key::SettingsRestoreReadFailed,
                },
                (Some(Ending::ImportFailed(Action::Import)), Row::Import) => {
                    Key::SettingsImportFailed
                }
                (Some(Ending::ImportFailed(Action::Restore)), Row::Backup) => {
                    Key::SettingsRestoreFailed
                }
                _ => return None,
            };
            Some(language.text(key))
        };
        let show = language.text(Key::SettingsShowInExplorer);
        let tone = if self.available() {
            RowTone::Normal
        } else {
            RowTone::Unavailable
        };
        let exported = self.revealed(Row::Export);
        let mut rows = vec![
            ActionRow {
                row: Row::Export,
                action: Action::Export,
                title: language.text(Key::SettingsExportTitle),
                description: language.text(Key::SettingsExportDescription).to_owned(),
                tone,
                label: language.text(Key::SettingsExportAction),
                enabled: self.ready(),
                status: exported.map(|path| {
                    let name = path.file_name().unwrap_or(path.as_os_str());
                    language.format(Key::SettingsExportedFormat, &[&name.to_string_lossy()])
                }),
                error: error(Row::Export),
                reveal: exported.map(|_| show),
            },
            ActionRow {
                row: Row::Import,
                action: Action::Import,
                title: language.text(Key::SettingsImportTitle),
                description: language.text(Key::SettingsImportDescription).to_owned(),
                tone,
                label: language.text(Key::SettingsImportAction),
                enabled: self.ready(),
                status: None,
                error: error(Row::Import),
                reveal: None,
            },
        ];
        if let Some(backup) = &self.backup {
            rows.push(ActionRow {
                row: Row::Backup,
                action: Action::Restore,
                title: language.text(Key::SettingsBackupTitle),
                description: language
                    .format_dates(Key::SettingsBackupDescriptionFormat, &[backup.saved_on]),
                tone,
                label: language.text(Key::SettingsRestoreAction),
                enabled: self.ready(),
                status: None,
                error: error(Row::Backup),
                reveal: self.available().then_some(show),
            });
        }
        DataView {
            title: language.text(Key::SettingsDataTitle),
            status: status.map(|key| language.text(key)),
            busy,
            heading: language.text(Key::SettingsTransferGroup),
            rows,
        }
    }
}

#[cfg(test)]
mod tests;
