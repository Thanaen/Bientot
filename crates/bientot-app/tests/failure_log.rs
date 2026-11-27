//! A window of the user's own day that cannot open, as without graphics, is written to its
//! diagnostic log by name, which the export keeps. It needs the headless GPUI of the
//! `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

use std::rc::Rc;
use std::sync::Arc;

use bientot_app::adjust::Seed;
use bientot_app::adjust::window::Opener;
use bientot_app::assets::{self, Assets};
use bientot_app::host::{Source, Today};
use bientot_app::live::{self, LiveHost};
use bientot_app::strings::Language;
use bientot_app::{adjust_view, settings_view, shell, week_view};
use bientot_domain::sources::LastInputReading;
use bientot_domain::time::{FixedTimeZoneProvider, FrozenClock};
use bientot_platform::activity::LastInputProbe;
use bientot_platform::placement::{FlyoutPlacement, Frame, InMemoryDesktop};
use bientot_platform::tray::TrayHost;
use bientot_store::data_directory::DataDirectory;
use gpui::{App, HeadlessAppContext};
use jiff::tz::TimeZone;

struct NoInput;

impl LastInputProbe for NoInput {
    fn read(&self) -> Option<LastInputReading> {
        None
    }
}

#[test]
fn a_window_that_cannot_open_is_logged_by_name() {
    // The tray already shows the app, so a window that cannot open stops nothing.
    shell::app_shown();
    let folder = tempfile::tempdir().unwrap();
    let directory = DataDirectory::from(folder.path().to_owned());
    let connection = bientot_store::database::open(&directory.database_file()).unwrap();
    let host = LiveHost::start(
        &directory,
        live::load_settings(&directory).unwrap(),
        Box::leak(Box::new(connection)),
        Box::leak(Box::new(FrozenClock::new(
            "2026-10-08T08:00:00Z".parse().unwrap(),
        ))),
        Box::leak(Box::new(FixedTimeZoneProvider::new("UTC", TimeZone::UTC))),
        &NoInput,
    )
    .unwrap();
    let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(
        assets::FONT_FAMILY,
    ));
    let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(Assets), || {
        anyhow::bail!("no graphics driver")
    });
    cx.update(|cx| {
        cx.set_global(Language::French);
        cx.set_global(Today::open(Source::Live(Box::new(host)), Language::French).unwrap());
        let desktop = Rc::new(InMemoryDesktop {
            placement: FlyoutPlacement::CompositorPlaced,
            usable_height: None,
            frame: Frame::App,
        });
        shell::install(desktop, TrayHost::Present, cx);
    });

    let opens: [fn(&mut App); 4] = [
        shell::open_flyout,
        week_view::open,
        settings_view::open,
        |cx| {
            let _ = adjust_view::window::open(None, Seed::Blank, Opener::Flyout, cx);
        },
    ];
    for open in opens {
        cx.update(open);
    }
    cx.run_until_parked();

    let log = std::fs::read_dir(folder.path().join("logs"))
        .unwrap()
        .map(|file| std::fs::read_to_string(file.unwrap().path()).unwrap())
        .collect::<String>();
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        [
            r#"{"TimestampUtc":"2026-10-08T08:00:00+00:00","Level":"Error","Event":"flyout.open_failed","Properties":null}"#,
            r#"{"TimestampUtc":"2026-10-08T08:00:00+00:00","Level":"Error","Event":"week.open_failed","Properties":null}"#,
            r#"{"TimestampUtc":"2026-10-08T08:00:00+00:00","Level":"Error","Event":"settings.open_failed","Properties":null}"#,
            r#"{"TimestampUtc":"2026-10-08T08:00:00+00:00","Level":"Error","Event":"adjustment.open_failed","Properties":null}"#,
        ]
    );
}
