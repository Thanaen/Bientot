//! A notice that an answer brings to the open flyout comes in over `motion::MENU`, then leaves
//! the window idle; one the flyout opens with shows at rest. It needs the headless GPUI of the
//! `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use bientot_app::demo::{self, DemoHost};
use bientot_app::flyout::{self, Flyout};
use bientot_app::host::{Source, Today};
use bientot_app::live::Tracking;
use bientot_app::shell;
use bientot_app::strings::Language;
use bientot_app::theme::motion;
use bientot_app::theme::size::FLYOUT_WIDTH;
use bientot_app::widgets::Notice;
use bientot_core::demo::DemoScenario;
use bientot_domain::accounting::TrackingState;
use gpui::{HeadlessAppContext, WindowHandle, px, size};

fn demo_host() -> DemoHost {
    let directory = demo::throwaway_directory();
    let host = DemoHost::open(DemoScenario::OnTrack, &directory).unwrap();
    std::fs::remove_dir_all(&directory).ok();
    host
}

/// The flyout, opened without a notice.
fn open(reduce_motion: bool) -> (HeadlessAppContext, WindowHandle<Flyout>) {
    let mut cx = common::app(reduce_motion, flyout::bind_keys);
    let day = demo_host().refresh(Language::French).unwrap();
    let window = common::open(&mut cx, size(FLYOUT_WIDTH, px(800.)), |window, cx| {
        Flyout::new(
            day,
            Tracking::new(TrackingState::Running, true),
            None,
            window,
            cx,
        )
    });
    next_frame(&mut cx, window);
    (cx, window)
}

fn saved() -> Notice {
    Notice::success("Diagnostic enregistré.")
}

fn answer(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>, notice: Notice) {
    cx.update(|cx| {
        window
            .update(cx, |flyout, _, cx| flyout.set_notice(notice, cx))
            .unwrap();
    });
    cx.run_until_parked();
}

fn next_frame(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> usize {
    common::next_frame(cx, window.into())
}

fn assert_comes_in(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>, which: &str) {
    common::assert_entrance_runs(
        cx,
        window.into(),
        motion::MENU,
        &format!("the {which} notice"),
    );
}

#[test]
fn an_answer_in_the_open_flyout_comes_in_each_time_then_rests() {
    let (mut cx, window) = open(false);
    assert_eq!(next_frame(&mut cx, window), 0, "the flyout is busy before");
    for which in ["first", "second"] {
        answer(&mut cx, window, saved());
        assert_comes_in(&mut cx, window, which);
    }
}

#[test]
fn a_notice_the_flyout_opens_with_shows_at_rest() {
    let mut cx = common::app(false, flyout::bind_keys);
    let today = Today::open(Source::Demo(Box::new(demo_host())), Language::French).unwrap();
    cx.update(|cx| {
        cx.set_global(today);
        shell::open_flyout_with(saved(), cx);
    });
    cx.run_until_parked();
    let window = cx.update(|cx| cx.windows()).into_iter().next().unwrap();
    assert_eq!(
        common::next_frame(&mut cx, window),
        0,
        "the notice comes in at opening"
    );

    cx.update(|cx| shell::open_flyout_with(Notice::error("L’export a échoué."), cx));
    cx.run_until_parked();
    common::assert_entrance_runs(&mut cx, window, motion::MENU, "the later notice");
}

#[test]
fn a_failed_switch_comes_in_and_a_successful_one_leaves_the_notice_still() {
    let (mut cx, window) = open(false);
    let failure = Notice::source_switch_failed("La détection intégrée n’a pas été enregistrée.");
    cx.update(|cx| {
        window
            .update(cx, |flyout, _, cx| flyout.switched(Some(failure), cx))
            .unwrap();
    });
    cx.run_until_parked();
    assert_comes_in(&mut cx, window, "failed switch");

    answer(&mut cx, window, saved());
    assert_comes_in(&mut cx, window, "success");
    cx.update(|cx| {
        window
            .update(cx, |flyout, _, cx| flyout.switched(None, cx))
            .unwrap();
    });
    cx.run_until_parked();
    assert_eq!(
        next_frame(&mut cx, window),
        0,
        "the kept notice comes in again"
    );
}

fn refreshed(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>, failure: Option<Notice>) {
    let day = demo_host().refresh(Language::French).unwrap();
    cx.update(|cx| {
        window
            .update(cx, |flyout, _, cx| {
                flyout.show(
                    day,
                    Tracking::new(TrackingState::Running, true),
                    failure.is_none(),
                    failure,
                    cx,
                )
            })
            .unwrap();
    });
    cx.run_until_parked();
}

#[test]
fn a_refresh_that_fails_in_the_open_flyout_comes_in_once() {
    let (mut cx, window) = open(false);
    let failure = Notice::refresh_pending("WorkTime n’a pas pu actualiser les données.");
    refreshed(&mut cx, window, Some(failure.clone()));
    assert_comes_in(&mut cx, window, "refresh failure");

    refreshed(&mut cx, window, Some(failure.clone()));
    assert_eq!(
        next_frame(&mut cx, window),
        0,
        "the failure shown comes in again"
    );

    refreshed(&mut cx, window, None);
    refreshed(&mut cx, window, Some(failure));
    assert_comes_in(&mut cx, window, "next refresh failure");
}

#[test]
fn with_reduced_motion_an_answer_shows_at_rest() {
    let (mut cx, window) = open(true);
    answer(&mut cx, window, saved());
    assert_eq!(next_frame(&mut cx, window), 0);

    cx.update(|cx| cx.set_reduce_motion(false));
    answer(&mut cx, window, saved());
    assert_comes_in(&mut cx, window, "next");
}
