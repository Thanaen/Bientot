//! A flyout taller than its work area allows: its window stops at the panel's allowed height
//! and Tab still reaches the controls of its scrolled body, scrolled into view. A flyout that
//! fits keeps the height of its content, and a click on its text leaves the focus with it. It
//! needs the headless GPUI of the `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Bounds, Focusable as _, HeadlessAppContext, KeyDownEvent, KeyUpEvent,
    Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, PlatformInput, Point,
    ScrollHandle, WindowHandle, px, size,
};
use worktime_app::assets::{self, Assets};
use worktime_app::demo::{self, DemoHost};
use worktime_app::flyout::{self, Alerts, Flyout, InstallUpdate, OpenGeneralSettings};
use worktime_app::live::Tracking;
use worktime_app::strings::Language;
use worktime_app::theme::ThemeMode;
use worktime_app::theme::size::FLYOUT_WIDTH;
use worktime_app::updates::{DemoOffer, Offer, Updates};
use worktime_app::widgets::Notice;
use worktime_core::demo::DemoScenario;
use worktime_domain::accounting::TrackingState;

/// The flyout with everything it can tell at once: two notices from the start, a failed
/// refresh and a Store update on offer, its panel no taller than `max_height` if given. Also
/// the count of « Mettre à jour » presses.
fn open(max_height: Option<Pixels>) -> (HeadlessAppContext, WindowHandle<Flyout>, Rc<Cell<u32>>) {
    let alerts = Alerts {
        announcements: vec![
            Notice::settings_recovered("Les réglages ont été restaurés."),
            Notice::warning("Les notifications ne sont pas disponibles."),
        ],
        refresh_failure: Some(Notice::refresh_pending("L’actualisation a échoué.")),
    };
    open_with(alerts, max_height)
}

/// The flyout of `alerts` and a Store update on offer, its panel no taller than `max_height` if
/// given. Also the count of « Mettre à jour » presses.
fn open_with(
    alerts: Alerts,
    max_height: Option<Pixels>,
) -> (HeadlessAppContext, WindowHandle<Flyout>, Rc<Cell<u32>>) {
    let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(
        assets::FONT_FAMILY,
    ));
    let mut cx = HeadlessAppContext::with_asset_source(text_system, Arc::new(Assets));
    let installs = Rc::new(Cell::new(0));
    cx.update(|cx| {
        cx.set_global(ThemeMode::Light);
        cx.set_global(Language::French);
        assets::load_fonts(cx).unwrap();
        flyout::bind_keys(cx);
        let mut updates = Updates::default();
        updates.found(Offer::new(DemoOffer));
        cx.set_global(updates);
        let installs = installs.clone();
        cx.on_action(move |_: &InstallUpdate, _| installs.set(installs.get() + 1));
    });
    let directory = demo::throwaway_directory();
    let day = DemoHost::open(DemoScenario::OnTrack, &directory)
        .unwrap()
        .refresh(Language::French);
    std::fs::remove_dir_all(&directory).ok();
    let day = day.unwrap();
    let window = cx
        .open_window(size(FLYOUT_WIDTH, px(800.)), |window, cx| {
            cx.new(|cx| {
                let flyout = Flyout::new(
                    day,
                    Tracking::new(TrackingState::Running, true),
                    None,
                    window,
                    cx,
                )
                .with_alerts(alerts);
                match max_height {
                    Some(height) => flyout.with_max_height(height),
                    None => flyout,
                }
            })
        })
        .unwrap();
    redraw(&mut cx, window);
    (cx, window, installs)
}

fn redraw(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) {
    cx.update_window(window.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
}

fn press(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>, key: &str) {
    let keystroke = Keystroke::parse(key).unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
        window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke }), cx);
    })
    .unwrap();
    redraw(cx, window);
}

fn window_height(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> Pixels {
    cx.update_window(window.into(), |_, window, _| window.bounds().size.height)
        .unwrap()
}

#[test]
fn a_flyout_taller_than_its_work_area_stops_at_the_height_the_panel_is_allowed() {
    let (mut cx, window, _) = open(None);
    let natural = window_height(&mut cx, window);
    let allowed = px(450.);
    assert!(
        natural > allowed,
        "the flyout of every alert is short already ({natural:?})"
    );

    let (mut cx, window, _) = open(Some(allowed));
    assert_eq!(window_height(&mut cx, window), allowed);
}

#[test]
fn a_flyout_that_fits_its_work_area_keeps_the_height_of_its_content() {
    let (mut cx, window, _) = open(None);
    let natural = window_height(&mut cx, window);
    let (mut cx, window, _) = open(Some(natural + px(100.)));
    assert_eq!(window_height(&mut cx, window), natural);
}

/// Tab until « Mettre à jour » has the focus, which Enter after each Tab tells.
fn tab_to_update(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>, installs: &Cell<u32>) {
    for _ in 0..32 {
        press(cx, window, "tab");
        press(cx, window, "enter");
        if installs.get() == 1 {
            return;
        }
    }
    panic!("Tab does not reach « Mettre à jour »");
}

fn click(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>, position: Point<Pixels>) {
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers: Modifiers::default(),
                click_count: 1,
            }),
            cx,
        );
    })
    .unwrap();
    redraw(cx, window);
}

fn body_scroll(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> ScrollHandle {
    window
        .read_with(cx, |flyout, _| flyout.body_scroll().clone())
        .unwrap()
}

fn focused_body_control(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<Flyout>,
) -> Option<Bounds<Pixels>> {
    window
        .update(cx, |flyout, window, cx| {
            flyout.focused_body_control(window, cx)
        })
        .unwrap()
}

fn flyout_has_focus(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> bool {
    window
        .update(cx, |flyout, window, cx| {
            flyout.focus_handle(cx).is_focused(window)
        })
        .unwrap()
}

/// How far the parts of the body shown whole stand from its edges: the gap over the topmost
/// one and the gap under the lowest.
fn margins(body: &ScrollHandle) -> (Pixels, Pixels) {
    let (viewport, offset) = (body.bounds(), body.offset().y);
    let shown: Vec<_> = (0..body.children_count())
        .filter_map(|ix| body.bounds_for_item(ix))
        .map(|part| (part.top() + offset, part.bottom() + offset))
        .filter(|&(top, bottom)| {
            top >= viewport.top() - px(0.5) && bottom <= viewport.bottom() + px(0.5)
        })
        .collect();
    let (top, _) = shown.first().copied().unwrap();
    let (_, bottom) = shown.last().copied().unwrap();
    (top - viewport.top(), viewport.bottom() - bottom)
}

fn assert_near(actual: Pixels, expected: Pixels, what: &str) {
    assert!(
        (actual - expected).abs() < px(0.5),
        "{what}: {actual:?}, not {expected:?}"
    );
}

#[test]
fn tab_scrolls_the_update_below_the_fold_of_a_short_flyout_into_view() {
    let (mut cx, window, installs) = open(Some(px(450.)));
    let body = body_scroll(&mut cx, window);
    assert_eq!(body.offset().y, px(0.));
    tab_to_update(&mut cx, window, &installs);
    assert!(
        body.offset().y < px(0.),
        "« Mettre à jour » has the focus out of view"
    );
}

#[test]
fn a_control_tab_reveals_keeps_the_body_padding_beside_it() {
    let (mut cx, window, installs) = open(Some(px(450.)));
    let body = body_scroll(&mut cx, window);
    tab_to_update(&mut cx, window, &installs);
    assert_near(
        margins(&body).1,
        flyout::BODY_PADDING,
        "under the update revealed",
    );

    let below_the_fold = body.offset().y;
    for _ in 0..32 {
        press(&mut cx, window, "shift-tab");
        if body.offset().y != below_the_fold {
            assert_near(
                margins(&body).0,
                flyout::BODY_PADDING,
                "over the part Maj+Tab revealed",
            );
            return;
        }
    }
    panic!("Maj+Tab never scrolls the body back up");
}

#[test]
fn a_click_on_the_text_of_the_flyout_leaves_the_focus_with_it() {
    let (mut cx, window, _) = open(None);
    assert!(flyout_has_focus(&mut cx, window));
    let body = body_scroll(&mut cx, window);
    let privacy = body.bounds_for_item(body.children_count() - 1).unwrap();
    click(&mut cx, window, privacy.center());
    assert!(
        flyout_has_focus(&mut cx, window),
        "the privacy line took the focus"
    );
}

#[test]
fn tab_reveals_the_button_at_the_bottom_of_a_notice_taller_than_the_body() {
    let message = "Les réglages enregistrés étaient illisibles : WorkTime a repris les derniers \
                   réglages valides. "
        .repeat(12);
    let alerts = Alerts {
        announcements: vec![Notice::settings_recovered(message)],
        refresh_failure: None,
    };
    let (mut cx, window, _) = open_with(alerts, Some(px(450.)));
    let opened = Rc::new(Cell::new(0));
    cx.update(|cx| {
        let opened = opened.clone();
        cx.on_action(move |_: &OpenGeneralSettings, _| opened.set(opened.get() + 1));
    });
    let body = body_scroll(&mut cx, window);
    let viewport = body.bounds();
    let notice = (0..body.children_count())
        .find(|&ix| {
            body.bounds_for_item(ix)
                .is_some_and(|part| part.size.height > viewport.size.height)
        })
        .expect("no part of the body is taller than it");

    for _ in 0..32 {
        press(&mut cx, window, "tab");
        let shown = body.bounds_for_item(notice).unwrap() + Point::new(px(0.), body.offset().y);
        let Some(button) =
            focused_body_control(&mut cx, window).filter(|button| shown.contains(&button.center()))
        else {
            continue;
        };
        assert!(
            button.top() >= viewport.top() && button.bottom() <= viewport.bottom(),
            "« Ouvrir les paramètres », at the bottom of the notice, has the focus out of view \
             ({button:?} outside {viewport:?})"
        );
        press(&mut cx, window, "enter");
        assert_eq!(
            opened.get(),
            1,
            "the focused button is not « Ouvrir les paramètres »"
        );
        return;
    }
    panic!("Tab does not reach « Ouvrir les paramètres »");
}
