//! The backend of the systems Bientôt has none for: ports that answer « unknown » or do nothing,
//! so that the app builds and its tests run there.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context as _;
use bientot_platform::startup::LaunchIntent;
use gpui::{Bounds, Window, px, size};

use super::{Platform, PlatformContext};

pub use bientot_platform::motion::InMemoryMotion as Motion;
pub use bientot_platform::single_instance::InMemoryInstance as Instance;

pub fn motion() -> Motion {
    Motion::default()
}

pub fn build(ctx: &PlatformContext<'_>) -> anyhow::Result<Platform> {
    // The sign-in marker alone tells a launch at sign-in.
    let launch_intent = if ctx.launch.windows_startup {
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
pub fn wait_for_exit(_pid: u32, _timeout: Duration) -> bool {
    true
}
