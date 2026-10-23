//! The « Données » page: exports the folder's history and settings to one file, or imports a
//! file exported on another PC, which replaces them once WorkTime restarts, and shows the copy
//! the last import saved. Nothing here knows GPUI; [`crate::transfer`] does the work.

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
}

/// A row of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Export,
    Import,
    /// The copy of the data the last import replaced.
    Backup,
}

/// What a row's button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Run(Action),
    /// Shows the row's file in File Explorer.
    Reveal,
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
    /// The open dialog runs, or the chosen file is read.
    Reading,
    /// The dialog asks to replace this PC's data with the file's.
    Confirming(Box<Chosen>),
    /// The file is staged and WorkTime restarts.
    Importing,
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
    Refused(Refusal),
    ImportFailed,
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
    pub button: Button,
    pub title: &'static str,
    pub description: String,
    pub tone: RowTone,
    pub label: &'static str,
    pub enabled: bool,
    /// How the row's last action went.
    pub status: Option<String>,
    pub error: Option<&'static str>,
    /// The link under the status that shows the exported file in File Explorer.
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

    /// Starts exporting or importing: the data folder it works on, `None` when it may not start
    /// now.
    pub fn start(&mut self, action: Action) -> Option<DataDirectory> {
        if !self.ready() {
            return None;
        }
        self.step = match action {
            Action::Export => Step::Exporting,
            Action::Import => Step::Reading,
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
        if self.step != Step::Reading {
            return;
        }
        self.step = Step::Idle;
        match read {
            None => {}
            Some(Ok(chosen)) => self.step = Step::Confirming(Box::new(chosen)),
            Some(Err(refusal)) => self.ending = Some(Ending::Refused(refusal)),
        }
    }

    /// « Annuler », or Escape: nothing changes.
    pub fn cancel(&mut self) {
        if matches!(self.step, Step::Confirming(_)) {
            self.step = Step::Idle;
        }
    }

    /// « Remplacer et redémarrer »: the file to stage before the restart, and the data folder
    /// it replaces.
    pub fn confirm(&mut self) -> Option<(PathBuf, DataDirectory)> {
        if !matches!(self.step, Step::Confirming(_)) {
            return None;
        }
        let directory = self.directory.clone()?;
        let Step::Confirming(chosen) = std::mem::replace(&mut self.step, Step::Importing) else {
            unreachable!();
        };
        Some((chosen.file, directory))
    }

    /// Staging the file, or the restart, failed: this PC's data has not changed.
    pub fn import_failed(&mut self) {
        if self.step == Step::Importing {
            self.step = Step::Idle;
            self.ending = Some(Ending::ImportFailed);
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
        let Step::Confirming(chosen) = &self.step else {
            return None;
        };
        let language = self.language;
        let mut paragraphs = vec![match chosen.contents.days {
            Some((first, last)) => language.format_dates(
                Key::SettingsImportDialogPeriodFormat,
                &[chosen.exported_on, first, last],
            ),
            None => {
                language.format_dates(Key::SettingsImportDialogEmptyFormat, &[chosen.exported_on])
            }
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
            Step::Reading => (Some(Key::SettingsImportReading), true),
            Step::Importing => (Some(Key::SettingsImportRestarting), true),
            Step::Idle | Step::Confirming(_) => (None, false),
        };
        let error = |row: Row| {
            let key = match (&self.ending, row) {
                (Some(Ending::Exported(Exported::Failed)), Row::Export) => {
                    Key::SettingsExportFailed
                }
                (Some(Ending::Refused(refusal)), Row::Import) => match refusal {
                    Refusal::NotATransfer => Key::SettingsImportNotATransfer,
                    Refusal::NewerVersion => Key::SettingsImportNewer,
                    Refusal::Unreadable => Key::SettingsImportReadFailed,
                },
                (Some(Ending::ImportFailed), Row::Import) => Key::SettingsImportFailed,
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
                button: Button::Run(Action::Export),
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
                button: Button::Run(Action::Import),
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
                button: Button::Reveal,
                title: language.text(Key::SettingsBackupTitle),
                description: language
                    .format_dates(Key::SettingsBackupDescriptionFormat, &[backup.saved_on]),
                tone,
                label: show,
                enabled: self.available(),
                status: None,
                error: None,
                reveal: None,
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
