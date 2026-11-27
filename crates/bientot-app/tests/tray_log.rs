//! The diagnostic log of a live day names the tray's backend and whether a host shows the icon,
//! then each change of the host, and so does the export, which holds the log. It also writes the
//! lines the other backends send, such as the idle backend Linux chose, which the export then
//! names. It needs the headless GPUI of the `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use std::rc::Rc;

use bientot_app::host::{self, Source, Today};
use bientot_app::live::{self, LiveHost};
use bientot_app::platform::{BackendNote, Backends};
use bientot_app::shell;
use bientot_app::strings::Language;
use bientot_core::diagnostics::{DiagnosticLogLevel, DiagnosticValue};
use bientot_domain::sources::LastInputReading;
use bientot_domain::time::{FixedTimeZoneProvider, SystemClock};
use bientot_platform::activity::LastInputProbe;
use bientot_platform::placement::{FlyoutPlacement, InMemoryDesktop};
use bientot_platform::tray::{InMemoryTray, TrayEvent, TrayHost};
use bientot_store::data_directory::DataDirectory;
use futures::channel::mpsc;
use jiff::tz::TimeZone;

struct NoInput;

impl LastInputProbe for NoInput {
    fn read(&self) -> Option<LastInputReading> {
        None
    }
}

/// The events of the log in `folder`, with their properties, in the order they were written.
fn logged(folder: &std::path::Path) -> Vec<String> {
    let mut files: Vec<_> = std::fs::read_dir(folder.join("logs"))
        .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default();
    files.sort();
    files
        .iter()
        .flat_map(|file| {
            std::fs::read_to_string(file)
                .unwrap()
                .lines()
                .map(|line| {
                    let start = line.find("\"Event\"").unwrap();
                    line[start..].to_owned()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A live day on `folder` showing in a tray of `backend`, whose host is present, and the
/// channel the tray's events go through.
fn installed(
    folder: &std::path::Path,
    backend: Option<&'static str>,
) -> (gpui::HeadlessAppContext, mpsc::UnboundedSender<TrayEvent>) {
    let directory = DataDirectory::from(folder.to_owned());
    let connection = bientot_store::database::open(&directory.database_file()).unwrap();
    let host = LiveHost::start(
        &directory,
        live::load_settings(&directory).unwrap(),
        Box::leak(Box::new(connection)),
        &SystemClock,
        Box::leak(Box::new(FixedTimeZoneProvider::new("UTC", TimeZone::UTC))),
        &NoInput,
    )
    .unwrap();
    let today = Today::open(Source::Live(Box::new(host)), Language::French).unwrap();
    let (events, received) = mpsc::unbounded();
    let mut cx = common::app(true, |_| {});
    cx.update(|cx| {
        cx.set_global(today);
        let desktop = Rc::new(InMemoryDesktop {
            placement: FlyoutPlacement::CompositorPlaced,
            usable_height: None,
            system_draws_frame: false,
        });
        shell::install(desktop, TrayHost::Present, cx);
        let tray = Rc::new(InMemoryTray::new(vec![16], None, TrayHost::Present));
        host::install_tray(tray, backend, received, cx);
    });
    cx.run_until_parked();
    (cx, events)
}

#[test]
fn the_log_names_the_backend_and_the_host_then_each_change_of_the_host() {
    let folder = tempfile::tempdir().unwrap();
    let (cx, events) = installed(folder.path(), Some("status-notifier"));

    events
        .unbounded_send(TrayEvent::HostChanged(TrayHost::Absent))
        .unwrap();
    events
        .unbounded_send(TrayEvent::HostChanged(TrayHost::Present))
        .unwrap();
    cx.run_until_parked();

    assert_eq!(
        logged(folder.path()),
        [
            r#""Event":"tray.backend","Properties":{"name":"status-notifier"}}"#,
            r#""Event":"tray.host","Properties":{"state":"present"}}"#,
            r#""Event":"tray.host","Properties":{"state":"absent"}}"#,
            r#""Event":"tray.host","Properties":{"state":"present"}}"#,
        ]
    );
}

#[test]
fn a_tray_without_a_backend_name_logs_nothing_at_start() {
    let folder = tempfile::tempdir().unwrap();
    let (_cx, _events) = installed(folder.path(), None);

    assert_eq!(logged(folder.path()), Vec::<String>::new());
}

#[test]
fn the_log_writes_each_line_the_backends_send_those_sent_before_the_day_included() {
    let folder = tempfile::tempdir().unwrap();
    let (mut cx, _events) = installed(folder.path(), None);
    let (notes, received) = mpsc::unbounded();
    let note = |event, properties: Vec<(&'static str, DiagnosticValue<'static>)>| BackendNote {
        level: DiagnosticLogLevel::Information,
        event,
        properties,
        idle: None,
        lifecycle: None,
    };
    cx.update(|cx| {
        cx.set_global(Backends {
            lifecycle: Some("logind"),
            ..Backends::default()
        })
    });

    notes
        .unbounded_send(BackendNote {
            idle: Some("none"),
            ..note("idle.backend", vec![("name", "none".into())])
        })
        .unwrap();
    cx.update(|cx| host::log_backend_notes(received, cx));
    notes
        .unbounded_send(BackendNote {
            idle: Some("MIT-SCREEN-SAVER"),
            ..note(
                "idle.backend_changed",
                vec![("from", "none".into()), ("to", "MIT-SCREEN-SAVER".into())],
            )
        })
        .unwrap();
    notes
        .unbounded_send(note("idle.inhibitor_blind", vec![("value", false.into())]))
        .unwrap();
    notes
        .unbounded_send(BackendNote {
            level: DiagnosticLogLevel::Warning,
            lifecycle: Some("none"),
            ..note("lifecycle.watch_failed", vec![("kind", "not_found".into())])
        })
        .unwrap();
    cx.run_until_parked();

    assert_eq!(
        logged(folder.path()),
        [
            r#""Event":"idle.backend","Properties":{"name":"none"}}"#,
            r#""Event":"idle.backend_changed","Properties":{"from":"none","to":"MIT-SCREEN-SAVER"}}"#,
            r#""Event":"idle.inhibitor_blind","Properties":{"value":false}}"#,
            r#""Event":"lifecycle.watch_failed","Properties":{"kind":"not_found"}}"#,
        ]
    );
    assert_eq!(
        cx.update(|cx| cx.global::<Backends>().named()),
        [("idle", "MIT-SCREEN-SAVER"), ("lifecycle", "none")],
        "the export names the idle and lifecycle backends in use"
    );
}
