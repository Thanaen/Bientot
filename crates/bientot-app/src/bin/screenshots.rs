//! Renders the flyout of every demo scenario, the week of the scenarios that store one, and the
//! states of « Corriger le temps… », « Paramètres » and the onboarding, to PNG files, in one
//! language and one theme.
//!
//! Usage: `bientot-screenshots <output directory> --french|--english --light|--dark
//! [--platform windows|linux]`. Each scenario module's doc lists the files it writes there; a
//! `<name>-motion-<percent>.png` is an entrance at that share of its duration. `--platform linux`
//! takes only the scenes that differ on Linux (`screenshots/platform.rs`); Windows, the default,
//! takes them all.
//! Runs on Linux only: it needs a Vulkan driver, and CI uses Mesa's software one (lavapipe).

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
use bientot_app::strings::Language;
use bientot_app::theme::{ThemeMode, motion};
use bientot_app::week_view;
use gpui::{AnyWindowHandle, HeadlessAppContext};
use platform::{Platform, Shots};

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
#[path = "screenshots/platform.rs"]
mod platform;
#[path = "screenshots/plugins.rs"]
mod plugins;
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

const USAGE: &str = "usage: bientot-screenshots <output directory> --french|--english \
                     --light|--dark [--platform windows|linux]";

fn main() -> anyhow::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let output = PathBuf::from(arguments.next().context(USAGE)?);
    let mut platform = Platform::Windows;
    let mut rest = Vec::new();
    while let Some(argument) = arguments.next() {
        if argument == "--platform" {
            let name = arguments.next().context(USAGE)?;
            platform = Platform::parse(&name).context(USAGE)?;
        } else {
            rest.push(argument);
        }
    }
    let launch = Launch::parse(rest).context(USAGE)?;
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
        cx.set_global(Shots::new(platform));
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

    for (group, scenes) in SCENES {
        if platform.takes_group(group) {
            scenes(&mut cx, language, &output)?;
        }
    }
    let missed = cx.update(|cx| cx.global::<Shots>().missed());
    anyhow::ensure!(missed.is_empty(), "no scene took {missed:?}");
    Ok(())
}

/// What takes a group of scenes, in one language, to a folder.
type Scenes = fn(&mut HeadlessAppContext, Language, &std::path::Path) -> anyhow::Result<()>;

/// Every group of scenes, by the name `screenshots/platform.rs` gives it, in the order taken.
const SCENES: [(&str, Scenes); 21] = [
    ("flyout::capture_scenarios", flyout::capture_scenarios),
    ("saturday::capture_day_off", saturday::capture_day_off),
    (
        "flyout::capture_record_work_unavailable",
        flyout::capture_record_work_unavailable,
    ),
    (
        "flyout::capture_unrecorded_focused",
        flyout::capture_unrecorded_focused,
    ),
    (
        "flyout::capture_unrecorded_counted",
        flyout::capture_unrecorded_counted,
    ),
    ("flyout::capture_notices", flyout::capture_notices),
    (
        "flyout::capture_appointment_review",
        flyout::capture_appointment_review,
    ),
    (
        "flyout::capture_rebuilt_review",
        flyout::capture_rebuilt_review,
    ),
    ("flyout::capture_updates", flyout::capture_updates),
    ("flyout::capture_palette", flyout::capture_palette),
    ("plugins::capture_cards", plugins::capture_cards),
    ("week::capture_weeks", week::capture_weeks),
    ("details::capture_details", details::capture_details),
    ("adjust::capture_states", adjust::capture_states),
    ("adjust::capture_declare_work", adjust::capture_declare_work),
    ("midnight::capture_midnight", midnight::capture_midnight),
    ("adjust::capture_edits", adjust::capture_edits),
    ("settings::capture_pages", settings::capture_pages),
    (
        "settings::capture_plugin_pages",
        settings::capture_plugin_pages,
    ),
    ("onboarding::capture_steps", onboarding::capture_steps),
    (
        "startup_failure::capture_failures",
        startup_failure::capture_failures,
    ),
];

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
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !cx.update(|cx| cx.global_mut::<Shots>().take(name)) {
        return Ok(());
    }
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
