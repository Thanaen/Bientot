use std::path::Path;

use anyhow::Context as _;
use worktime_core::demo::DemoScenario;
use worktime_core::diagnostics::{
    DiagnosticInitializationState, DiagnosticLogLevel, DiagnosticLogOptions, LocalDiagnosticLogger,
};
use worktime_domain::configuration::WorkTimeSettings;
use worktime_domain::time::SystemClock;
use worktime_store::settings::SettingsStore;

use super::*;
use crate::demo::DemoHost;
use crate::live;
use crate::widgets::NoticeKind;

/// Opens the ledger of `folder` as `main.rs` does, on a file that is not a database.
fn start_on_a_corrupt_ledger(folder: &Path) -> anyhow::Error {
    let directory = DataDirectory::from(folder.to_owned());
    std::fs::write(directory.database_file(), "not a database").unwrap();
    match live::open_ledger(&directory) {
        Ok(_) => panic!("a ledger that is not a database opened"),
        Err(error) => error,
    }
}

fn failed_start(folder: &Path, forced: Option<Language>) -> StartupFailure {
    let error = start_on_a_corrupt_ledger(folder);
    StartupFailure::new(
        &error,
        Some(&DataDirectory::from(folder.to_owned())),
        forced,
    )
}

fn ledger_error(folder: &Path) -> Vec<SharedString> {
    vec![
        format!(
            "cannot open the ledger {}",
            folder.join("worktime.db").display()
        )
        .into(),
        "SQLite failed: file is not a database".into(),
        "Error code 26: file is not a database".into(),
    ]
}

#[test]
fn a_ledger_that_does_not_open_shows_the_error_and_offers_the_export() {
    let folder = tempfile::tempdir().unwrap();
    let failure = failed_start(folder.path(), Some(Language::English));
    let detail = ledger_error(folder.path());
    assert_eq!(
        failure.page(),
        FailurePage {
            title: "WorkTime",
            heading: "WorkTime could not start",
            message: "Check the settings and local database, then restart WorkTime. If it keeps \
                      happening, export diagnostics.",
            details_header: "Details",
            detail: &detail,
            notice: None,
            privacy: "You choose where to save the ZIP and who to share it with.",
            export: Action {
                label: "Export diagnostics…",
                name: "Export local diagnostics",
                enabled: true,
            },
            close: "Close",
        }
    );
}

#[test]
fn the_failure_reads_in_french() {
    let folder = tempfile::tempdir().unwrap();
    let failure = failed_start(folder.path(), Some(Language::French));
    let detail = ledger_error(folder.path());
    assert_eq!(
        failure.page(),
        FailurePage {
            title: "WorkTime",
            heading: "WorkTime n’a pas pu démarrer",
            message: "Vérifiez les paramètres et la base de données locale, puis relancez WorkTime. \
                      Si le problème persiste, exportez le diagnostic.",
            details_header: "Détails",
            detail: &detail,
            notice: None,
            privacy: "Vous choisissez où enregistrer le ZIP et avec qui le partager.",
            export: Action {
                label: "Exporter le diagnostic…",
                name: "Exporter le diagnostic local",
                enabled: true,
            },
            close: "Fermer",
        }
    );
}

#[test]
fn the_failure_speaks_the_language_the_settings_chose() {
    let folder = tempfile::tempdir().unwrap();
    let settings = WorkTimeSettings {
        language_override: Some("en-US".to_owned()),
        ..WorkTimeSettings::default()
    };
    SettingsStore::new(DataDirectory::from(folder.path().to_owned()).settings_file())
        .unwrap()
        .save(&settings)
        .unwrap();
    let failure = failed_start(folder.path(), None);
    assert_eq!(failure.language(), Language::English);
    assert_eq!(failure.page().heading, "WorkTime could not start");
}

#[test]
fn invalid_settings_stay_where_they_are() {
    let folder = tempfile::tempdir().unwrap();
    let directory = DataDirectory::from(folder.path().to_owned());
    std::fs::write(directory.settings_file(), "{broken").unwrap();
    let error = anyhow::anyhow!("cannot read the settings");

    let failure = StartupFailure::new(&error, Some(&directory), Some(Language::English));

    assert_eq!(failure.contents().snapshot.settings, None);
    assert_eq!(
        std::fs::read_to_string(directory.settings_file()).unwrap(),
        "{broken"
    );
    assert_eq!(
        std::fs::read_dir(folder.path()).unwrap().count(),
        1,
        "nothing written to the data folder"
    );
}

#[test]
fn the_export_saves_the_earlier_log_without_a_started_day() {
    let folder = tempfile::tempdir().unwrap();
    let directory = DataDirectory::from(folder.path().to_owned());
    // A run before this one logged its start.
    LocalDiagnosticLogger::new(
        directory.log_directory(),
        DiagnosticLogOptions::default(),
        &SystemClock,
    )
    .write(DiagnosticLogLevel::Information, "startup.tray_ready", &[])
    .unwrap();
    let failure = failed_start(folder.path(), Some(Language::English));

    let contents = failure.contents();
    assert_eq!(
        contents.snapshot.initialization,
        DiagnosticInitializationState::Failed
    );
    assert_eq!(contents.snapshot.summary, None);
    assert!(contents.snapshot.settings.is_some(), "the settings read");
    let zip = folder
        .path()
        .join("export")
        .join("worktime-diagnostics.zip");
    let written = contents.write(&zip).unwrap();
    assert_eq!(written.log_record_count, 1);
    let archive = std::fs::read(&zip).unwrap();
    assert!(archive.starts_with(b"PK\x03\x04"), "a ZIP archive");
    for entry in ["manifest.json", "snapshot.json", "logs.jsonl"] {
        assert!(
            archive
                .windows(entry.len())
                .any(|window| window == entry.as_bytes()),
            "the archive holds {entry}"
        );
    }
}

#[test]
fn an_unreadable_data_folder_still_exports_the_failure() {
    let folder = tempfile::tempdir().unwrap();
    // A file where the data folder should be.
    let data = folder.path().join("data");
    std::fs::write(&data, "").unwrap();
    let error = anyhow::anyhow!("cannot read the settings");
    let failure = StartupFailure::new(
        &error,
        Some(&DataDirectory::from(data.clone())),
        Some(Language::English),
    );
    assert_eq!(failure.contents().snapshot.settings, None);
    assert_eq!(failure.contents().logs, Some(data.join("logs")));
    let zip = folder.path().join("worktime-diagnostics.zip");
    assert_eq!(failure.contents().write(&zip).unwrap().log_record_count, 0);
}

#[test]
fn a_running_export_disables_its_button_then_tells_its_outcome() {
    let folder = tempfile::tempdir().unwrap();
    let mut failure = failed_start(folder.path(), Some(Language::English));
    failure.start_export();
    assert_eq!(
        failure.page().export,
        Action {
            label: "Exporting…",
            name: "Export local diagnostics",
            enabled: false,
        }
    );

    failure.exported(Outcome::Saved);
    let page = failure.page();
    assert!(page.export.enabled);
    assert_eq!(page.export.label, "Export diagnostics…");
    assert_eq!(
        page.notice,
        Some(Notice {
            message: "The diagnostic ZIP was saved.".to_owned(),
            kind: NoticeKind::Success,
            undo: None,
        })
    );

    failure.start_export();
    assert_eq!(
        failure.page().notice,
        None,
        "a new export clears the last outcome"
    );
    failure.exported(Outcome::Failed);
    assert_eq!(
        failure.page().notice,
        Some(Notice {
            message: "WorkTime could not save the diagnostic ZIP. Choose another location and \
                      try again."
                .to_owned(),
            kind: NoticeKind::Error,
            undo: None,
        })
    );

    failure.start_export();
    failure.exported(Outcome::Cancelled);
    let page = failure.page();
    assert_eq!(page.notice, None, "a closed dialog tells nothing");
    assert!(page.export.enabled);
}

/// What the page says when the demo of `scenario` refuses `folder`: the same whether the refusal
/// reaches it as is or wrapped in another error.
fn refused_demo(scenario: DemoScenario, folder: &Path) -> String {
    let message = |error: anyhow::Error| {
        StartupFailure::new(&error, None, Some(Language::English))
            .page()
            .message
            .to_owned()
    };
    let Err(refusal) = DemoHost::open(scenario, folder) else {
        panic!("the demo opened {}", folder.display());
    };
    let Err(wrapped) = DemoHost::open(scenario, folder).context("cannot start the demo") else {
        panic!("the demo opened {}", folder.display());
    };
    let said = message(refusal);
    assert_eq!(message(wrapped), said, "a wrapped refusal says the same");
    said
}

#[test]
fn a_demo_refusing_its_folder_says_why() {
    let real_data = tempfile::tempdir().unwrap();
    SettingsStore::new(DataDirectory::from(real_data.path().to_owned()).settings_file())
        .unwrap()
        .save(&WorkTimeSettings::default())
        .unwrap();
    assert_eq!(
        refused_demo(DemoScenario::Late, real_data.path()),
        "This folder holds WorkTime data that the demo did not create, and the demo leaves it \
         untouched. Launch the demo with another folder, or without --data-dir."
    );

    let other_scenario = tempfile::tempdir().unwrap();
    drop(DemoHost::open(DemoScenario::ProblemPeriods, other_scenario.path()).unwrap());
    assert_eq!(
        refused_demo(DemoScenario::Late, other_scenario.path()),
        "This folder keeps the demo of another scenario. Launch that scenario again, or launch \
         this one with another folder."
    );
}
