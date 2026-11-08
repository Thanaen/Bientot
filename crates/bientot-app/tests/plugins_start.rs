//! What a start does with the plugins: with none on, no `plugins` thread and no marker (FR-018);
//! after two starts in a row that crashed, the plugins stay off until « Réactiver les plugins »
//! (FR-052). It needs the headless GPUI of the `screenshots` feature.
#![cfg(all(feature = "screenshots", feature = "plugins", target_os = "linux"))]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use bientot_app::plugins;
use bientot_app::strings::Language;
use bientot_plugin_api::PluginId;
use bientot_plugins::state::{Grant, InstalledPlugin, SCHEMA, StateFile, Status};
use bientot_plugins::trust::Trust;
use bientot_store::data_directory::DataDirectory;
use gpui::HeadlessAppContext;

/// A plugin in `status` whose files are not there: turned on, it fails its check.
fn installed(status: Status) -> InstalledPlugin {
    InstalledPlugin {
        version: "1.0.0".into(),
        trust: Trust::Unsigned {
            package_hash: "00".into(),
        },
        files: BTreeMap::from([("plugin.json".into(), "00".into())]),
        manifest_hash: "00".into(),
        granted: Grant {
            capabilities: BTreeSet::new(),
            manifest_hash: "00".into(),
        },
        status,
        failures: Vec::new(),
        card_preference: false,
        hidden_on: None,
        rank: 0,
        pending: None,
    }
}

fn with_plugin(folder: &Path, status: Status) {
    let state = StateFile {
        schema: SCHEMA,
        plugins: BTreeMap::from([(PluginId::parse("someone.focus").unwrap(), installed(status))]),
    };
    std::fs::create_dir_all(folder.join("plugins")).unwrap();
    state
        .save(&folder.join("plugins").join("state.json"))
        .unwrap();
}

fn start(folder: &Path) -> HeadlessAppContext {
    let mut cx = common::app(true, |_| {});
    let directory = DataDirectory::from(folder.to_owned());
    cx.update(|cx| plugins::start(&directory, Language::French, cx));
    cx.run_until_parked();
    cx
}

fn marker(folder: &Path) -> Option<String> {
    std::fs::read_to_string(folder.join("plugins").join("starting")).ok()
}

#[test]
fn with_no_plugin_installed_a_start_reads_an_empty_folder_and_nothing_more() {
    let folder = tempfile::tempdir().unwrap();
    let mut cx = start(folder.path());
    assert!(!cx.update(|cx| plugins::running(cx)));
    assert!(!folder.path().join("plugins").exists());
}

#[test]
fn with_no_plugin_on_a_start_runs_no_thread_and_counts_nothing() {
    let folder = tempfile::tempdir().unwrap();
    with_plugin(folder.path(), Status::Off);
    let mut cx = start(folder.path());
    assert!(!cx.update(|cx| plugins::running(cx)));
    assert_eq!(marker(folder.path()), None);
}

#[test]
fn a_start_with_plugins_on_is_counted_until_they_have_run_a_while() {
    let folder = tempfile::tempdir().unwrap();
    with_plugin(folder.path(), Status::Active);
    let cx = start(folder.path());
    assert_eq!(marker(folder.path()).as_deref(), Some("1"));
    cx.advance_clock(Duration::from_secs(30));
    cx.run_until_parked();
    assert_eq!(marker(folder.path()), None);
}

#[test]
fn after_two_crashed_starts_the_plugins_stay_off_until_they_are_turned_back_on() {
    let folder = tempfile::tempdir().unwrap();
    with_plugin(folder.path(), Status::Active);
    std::fs::write(folder.path().join("plugins").join("starting"), "2").unwrap();
    let mut cx = start(folder.path());
    assert!(cx.update(|cx| plugins::paused(cx)));
    assert!(!cx.update(|cx| plugins::running(cx)));
    // Its files were not checked: it is still on, for when the plugins come back.
    let listed = cx.update(|cx| plugins::installed(cx)).unwrap();
    assert_eq!(listed[0].status, Status::Active);
    // « Réactiver les plugins »: a start that counts again.
    cx.update(plugins::resume);
    assert!(!cx.update(|cx| plugins::paused(cx)));
    assert_eq!(marker(folder.path()).as_deref(), Some("1"));
    // This plugin's files are missing: its check turned it off, and no thread runs for it.
    let listed = cx.update(|cx| plugins::installed(cx)).unwrap();
    assert_eq!(listed[0].status, Status::FilesChanged);
    assert!(!cx.update(|cx| plugins::running(cx)));
}
