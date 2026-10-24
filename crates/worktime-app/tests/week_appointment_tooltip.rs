//! The tooltip of an appointment's bracket over the week's timeline shows while the pointer
//! rests on the bracket, and goes once it leaves, the bubble opens or the open day changes. It
//! needs the headless GPUI of the `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use gpui::{
    HeadlessAppContext, Modifiers, MouseMoveEvent, Pixels, PlatformInput, Point, WindowHandle,
    point, px, size,
};
use jiff::civil::{Weekday, time};
use worktime_app::demo::{self, DemoHost};
use worktime_app::strings::Language;
use worktime_app::theme::size as token;
use worktime_app::week_view::{self, WeekView};
use worktime_core::demo::DemoScenario;
use worktime_domain::configuration::{RecurringAppointment, WorkTimeSettings};

/// The history week, open on today, Friday, with an appointment from 17:00 past now.
fn open() -> (HeadlessAppContext, WindowHandle<WeekView>) {
    let mut cx = common::app(true, week_view::bind_keys);
    let directory = demo::throwaway_directory();
    let week = DemoHost::open(DemoScenario::HistoryWeek, &directory).and_then(|mut host| {
        let settings = WorkTimeSettings {
            recurring_appointments: vec![RecurringAppointment {
                id: "friday".to_owned(),
                name: "Réunion d’équipe".to_owned(),
                weekday: Weekday::Friday,
                start: time(17, 0, 0, 0),
                end: time(19, 30, 0, 0),
                enabled: true,
                since: None,
                alternate_weeks_from: None,
                automatic: false,
            }],
            ..host.settings().clone()
        };
        host.save_settings(&settings)?;
        host.refresh(Language::French)?;
        host.week()
    });
    std::fs::remove_dir_all(&directory).ok();
    let week = week.unwrap();
    let window = common::open(
        &mut cx,
        size(token::WEEK_WINDOW_WIDTH, token::WEEK_WINDOW_HEIGHT),
        |window, cx| WeekView::new(Some(week), Language::French, window, cx),
    );
    (cx, window)
}

/// The middle of Friday's bracket, above its strip.
fn bracket(cx: &mut HeadlessAppContext, window: WindowHandle<WeekView>) -> Point<Pixels> {
    window
        .update(cx, |view, _, _| {
            let strip = view.strip_bounds()?;
            let friday = view
                .state_mut()?
                .page()
                .days
                .into_iter()
                .find(|row| row.is_today)?;
            let marker = friday.markers.first()?;
            Some(point(
                strip.origin.x
                    + px((marker.start + marker.length / 2.) * f32::from(strip.size.width)),
                strip.origin.y
                    - token::APPOINTMENT_MARKER_GAP_FOCUSED
                    - token::APPOINTMENT_MARKER_HEIGHT / 2.,
            ))
        })
        .unwrap()
        .expect("Friday draws its appointment's bracket")
}

fn move_to(cx: &mut HeadlessAppContext, window: WindowHandle<WeekView>, position: Point<Pixels>) {
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(
            PlatformInput::MouseMove(MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Modifiers::default(),
            }),
            cx,
        );
    })
    .unwrap();
    common::redraw(cx, window);
}

fn tooltip(cx: &mut HeadlessAppContext, window: WindowHandle<WeekView>) -> Option<Vec<String>> {
    window
        .read_with(cx, |view, _| view.appointment_tooltip())
        .unwrap()
}

fn shown() -> Option<Vec<String>> {
    Some(vec![
        "Réunion d’équipe, horaire prévu 17:00–19:30".to_owned(),
    ])
}

#[test]
fn the_tooltip_shows_on_the_bracket_and_goes_when_the_pointer_leaves() {
    let (mut cx, window) = open();
    assert_eq!(tooltip(&mut cx, window), None);
    let on = bracket(&mut cx, window);
    move_to(&mut cx, window, on);
    assert_eq!(tooltip(&mut cx, window), shown());
    // Still within the 14 px the pointer finds it on, 6 px above the bracket's middle.
    move_to(&mut cx, window, on - point(px(0.), px(6.)));
    assert_eq!(tooltip(&mut cx, window), shown());
    move_to(&mut cx, window, on - point(px(0.), px(9.)));
    assert_eq!(tooltip(&mut cx, window), None);
}

/// The bubble opens on Friday itself, whose bracket stays under the pointer, as the host opens
/// it without a key or a click: the pointer never leaves the bracket.
#[test]
fn the_tooltip_goes_when_the_bubble_opens() {
    let (mut cx, window) = open();
    let on = bracket(&mut cx, window);
    move_to(&mut cx, window, on);
    assert_eq!(tooltip(&mut cx, window), shown());
    let opened = window
        .update(&mut cx, |view, _, cx| {
            let state = view.state_mut().unwrap();
            let friday = state.timeline.days.last().unwrap();
            let review = friday.segments.last().unwrap().id(friday.date);
            state.open(review);
            cx.notify();
            state.bubble().map(|bubble| bubble.selection.date)
        })
        .unwrap();
    common::redraw(&mut cx, window);
    assert_eq!(opened, Some(jiff::civil::date(2026, 9, 4)));
    assert_eq!(
        bracket(&mut cx, window),
        on,
        "the bracket stays where it was"
    );
    assert_eq!(tooltip(&mut cx, window), None);
}

#[test]
fn the_tooltip_goes_when_the_open_day_changes() {
    let (mut cx, window) = open();
    let on = bracket(&mut cx, window);
    move_to(&mut cx, window, on);
    assert_eq!(tooltip(&mut cx, window), shown());
    common::press(&mut cx, window, "up");
    assert_eq!(tooltip(&mut cx, window), None);
}
