//! « Oui, compter 26 min » of the flyout's hint counts the unrecorded time as work and leaves a
//! notice with « Annuler », which brings it back, each reached with Tab and pressed with Enter as
//! a keyboard user does. It needs the headless GPUI of the `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use gpui::{HeadlessAppContext, WindowHandle};
use worktime_app::demo::{self, DemoHost};
use worktime_app::flyout::{self, Flyout};
use worktime_app::host::{self, Source, Today};
use worktime_app::shell;
use worktime_app::strings::Language;
use worktime_app::week_view::WeekView;
use worktime_app::widgets::NoticeKind;
use worktime_core::demo::DemoScenario;
use worktime_domain::intervals::UtcInterval;

/// `text` with each `~` as a no-break space.
fn nb(text: &str) -> String {
    text.replace('~', "\u{a0}")
}

/// The app on the problem periods' day, its flyout open.
fn open() -> (HeadlessAppContext, WindowHandle<Flyout>) {
    let mut cx = common::app(true, flyout::bind_keys);
    let directory = demo::throwaway_directory();
    let host = DemoHost::open(DemoScenario::ProblemPeriods, &directory).unwrap();
    std::fs::remove_dir_all(&directory).ok();
    let today = Today::open(Source::Demo(Box::new(host)), Language::French).unwrap();
    cx.update(|cx| {
        host::install(today, cx);
        shell::open_flyout(cx);
    });
    cx.run_until_parked();
    let window = cx.update(|cx| cx.windows()).into_iter().next().unwrap();
    let window = window.downcast::<Flyout>().unwrap();
    common::redraw(&mut cx, window);
    (cx, window)
}

/// The notice and the time worked the flyout shows, and the hint's sentence, if any.
fn shown(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<Flyout>,
) -> (Option<(String, NoticeKind, bool)>, String, Option<String>) {
    let notice = cx.update(|cx| {
        window
            .read(cx)
            .unwrap()
            .shown_notice()
            .map(|notice| (notice.message.clone(), notice.kind, notice.undo.is_some()))
    });
    let day = cx.update(|cx| cx.global::<Today>().view().clone());
    (
        notice,
        day.worked.unwrap().worked.to_string(),
        day.source_hint.map(|hint| hint.message().to_string()),
    )
}

/// Tab until the focus is in `part` of the body, then Enter.
fn press_in_part(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>, part: usize) {
    tab_to_part(cx, window, part);
    common::press(cx, window, "enter");
    common::redraw(cx, window);
}

/// Tab until the focus is in `part` of the body: on its first control.
fn tab_to_part(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>, part: usize) {
    for _ in 0..16 {
        common::press(cx, window, "tab");
        let focused = cx
            .update_window(window.into(), |_, window, cx| {
                window
                    .root::<Flyout>()
                    .flatten()
                    .and_then(|flyout| flyout.read(cx).focused_part(window, cx))
            })
            .unwrap();
        if focused == Some(part) {
            return;
        }
    }
    panic!("Tab does not reach part {part}");
}

#[test]
fn counting_the_unrecorded_time_from_the_keyboard_then_undoing_it() {
    let (mut cx, window) = open();
    let hint = nb(
        "Temps non renseigné · 26~min en 2~créneaux, entre 09:40 et 14:20. \
         Travailliez-vous\u{202f}?",
    );
    assert_eq!(
        shown(&mut cx, window),
        (None, nb("5~h~04~min"), Some(hint.clone()))
    );

    // The hint is the first part of the body; « Oui, compter 26 min » its first button.
    press_in_part(&mut cx, window, 0);
    assert_eq!(
        shown(&mut cx, window),
        (
            Some((
                nb("26~min comptées comme travail."),
                NoticeKind::Success,
                true
            )),
            nb("5~h~30~min"),
            None
        )
    );

    // The notice, above the day, holds « Annuler ».
    press_in_part(&mut cx, window, 0);
    assert_eq!(
        shown(&mut cx, window),
        (
            Some(("Action annulée.".to_owned(), NoticeKind::Success, false)),
            nb("5~h~04~min"),
            Some(hint)
        )
    );
}

/// « Voir sur la frise » opens the week on the hint's day with its first gap selected, as a
/// break lies between the two, and its bubble open.
#[test]
fn showing_the_unrecorded_time_opens_the_week_on_it() {
    let (mut cx, window) = open();
    tab_to_part(&mut cx, window, 0);
    common::press(&mut cx, window, "tab");
    common::press(&mut cx, window, "enter");
    let week = cx
        .update(|cx| cx.windows())
        .into_iter()
        .find_map(|window| window.downcast::<WeekView>())
        .expect("the week opens");
    let (selected, bubble) = cx
        .update(|cx| {
            week.update(cx, |view, _, _| {
                let state = view.state_mut().expect("the demo week reads");
                (
                    state.selection().map(|selection| selection.range),
                    state.bubble().map(|bubble| bubble.title),
                )
            })
        })
        .unwrap();
    assert_eq!(
        selected,
        UtcInterval::new(
            "2026-08-31T09:40:12Z".parse().unwrap(),
            "2026-08-31T09:45:42Z".parse().unwrap()
        )
        .ok()
    );
    assert!(bubble.is_some(), "the bubble opens on it");
}
