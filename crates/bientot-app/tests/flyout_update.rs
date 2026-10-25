//! « Mettre à jour » pressed with the keyboard: the flyout keeps the focus while the Store
//! installs, then hands it to « Réessayer ». It needs the headless GPUI of the `screenshots`
//! feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use bientot_app::assets::{self, Assets};
use bientot_app::demo::{self, DemoHost};
use bientot_app::flyout::{self, Flyout, InstallUpdate};
use bientot_app::live::Tracking;
use bientot_app::strings::Language;
use bientot_app::theme::ThemeMode;
use bientot_app::theme::size::FLYOUT_WIDTH;
use bientot_app::updates::{DemoOffer, InstallFailure, Offer, UpdateCallout, Updates};
use bientot_core::demo::DemoScenario;
use bientot_domain::accounting::TrackingState;
use gpui::{
    AppContext as _, BorrowAppContext as _, FocusHandle, HeadlessAppContext, KeyDownEvent,
    KeyUpEvent, Keystroke, PlatformInput, WindowHandle, px, size,
};

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

fn focused(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> Option<FocusHandle> {
    cx.update_window(window.into(), |_, window, cx| window.focused(cx))
        .unwrap()
}

/// Whether Tab still moves the focus from the focused element, as it does in the flyout.
fn tab_moves_on(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> bool {
    let before = focused(cx, window);
    press(cx, window, "tab");
    let moved = focused(cx, window) != before;
    if let Some(before) = before {
        cx.update_window(window.into(), |_, window, cx| window.focus(&before, cx))
            .unwrap();
        redraw(cx, window);
    }
    moved
}

/// The flyout with a Store update on offer, and the count of « Mettre à jour » presses.
fn open() -> (HeadlessAppContext, WindowHandle<Flyout>, Rc<Cell<u32>>) {
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
        cx.on_action(move |_: &InstallUpdate, cx| {
            installs.set(installs.get() + 1);
            cx.update_global::<Updates, _>(|updates, _| {
                updates.install();
            });
        });
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
                Flyout::new(
                    day,
                    Tracking::new(TrackingState::Running, true),
                    None,
                    window,
                    cx,
                )
            })
        })
        .unwrap();
    redraw(&mut cx, window);
    (cx, window, installs)
}

/// Tabs to « Mettre à jour » and presses it with Enter; returns the button's focus.
fn press_update(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<Flyout>,
    installs: &Cell<u32>,
) -> FocusHandle {
    for _ in 0..32 {
        press(cx, window, "tab");
        let candidate = focused(cx, window);
        press(cx, window, "enter");
        if installs.get() == 1 {
            return candidate.expect("« Mettre à jour » has the focus");
        }
    }
    panic!("Tab reaches « Mettre à jour »");
}

#[test]
fn installing_keeps_the_focus_on_the_update_button() {
    let (mut cx, window, installs) = open();
    let button = press_update(&mut cx, window, &installs);
    assert_eq!(
        cx.update(|cx| cx.global::<Updates>().callout()),
        Some(UpdateCallout::Installing)
    );
    assert_eq!(focused(&mut cx, window), Some(button.clone()));
    assert!(tab_moves_on(&mut cx, window), "focus lost while installing");

    press(&mut cx, window, "enter");
    assert_eq!(installs.get(), 1, "« Installation… » does nothing");

    cx.update(|cx| {
        cx.update_global::<Updates, _>(|updates, _| {
            updates.installed(&Err(InstallFailure {
                error_type: "test",
                message: "the Store refused".to_owned(),
            }));
        });
    });
    redraw(&mut cx, window);
    assert_eq!(focused(&mut cx, window), Some(button));
    assert!(tab_moves_on(&mut cx, window), "focus lost on « Réessayer »");
}

#[test]
fn an_installed_update_hands_the_focus_back_to_the_flyout() {
    let (mut cx, window, installs) = open();
    press_update(&mut cx, window, &installs);
    cx.update(|cx| {
        cx.update_global::<Updates, _>(|updates, _| updates.installed(&Ok(())));
    });
    redraw(&mut cx, window);
    assert_eq!(cx.update(|cx| cx.global::<Updates>().callout()), None);
    assert!(tab_moves_on(&mut cx, window), "Tab lost with the callout");

    let escape = Keystroke::parse("escape").unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke: escape,
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        cx.update_window(window.into(), |_, _, _| ()).is_err(),
        "Échap no longer closes the flyout"
    );
}
