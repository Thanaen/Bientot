//! A click on the minutes of a time field leaves the focus on them, so digits typed next
//! change the minutes (#295): in « Déclarer du temps travaillé » and in the schedule of
//! « Paramètres ». It needs the headless GPUI of the `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use gpui::{HeadlessAppContext, Pixels, Point, Size, WindowHandle, point, px};

use common::{click, mouse_down, mouse_down_event, mouse_up, press, redraw};
use jiff::tz::TimeZone;
use worktime_app::adjust::window::Opener;
use worktime_app::adjust::{AdjustForm, Seed, Target, TimePart};
use worktime_app::adjust_view::{self, AdjustView};
use worktime_app::settings::{Clock, Page, SettingsForm, UsagePingFacts};
use worktime_app::settings_view::{self, SettingsView};
use worktime_app::strings::Language;
use worktime_domain::accounting::AdjustmentId;
use worktime_domain::time::LocalWorkday;
use worktime_domain::usage_ping::{Build, Channel};

/// A point of the minutes `minutes` reads, where a press of the mouse gives them the focus,
/// which ↑ then tells. Only that press is released, so no button of the window is clicked.
fn point_on_minutes<V: 'static>(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<V>,
    size: Size<Pixels>,
    minutes: impl Fn(&mut HeadlessAppContext) -> i8,
) -> Point<Pixels> {
    let before = minutes(cx);
    // One press on each control that takes the focus, all in one update: each update draws.
    let presses = cx
        .update_window(window.into(), |_, window, cx| {
            let mut focuses = vec![window.focused(cx)];
            let mut presses = Vec::new();
            for y in (0..f32::from(size.height) as i32).step_by(2) {
                for x in (0..f32::from(size.width) as i32).step_by(2) {
                    let position = point(px(x as f32), px(y as f32));
                    window.dispatch_event(mouse_down_event(position), cx);
                    let focus = window.focused(cx);
                    if !focuses.contains(&focus) {
                        focuses.push(focus);
                        presses.push(position);
                    }
                }
            }
            presses
        })
        .unwrap();
    for position in presses {
        mouse_down(cx, window, position);
        press(cx, window, "up");
        let stepped = minutes(cx) != before;
        press(cx, window, "down");
        if stepped {
            assert_eq!(minutes(cx), before, "↓ does not undo ↑");
            mouse_up(cx, window, position);
            redraw(cx, window);
            return position;
        }
    }
    panic!("no press of the mouse gives the minutes the focus");
}

#[test]
fn a_click_on_the_minutes_of_a_declaration_lets_digits_change_them() {
    let mut cx = common::app(false, adjust_view::bind_keys);
    let workday = LocalWorkday::new(jiff::civil::date(2026, 10, 8), "UTC");
    let target = Target {
        workday,
        zone: TimeZone::UTC,
    };
    let id = AdjustmentId::new("click").unwrap();
    let form = AdjustForm::new(
        target,
        Seed::DeclareWork(None),
        id,
        jiff::Timestamp::UNIX_EPOCH,
        Language::French,
    );
    let size = gpui::size(px(adjust_view::WINDOW_WIDTH), px(form.window_height()));
    let window = common::open(&mut cx, size, |window, cx| {
        AdjustView::new(form, Opener::Week, window, cx)
    });
    let start = |cx: &mut HeadlessAppContext| {
        window
            .update(cx, |view, _, _| view.form_mut().start.text())
            .unwrap()
    };
    assert_eq!(start(&mut cx), "09:00");

    let minutes = point_on_minutes(&mut cx, window, size, |cx| {
        window
            .update(cx, |view, _, _| {
                view.form_mut().start.value(TimePart::Minutes)
            })
            .unwrap()
    });
    click(&mut cx, window, minutes);
    press(&mut cx, window, "4");
    press(&mut cx, window, "5");
    assert_eq!(
        start(&mut cx),
        "09:45",
        "the click on the minutes gave the focus back to the hours"
    );

    let beside = point(minutes.x + px(80.), minutes.y);
    click(&mut cx, window, beside);
    press(&mut cx, window, "1");
    press(&mut cx, window, "4");
    assert_eq!(
        start(&mut cx),
        "14:45",
        "a click in the field beside its parts does not give the hours the focus"
    );
}

#[test]
fn a_click_on_the_minutes_of_the_schedule_lets_digits_change_them() {
    let mut cx = common::app(false, settings_view::bind_keys);
    let usage_ping = UsagePingFacts {
        build: Build::new("1.0.0", Channel::Store).unwrap(),
        released: None,
        today: jiff::civil::date(2026, 10, 13),
        last_sent: None,
    };
    let mut form = SettingsForm::new(Default::default(), usage_ping, Language::French);
    form.go_to(Page::Schedule);
    let size = gpui::size(settings_view::WINDOW_WIDTH, settings_view::WINDOW_HEIGHT);
    let window = common::open(&mut cx, size, |window, cx| {
        SettingsView::new(form, None, window, cx)
    });
    let lunch_start = |cx: &mut HeadlessAppContext| {
        cx.update(|cx| window.read(cx).unwrap().form().time(Clock::LunchStart))
    };
    assert_eq!(lunch_start(&mut cx).text(), "12:00");

    let minutes = point_on_minutes(&mut cx, window, size, |cx| {
        lunch_start(cx).value(TimePart::Minutes)
    });
    click(&mut cx, window, minutes);
    press(&mut cx, window, "4");
    press(&mut cx, window, "5");
    assert_eq!(
        lunch_start(&mut cx).text(),
        "12:45",
        "the click on the minutes gave the focus back to the hours"
    );

    let beside = point(minutes.x + px(40.), minutes.y);
    click(&mut cx, window, beside);
    press(&mut cx, window, "1");
    press(&mut cx, window, "4");
    assert_eq!(
        lunch_start(&mut cx).text(),
        "14:45",
        "a click in the field beside its parts does not give the hours the focus"
    );
}
