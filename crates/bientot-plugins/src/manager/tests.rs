use std::sync::Mutex;
use std::time::Instant;

use bientot_plugin_api::Language;
use bientot_plugin_api::view::Prepared;

use super::*;
use crate::package::tests::{build, entries, key, manifest_json};
use crate::runtime::WasmSandbox;
use crate::runtime::tests::{IMPORTS, Lines, TREE, call, module};
use crate::thread::{Host, HostUpdate, Request};

/// A plugin that sets its card on every event.
fn card_plugin() -> Vec<u8> {
    module(
        IMPORTS,
        &[(64, TREE)],
        &format!("{} i64.const 0", call("set", 64, TREE)),
        "",
    )
}

fn package(signer: Option<u8>, version: &str) -> Vec<u8> {
    let mut manifest = manifest_json();
    manifest["version"] = version.into();
    build(
        &entries(&manifest, &card_plugin()),
        signer.map(key).as_ref(),
        &[],
    )
}

struct Setup {
    _dir: tempfile::TempDir,
    plugins: PluginsDir,
    state: StateFile,
    runtime: Runtime,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let plugins = PluginsDir::new(dir.path().join("plugins"));
    let state = StateFile::load(&plugins.state_path()).unwrap();
    Setup {
        _dir: dir,
        plugins,
        state,
        runtime: Runtime::default(),
    }
}

fn id() -> PluginId {
    PluginId::parse("bientot.focus").unwrap()
}

#[test]
fn install_load_run_and_remove() {
    let mut s = setup();
    let offer = offer(&package(Some(1), "0.1.0"), &[], &s.runtime, &s.state).unwrap();
    assert!(offer.needs_consent(&s.state));
    assert!(offer.fingerprint().is_some());
    install(&s.plugins, &mut s.state, offer).unwrap();
    assert_eq!(StateFile::load(&s.plugins.state_path()).unwrap(), s.state);
    assert_eq!(s.state.plugins[&id()].pending, Some(Pending::Installed));

    let (loaded, problems) = load_active(&s.plugins, &mut s.state, &[]);
    assert!(problems.is_empty());
    assert_eq!(loaded.len(), 1);

    // The real runtime behind the thread logic: the card arrives in the app language.
    let updates: Arc<Mutex<Vec<HostUpdate>>> = Arc::default();
    let sink = updates.clone();
    let now = Instant::now();
    let mut host = Host::new(
        WasmSandbox::new(Runtime::default()),
        Language::Fr,
        Arc::new(Lines::default()),
        Arc::new(|| "2026-11-08T12:00:00Z".parse().unwrap()),
        Box::new(move |update| sink.lock().unwrap().push(update)),
        now,
    );
    let day = serde_json::from_str(include_str!(
        "../../../bientot-plugin-api/tests/fixtures/api-1.0/day-snapshot.json"
    ))
    .unwrap();
    host.accept(Request::Load(Box::new(loaded[0].clone())), now);
    host.accept(Request::Day(Arc::new(day)), now);
    host.step(now);
    let updates = updates.lock().unwrap();
    assert!(updates.contains(&HostUpdate::Delivered { plugin: id() }));
    let card = updates
        .iter()
        .find_map(|u| match u {
            HostUpdate::Card {
                card: Some(card), ..
            } => Some(card.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        card.lines[0],
        Prepared::Text {
            text: "Concentration".into(),
            role: bientot_plugin_api::view::Role::Body
        }
    );
    delivered(&s.plugins, &mut s.state, &id());
    assert_eq!(s.state.plugins[&id()].pending, None);

    remove(&s.plugins, &mut s.state, &id()).unwrap();
    assert!(s.state.plugins.is_empty());
    assert!(!s.plugins.plugin(&id()).exists());
}

#[test]
fn changed_files_keep_the_plugin_off() {
    let mut s = setup();
    let offer = offer(&package(None, "0.1.0"), &[], &s.runtime, &s.state).unwrap();
    install(&s.plugins, &mut s.state, offer).unwrap();
    std::fs::write(s.plugins.plugin(&id()).join("locales/fr.json"), b"{}").unwrap();
    let (loaded, problems) = load_active(&s.plugins, &mut s.state, &[]);
    assert!(loaded.is_empty());
    assert_eq!(problems, vec![(id(), LoadProblem::FilesChanged)]);
    assert_eq!(s.state.plugins[&id()].status, Status::FilesChanged);
    assert_eq!(
        StateFile::load(&s.plugins.state_path()).unwrap().plugins[&id()].status,
        Status::FilesChanged
    );
}

#[test]
fn an_update_by_the_same_author_skips_consent_and_says_upgraded() {
    let mut s = setup();
    let first = offer(&package(Some(1), "0.1.0"), &[], &s.runtime, &s.state).unwrap();
    install(&s.plugins, &mut s.state, first).unwrap();
    delivered(&s.plugins, &mut s.state, &id());
    let update = offer(&package(Some(1), "0.2.0"), &[], &s.runtime, &s.state).unwrap();
    assert!(!update.needs_consent(&s.state));
    install(&s.plugins, &mut s.state, update).unwrap();
    assert_eq!(
        s.state.plugins[&id()].pending,
        Some(Pending::Upgraded {
            from: "0.1.0".into()
        })
    );
    assert!(matches!(
        offer(&package(Some(2), "0.3.0"), &[], &s.runtime, &s.state),
        Err(InstallError::Trust(Refusal::OtherAuthor { .. }))
    ));
}

#[test]
fn a_module_the_manifest_does_not_allow_is_refused() {
    let s = setup();
    let mut manifest = manifest_json();
    manifest["capabilities"] = serde_json::json!(["read:day"]);
    let bytes = build(&entries(&manifest, &card_plugin()), None, &[]);
    assert!(matches!(
        offer(&bytes, &[], &s.runtime, &s.state),
        Err(InstallError::Module(ModuleError::NotGranted(
            Capability::UiFlyout
        )))
    ));
}

#[test]
fn the_third_failure_turns_it_off() {
    let mut s = setup();
    let first = offer(&package(None, "0.1.0"), &[], &s.runtime, &s.state).unwrap();
    install(&s.plugins, &mut s.state, first).unwrap();
    let at = |m: i64| Timestamp::from_second(1_800_000_000 + m * 60).unwrap();
    assert!(!record_failure(
        &s.plugins,
        &mut s.state,
        &id(),
        at(0),
        "trap"
    ));
    assert!(!record_failure(
        &s.plugins,
        &mut s.state,
        &id(),
        at(1),
        "trap"
    ));
    assert!(record_failure(
        &s.plugins,
        &mut s.state,
        &id(),
        at(2),
        "trap"
    ));
    let (loaded, _) = load_active(&s.plugins, &mut s.state, &[]);
    assert!(loaded.is_empty());
}
