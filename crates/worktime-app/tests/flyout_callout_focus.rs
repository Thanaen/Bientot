//! A callout of an appointment closed from the keyboard hands the focus to the next part of the
//! flyout, not back to its first control (#12). It needs the headless GPUI of the `screenshots`
//! feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{Focusable as _, HeadlessAppContext, WindowHandle, px, size};
use jiff::tz::TimeZone;
use worktime_app::automatic::{self, CountedAbsence, TrustOffer};
use worktime_app::demo::{self, DemoHost};
use worktime_app::flyout::{self, Flyout, HideCount, HideTrustOffer};
use worktime_app::live::Tracking;
use worktime_app::strings::Language;
use worktime_app::theme::size::FLYOUT_WIDTH;
use worktime_app::workday::DayView;
use worktime_core::demo::DemoScenario;
use worktime_domain::accounting::{LongLockReviewId, TrackingState};
use worktime_domain::intervals::UtcInterval;
use worktime_domain::time::LocalWorkday;

/// The demo day with an absence counted automatically and the offer to count the meeting so.
fn day_with_callouts() -> DayView {
    let directory = demo::throwaway_directory();
    let mut host = DemoHost::open(DemoScenario::OnTrack, &directory).unwrap();
    let mut day = host.refresh(Language::French).unwrap();
    std::fs::remove_dir_all(&directory).ok();
    let today = host.today();
    let at = |hour| {
        today
            .at(hour, 0, 0, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp()
    };
    let (start, end) = (at(14), at(15));
    let workday = LocalWorkday::new(today, "UTC");
    let review = LongLockReviewId::for_appointment("thursday-1400", start);
    day.counted_absence = Some(CountedAbsence {
        adjustment_id: automatic::count_id(&workday, &review),
        workday,
        interval: UtcInterval::new(start, end).unwrap(),
        local_start: jiff::civil::time(14, 0, 0, 0),
        local_end: jiff::civil::time(15, 0, 0, 0),
        duration: end.duration_since(start),
        appointment: "Réunion".to_owned(),
    });
    day.trust_offer = Some(TrustOffer {
        appointment_id: "thursday-1400".to_owned(),
        appointment: "Réunion".to_owned(),
        weekday: jiff::civil::Weekday::Thursday,
        streak: automatic::TRUST_STREAK,
    });
    day
}

/// The flyout of `day`, and the count of crosses pressed on its callouts.
fn open(day: DayView) -> (HeadlessAppContext, WindowHandle<Flyout>, Rc<Cell<u32>>) {
    let mut cx = common::app(true, flyout::bind_keys);
    let closed = Rc::new(Cell::new(0));
    cx.update(|cx| {
        let counted = closed.clone();
        cx.on_action(move |_: &HideCount, _| counted.set(counted.get() + 1));
        let offered = closed.clone();
        cx.on_action(move |_: &HideTrustOffer, _| offered.set(offered.get() + 1));
    });
    let window = common::open(&mut cx, size(FLYOUT_WIDTH, px(800.)), |window, cx| {
        Flyout::new(
            day,
            Tracking::new(TrackingState::Running, true),
            None,
            window,
            cx,
        )
    });
    (cx, window, closed)
}

fn focused_part(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> Option<usize> {
    cx.update_window(window.into(), |_, window, cx| {
        window
            .root::<Flyout>()
            .flatten()
            .and_then(|flyout| flyout.read(cx).focused_part(window, cx))
    })
    .unwrap()
}

fn flyout_has_focus(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> bool {
    cx.update_window(window.into(), |_, window, cx| {
        window
            .root::<Flyout>()
            .flatten()
            .is_some_and(|flyout| flyout.read(cx).focus_handle(cx).is_focused(window))
    })
    .unwrap()
}

/// Tab until Enter presses a cross, which `closed` tells; the part it is in.
fn close_with_the_keyboard(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<Flyout>,
    closed: &Cell<u32>,
) -> usize {
    for _ in 0..32 {
        common::press(cx, window, "tab");
        let part = focused_part(cx, window);
        common::press(cx, window, "enter");
        if closed.get() == 1 {
            return part.expect("the cross is in a part of the body");
        }
    }
    panic!("Tab does not reach the cross of a callout");
}

/// The day shown again, as the host shows it once the callout is closed, then two frames: the
/// one without the callout, and the next, where the focus moves.
fn show(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>, day: DayView) {
    cx.update(|cx| {
        window
            .update(cx, |flyout, _, cx| {
                flyout.show(
                    day,
                    Tracking::new(TrackingState::Running, true),
                    false,
                    None,
                    cx,
                )
            })
            .unwrap();
    });
    common::redraw(cx, window);
    common::next_frame(cx, window.into());
}

#[test]
fn closing_the_counted_absence_hands_the_focus_to_the_offer_after_it() {
    let day = day_with_callouts();
    let (mut cx, window, closed) = open(day.clone());

    let part = close_with_the_keyboard(&mut cx, window, &closed);
    show(
        &mut cx,
        window,
        DayView {
            counted_absence: None,
            ..day
        },
    );

    assert!(!flyout_has_focus(&mut cx, window));
    assert_eq!(focused_part(&mut cx, window), Some(part));
}

#[test]
fn a_callout_still_shown_keeps_the_focus_on_its_cross() {
    let day = day_with_callouts();
    let (mut cx, window, closed) = open(day.clone());

    let part = close_with_the_keyboard(&mut cx, window, &closed);
    show(&mut cx, window, day);

    assert_eq!(focused_part(&mut cx, window), Some(part));
}
