//! Renders the flyout of every demo scenario, the week of the scenarios that store one, and the
//! states of « Corriger le temps… », « Paramètres » and the onboarding, to PNG files, in one
//! language and one theme.
//!
//! Usage: `bientot-screenshots <output directory> --french|--english --light|--dark`. Each
//! scenario module's doc lists the files it writes there; a `<name>-motion-<percent>.png` is an
//! entrance at that share of its duration.
//! Linux only: it needs a Vulkan driver, and CI uses Mesa's software one (lavapipe).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use bientot_app::adjust_view;
use bientot_app::assets::{self, Assets};
use bientot_app::launch::Launch;
use bientot_app::onboarding_view;
use bientot_app::palette_view;
use bientot_app::settings_view;
use bientot_app::startup_failure_view;
use bientot_app::theme::{ThemeMode, motion};
use bientot_app::week_view;
use gpui::{AnyWindowHandle, HeadlessAppContext};

#[path = "screenshots/adjust.rs"]
mod adjust;
#[path = "screenshots/details.rs"]
mod details;
#[path = "screenshots/fixtures.rs"]
mod fixtures;
#[path = "screenshots/flyout.rs"]
mod flyout;
#[path = "screenshots/midnight.rs"]
mod midnight;
#[path = "screenshots/onboarding.rs"]
mod onboarding;
#[path = "screenshots/saturday.rs"]
mod saturday;
#[path = "screenshots/settings.rs"]
mod settings;
#[path = "screenshots/shared.rs"]
mod shared;
#[path = "screenshots/startup_failure.rs"]
mod startup_failure;
#[path = "screenshots/week.rs"]
mod week;

/// The corrections the history week demo records.
const HISTORY_WEEK_MEETING: &str = "demo-history-week-meeting";
const HISTORY_WEEK_MANUAL: &str = "demo-history-week-manual";

const USAGE: &str =
    "usage: bientot-screenshots <output directory> --french|--english --light|--dark";

fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let output = PathBuf::from(arguments.next().context(USAGE)?);
    let launch = Launch::parse(arguments).context(USAGE)?;
    anyhow::ensure!(launch.theme != ThemeMode::System, USAGE);
    let language = launch.language.context(USAGE)?;
    std::fs::create_dir_all(&output)?;

    // Only the bundled fonts, so the images do not depend on the machine's fonts.
    let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(
        assets::FONT_FAMILY,
    ));
    let mut cx = HeadlessAppContext::with_platform(
        text_system,
        Arc::new(Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        cx.set_global(launch.theme);
        cx.set_global(language);
        assets::load_fonts(cx)?;
        bientot_app::flyout::bind_keys(cx);
        week_view::bind_keys(cx);
        palette_view::bind_keys(cx);
        adjust_view::bind_keys(cx);
        settings_view::bind_keys(cx);
        onboarding_view::bind_keys(cx);
        startup_failure_view::bind_keys(cx);
        anyhow::Ok(())
    })?;

    flyout::capture_scenarios(&mut cx, language, &output)?;
    saturday::capture_day_off(&mut cx, language, &output)?;
    flyout::capture_record_work_unavailable(&mut cx, language, &output)?;
    flyout::capture_unrecorded_focused(&mut cx, language, &output)?;
    flyout::capture_unrecorded_counted(&mut cx, language, &output)?;
    flyout::capture_notices(&mut cx, language, &output)?;
    flyout::capture_appointment_review(&mut cx, language, &output)?;
    flyout::capture_rebuilt_review(&mut cx, language, &output)?;
    flyout::capture_updates(&mut cx, language, &output)?;
    flyout::capture_palette(&mut cx, language, &output)?;
    week::capture_weeks(&mut cx, language, &output)?;
    details::capture_details(&mut cx, language, &output)?;
    adjust::capture_states(&mut cx, language, &output)?;
    adjust::capture_declare_work(&mut cx, language, &output)?;
    midnight::capture_midnight(&mut cx, language, &output)?;
    adjust::capture_edits(&mut cx, language, &output)?;
    settings::capture_pages(&mut cx, language, &output)?;
    onboarding::capture_steps(&mut cx, language, &output)?;
    startup_failure::capture_failures(&mut cx, language, &output)?;
    Ok(())
}

/// The first frame of the dialog's entrance, the next two at 60 Hz (16.7 ms and 33.3 ms of
/// 250), midway and at rest, in percent.
const DIALOG_FRAMES: [u32; 5] = [0, 7, 13, 50, 100];
/// The same frames of a menu's 200 ms.
const MENU_FRAMES: [u32; 5] = [0, 8, 17, 50, 100];

/// An entrance of `timing` that starts on the next frame, as `<name>-motion-<percent>.png`, on a
/// clock that stands still between frames; the last percent is 100, at rest.
fn capture_entrance(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    timing: motion::Timing,
    percents: [u32; 5],
    output: &std::path::Path,
    name: &str,
) -> anyhow::Result<()> {
    let duration = timing.duration();
    let mut elapsed = Duration::ZERO;
    for percent in percents {
        // Steps up to each share rather than by it, so 100 lands on the duration exactly.
        let at = duration * percent / 100;
        cx.advance_clock(at - elapsed);
        elapsed = at;
        capture(
            cx,
            window,
            &output.join(format!("{name}-motion-{percent}.png")),
        )?;
    }
    Ok(())
}

fn capture(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    cx.run_until_parked();
    cx.update_window(window, |_, window, _| window.refresh())?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(window)?;
    image
        .save(path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    println!("{}", path.display());
    Ok(())
}
