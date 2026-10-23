//! Why WorkTime did not start, as its window tells it: the error, and « Exporter le
//! diagnostic… », which .NET offers in its main window once `ObserveInitializationAsync` saw the
//! initialization fail. The ZIP needs no day: it holds the settings if they still read and the
//! log of the earlier runs.

use gpui::SharedString;
use worktime_store::data_directory::DataDirectory;
use worktime_store::settings::SettingsStore;

use crate::demo::FolderRefusal;
use crate::diagnostics::{Contents, Outcome};
use crate::strings::{Key, Language};
use crate::widgets::Notice;

/// Where the export stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Export {
    /// Nothing asked yet, or the save dialog closed.
    Ready,
    Running,
    Ended(Outcome),
}

#[derive(Debug)]
pub struct StartupFailure {
    language: Language,
    message: Key,
    detail: Vec<SharedString>,
    contents: Contents,
    export: Export,
}

/// A button of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub label: &'static str,
    /// What a screen reader names it.
    pub name: &'static str,
    pub enabled: bool,
}

/// What the window shows, every text resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailurePage<'a> {
    pub title: &'static str,
    pub heading: &'static str,
    pub message: &'static str,
    pub details_header: &'static str,
    /// The error, then each cause.
    pub detail: &'a [SharedString],
    /// The export's outcome, until the next export.
    pub notice: Option<Notice>,
    pub privacy: &'static str,
    pub export: Action,
    /// Quits WorkTime.
    pub close: &'static str,
}

impl StartupFailure {
    /// A start on `directory` that failed with `error`. The texts are in `forced`, else in the
    /// language of the settings if they still read. A demo refused its folder says why. Nothing
    /// is written to `directory`: invalid settings stay where they are, for the ZIP's reader to
    /// find.
    pub fn new(
        error: &anyhow::Error,
        directory: Option<&DataDirectory>,
        forced: Option<Language>,
    ) -> Self {
        let settings = directory
            .and_then(|directory| SettingsStore::new(directory.settings_file()).ok())
            .and_then(|store| store.peek());
        Self {
            language: Language::resolve(
                forced,
                settings
                    .as_ref()
                    .and_then(|settings| settings.language_override.as_deref()),
            ),
            message: error
                .chain()
                .find_map(|cause| cause.downcast_ref::<FolderRefusal>())
                .map_or(Key::StartupFailureMessage, FolderRefusal::message),
            detail: detail(error),
            contents: Contents::of_failed_start(
                settings.as_ref(),
                directory.map(DataDirectory::log_directory),
            ),
            export: Export::Ready,
        }
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// What « Exporter le diagnostic… » saves.
    pub fn contents(&self) -> &Contents {
        &self.contents
    }

    /// The export started; it runs until [`Self::exported`].
    pub fn start_export(&mut self) {
        self.export = Export::Running;
    }

    pub fn exported(&mut self, outcome: Outcome) {
        self.export = Export::Ended(outcome);
    }

    pub fn page(&self) -> FailurePage<'_> {
        let text = |key| self.language.text(key);
        FailurePage {
            title: text(Key::ShellWindowTitle),
            heading: text(Key::StartupFailureTitle),
            message: text(self.message),
            details_header: text(Key::StartupFailureDetailsHeader),
            detail: &self.detail,
            notice: match self.export {
                Export::Ended(outcome) => outcome.notice(self.language),
                Export::Ready | Export::Running => None,
            },
            privacy: text(Key::ShellDiagnosticPrivacyText),
            export: Action {
                label: text(match self.export {
                    Export::Running => Key::StartupFailureExportingButton,
                    Export::Ready | Export::Ended(_) => Key::ShellExportDiagnosticsContent,
                }),
                name: text(Key::ShellExportDiagnosticsAutomationPropertiesName),
                enabled: self.export != Export::Running,
            },
            close: text(Key::StartupFailureCloseButton),
        }
    }
}

/// The error, then each cause; a cause whose text the one above already holds says nothing new.
fn detail(error: &anyhow::Error) -> Vec<SharedString> {
    let mut lines: Vec<SharedString> = Vec::new();
    for cause in error.chain() {
        let message = cause.to_string();
        if !lines.last().is_some_and(|last| last.contains(&message)) {
            lines.push(message.into());
        }
    }
    lines
}

#[cfg(test)]
mod tests;
