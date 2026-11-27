//! The backend of the systems Bientôt has none for: ports that answer « unknown » or do nothing,
//! so that the app builds and its tests run there.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use bientot_core::diagnostics::{FinalPaths, NoFinalPathCheck};
use bientot_platform::locale::InMemoryLocale;
use bientot_platform::startup::LaunchIntent;
use bientot_platform::system::InMemorySystem;
use gpui::{App, AsyncApp, Bounds, PathPromptOptions, Window, px, size};

use super::{Platform, PlatformContext};
use crate::updates::NoUpdates;

pub use bientot_platform::motion::InMemoryMotion as Motion;
#[cfg_attr(
    target_os = "linux",
    allow(unused_imports, reason = "the Linux backend has its own")
)]
pub use bientot_platform::single_instance::InMemoryInstance as Instance;

/// No notification shows here, so none is refused.
#[cfg_attr(
    target_os = "linux",
    allow(dead_code, reason = "the Linux backend has its own")
)]
pub type NotificationsRefused = Infallible;

pub fn motion() -> Motion {
    Motion::default()
}

pub fn build(ctx: &PlatformContext<'_>) -> anyhow::Result<Platform> {
    // The sign-in marker alone tells a launch at sign-in.
    let launch_intent = if ctx.launch.sign_in {
        LaunchIntent::SignIn
    } else {
        LaunchIntent::Manual
    };
    Ok(Platform {
        launch_intent,
        ..Platform::null()
    })
}

/// Gives `window` the size of `client`: no desktop here lets the app place it.
pub fn place_window(window: &mut Window, client: Bounds<i32>) {
    let scale = window.scale_factor();
    window.resize(size(
        px(client.size.width as f32 / scale),
        px(client.size.height as f32 / scale),
    ));
}

pub fn activate_window(window: &mut Window) {
    window.activate_window();
}

pub fn run_update_hooks() -> bool {
    false
}

pub fn install_pending_update() {}

pub fn local_app_data() -> anyhow::Result<PathBuf> {
    let variable = |name| std::env::var_os(name).map(PathBuf::from);
    variable("XDG_DATA_HOME")
        .or_else(|| variable("HOME").map(|home| home.join(".local/share")))
        .context("no folder for the application data")
}

/// The caller already wrote it to the standard error.
pub fn show_fatal(_title: &str, _body: &str) {}

/// Cannot watch a process here: answers at once, as if it had exited.
#[cfg_attr(
    target_os = "linux",
    allow(dead_code, reason = "the Linux backend has its own")
)]
pub fn wait_for_exit(_pid: u32, _timeout: Duration) -> bool {
    true
}

/// Names no language.
#[cfg_attr(
    target_os = "linux",
    allow(dead_code, reason = "the Linux backend has its own")
)]
pub fn system_locale() -> InMemoryLocale {
    InMemoryLocale::default()
}

/// Describes no system.
#[cfg_attr(
    target_os = "linux",
    allow(dead_code, reason = "the Linux backend has its own")
)]
pub fn system() -> Option<InMemorySystem> {
    None
}

pub fn store_updates() -> Result<Option<NoUpdates>, Infallible> {
    Ok(None)
}

pub fn direct_updates() -> Option<NoUpdates> {
    None
}

/// GPUI's own dialog, which offers every file.
pub async fn pick_save(
    suggested: String,
    _file_type: String,
    _extension: &'static str,
    cx: &mut AsyncApp,
) -> anyhow::Result<Option<PathBuf>> {
    let directory = std::env::home_dir().unwrap_or_default();
    let chosen = cx.update(|cx| cx.prompt_for_new_path(&directory, Some(&suggested)));
    chosen.await?
}

/// GPUI's own dialog, which offers every file.
pub async fn pick_open(
    _file_type: String,
    _extensions: &'static [&'static str],
    cx: &mut AsyncApp,
) -> anyhow::Result<Option<PathBuf>> {
    let chosen = cx.update(|cx| {
        cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        })
    });
    Ok(chosen.await??.and_then(|paths| paths.into_iter().next()))
}

/// Opens nothing: only the screenshots and the tests run here.
pub fn open_in_editor(_file: &Path, _cx: &App) {}

/// The IANA name of the zone is all the system tells.
pub fn system_zone_id() -> Option<String> {
    None
}

/// The checks .NET runs off Windows: none.
pub fn final_paths() -> &'static (dyn FinalPaths + Sync) {
    &NoFinalPathCheck
}

pub fn final_directory(_directory: &Path) -> Option<PathBuf> {
    None
}

pub fn operating_system_version() -> [u32; 4] {
    [0; 4]
}

pub fn native_machine() -> Option<u16> {
    None
}
