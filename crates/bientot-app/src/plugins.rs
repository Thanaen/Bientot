//! The plugins of `specs/003-plugin-runtime` in the app: the `plugins` thread, fed with the
//! published day, and the cards it sends back, kept for the flyout.
//!
//! Nothing here runs when no plugin is on: no thread, no engine (FR-018).

use std::collections::BTreeMap;
use std::sync::Arc;

use bientot_core::diagnostics::{DiagnosticLogLevel, DiagnosticValue};
use bientot_plugin_api::{Capability, PluginId, PreparedCard};
use bientot_plugins::files::PluginsDir;
use bientot_plugins::log::LogFiles;
use bientot_plugins::manager;
use bientot_plugins::runtime::{POOL_BYTES, Pool, Runtime, WasmSandbox};
use bientot_plugins::state::StateFile;
use bientot_plugins::thread::{self, HostUpdate, PluginThread, Request};
use bientot_store::data_directory::DataDirectory;
use futures::StreamExt as _;
use gpui::{App, Global};
use jiff::Timestamp;

use crate::host::Today;
use crate::palette;
use crate::strings::Language;

/// The cards of the plugins that show one, by plugin, for the flyout. Swapped whole, so that a
/// flyout drawing keeps the set it started with.
#[derive(Default)]
pub struct PluginCards(pub Arc<BTreeMap<PluginId, PreparedCard>>);

impl Global for PluginCards {}

/// The running plugins.
struct Plugins {
    thread: PluginThread,
    dir: PluginsDir,
    state: StateFile,
    /// Some loaded plugin reads the day, through `bt:day@1 current` or `day.refreshed`.
    reads_day: bool,
    /// The app's language, which changes only with a restart.
    language: Language,
}

impl Global for Plugins {}

/// The folder of the plugins in `directory`.
pub fn folder(directory: &DataDirectory) -> PluginsDir {
    PluginsDir::new(directory.path().join("plugins"))
}

/// Starts the plugins that are on, after the tray is ready (plan, Lifecycle 2 and 3).
pub fn start(directory: &DataDirectory, language: Language, cx: &mut App) {
    cx.set_global(PluginCards::default());
    let dir = folder(directory);
    if !dir.state_path().exists() {
        return;
    }
    let mut state = match StateFile::load(&dir.state_path()) {
        Ok(state) => state,
        Err(error) => {
            diagnose(
                cx,
                "plugins.state_unreadable",
                &[("error", error.to_string().as_str().into())],
            );
            return;
        }
    };
    state.on_start();
    dir.sweep();
    let reserved = palette::reserved_titles();
    let reserved: Vec<&str> = reserved.iter().map(String::as_str).collect();
    let (specs, problems) = manager::load_active(&dir, &mut state, &reserved);
    for (id, problem) in problems {
        let problem = format!("{problem:?}");
        diagnose(
            cx,
            "plugins.stays_off",
            &[
                ("plugin", id.as_str().into()),
                ("problem", problem.as_str().into()),
            ],
        );
    }
    if specs.is_empty() {
        return;
    }
    let reads_day = specs
        .iter()
        .any(|spec| spec.granted.contains(&Capability::ReadDay));
    let clock: bientot_plugins::host::Clock = Arc::new(Timestamp::now);
    let log = Arc::new(LogFiles::new(
        directory.log_directory().join("plugins"),
        clock.clone(),
    ));
    log.prune();
    let (updates, mut received) = futures::channel::mpsc::unbounded();
    let notify = Box::new(move |update: HostUpdate| {
        let _ = updates.unbounded_send(update);
    });
    let spawned = thread::spawn(
        || WasmSandbox::new(Runtime::new(Pool::new(POOL_BYTES))),
        plugin_language(language),
        log,
        clock,
        notify,
    );
    let thread = match spawned {
        Ok(thread) => thread,
        Err(error) => {
            diagnose(
                cx,
                "plugins.no_thread",
                &[("error", error.to_string().as_str().into())],
            );
            return;
        }
    };
    for spec in specs {
        thread.send(Request::Load(Box::new(spec)));
    }
    cx.set_global(Plugins {
        thread,
        dir,
        state,
        reads_day,
        language,
    });
    cx.spawn(async move |cx| {
        while let Some(update) = received.next().await {
            cx.update(|cx| apply(update, cx));
        }
    })
    .detach();
    day_changed(cx);
}

/// Sends the published day to the plugins that read it. The thread keeps only the latest, which
/// `bt:day@1 current` answers, and sends `day.refreshed` to the plugins that listen to it on a
/// structural change, at most every 30 s (FR-032).
pub fn day_changed(cx: &mut App) {
    let Some(plugins) = cx.try_global::<Plugins>() else {
        return;
    };
    if !plugins.reads_day {
        return;
    }
    let language = plugin_language(plugins.language);
    let day = cx
        .global_mut::<Today>()
        .live()
        .and_then(|host| host.plugin_day(language));
    if let Some(day) = day {
        cx.global::<Plugins>()
            .thread
            .send(Request::Day(Arc::new(day)));
    }
}

fn apply(update: HostUpdate, cx: &mut App) {
    match update {
        HostUpdate::Card { plugin, card } => set_card(plugin, card, cx),
        HostUpdate::Failed { plugin, error, at } => {
            diagnose(
                cx,
                "plugins.failed",
                &[
                    ("plugin", plugin.as_str().into()),
                    ("error", error.as_str().into()),
                ],
            );
            let plugins = cx.global_mut::<Plugins>();
            if manager::record_failure(&plugins.dir, &mut plugins.state, &plugin, at, &error) {
                plugins.thread.send(Request::Unload(plugin.clone()));
                set_card(plugin, None, cx);
            }
        }
        HostUpdate::Delivered { plugin } => {
            let plugins = cx.global_mut::<Plugins>();
            manager::delivered(&plugins.dir, &mut plugins.state, &plugin);
        }
    }
}

fn set_card(plugin: PluginId, card: Option<PreparedCard>, cx: &mut App) {
    let cards = &mut cx.global_mut::<PluginCards>().0;
    let changed = match card {
        Some(card) => {
            cards.get(&plugin) != Some(&card) && {
                Arc::make_mut(cards).insert(plugin, card);
                true
            }
        }
        None => {
            cards.contains_key(&plugin) && {
                Arc::make_mut(cards).remove(&plugin);
                true
            }
        }
    };
    if changed {
        cx.refresh_windows();
    }
}

/// Writes `event` to the app's diagnostic log. Only what Bientôt says of a plugin goes there,
/// never the plugin's own log lines (FR-034).
fn diagnose(cx: &mut App, event: &str, properties: &[(&str, DiagnosticValue)]) {
    if let Some(host) = cx.global_mut::<Today>().live() {
        host.log(DiagnosticLogLevel::Warning, event, properties);
    }
}

fn plugin_language(language: Language) -> bientot_plugin_api::Language {
    match language {
        Language::French => bientot_plugin_api::Language::Fr,
        Language::English => bientot_plugin_api::Language::En,
    }
}
