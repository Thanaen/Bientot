//! The « … » of a plugin card opens its menu and a second click on it closes the menu, which
//! does not open again on the release. A click outside closes it too, and never leaves the
//! focus on an item that is no longer drawn. It needs the headless GPUI of the `screenshots`
//! feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use bientot_app::demo::{self, DemoHost};
use bientot_app::flyout::{self, Flyout, PluginMenuState};
use bientot_app::live::Tracking;
use bientot_app::plugins::{PluginCard, PluginCards};
use bientot_app::strings::Language;
use bientot_app::theme::size::FLYOUT_WIDTH;
use bientot_core::demo::DemoScenario;
use bientot_domain::accounting::TrackingState;
use bientot_plugin_api::PluginId;
use bientot_plugin_api::view::{Prepared, PreparedCard, Role};
use gpui::{HeadlessAppContext, Pixels, Point, WindowHandle, point, px, size};

const HEIGHT: f32 = 800.;

fn card(name: &str) -> PluginCard {
    PluginCard {
        name: name.to_owned().into(),
        card: PreparedCard {
            lines: vec![Prepared::Text {
                text: "Une ligne".into(),
                role: Role::Body,
            }],
            actions: Vec::new(),
        },
    }
}

fn open() -> (HeadlessAppContext, WindowHandle<Flyout>) {
    let mut cx = common::app(true, flyout::bind_keys);
    let (shown, other) = (
        PluginId::parse("someone.first").unwrap(),
        PluginId::parse("someone.second").unwrap(),
    );
    let cards = PluginCards {
        cards: Arc::new(BTreeMap::from([
            (shown.clone(), card("Premier")),
            (other.clone(), card("Second")),
        ])),
        shown: Some(shown),
        more: vec![other],
        ..PluginCards::default()
    };
    cx.update(|cx| cx.set_global(cards));
    let directory = demo::throwaway_directory();
    let day = DemoHost::open(DemoScenario::OnTrack, &directory)
        .unwrap()
        .refresh(Language::French)
        .unwrap();
    std::fs::remove_dir_all(&directory).ok();
    let window = common::open(&mut cx, size(FLYOUT_WIDTH, px(HEIGHT)), |window, cx| {
        Flyout::new(
            day,
            Tracking::new(TrackingState::Running, true),
            None,
            window,
            cx,
        )
    });
    (cx, window)
}

fn state(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> PluginMenuState {
    cx.update_window(window.into(), |_, window, cx| {
        window
            .root::<Flyout>()
            .flatten()
            .unwrap()
            .read(cx)
            .plugin_menu_state(window)
    })
    .unwrap()
}

/// A point of « … », the first where a press gives it the focus. Only that press is released,
/// so the menu is open on return and nothing else was clicked.
fn open_menu(cx: &mut HeadlessAppContext, window: WindowHandle<Flyout>) -> Point<Pixels> {
    for y in (0..HEIGHT as i32).step_by(2) {
        for x in (0..f32::from(FLYOUT_WIDTH) as i32).step_by(2) {
            let position = point(px(x as f32), px(y as f32));
            common::mouse_down(cx, window, position);
            if state(cx, window).button_focused {
                common::mouse_up(cx, window, position);
                common::redraw(cx, window);
                return position;
            }
        }
    }
    panic!("no press reaches « … »");
}

#[test]
fn a_second_click_on_the_button_closes_the_menu() {
    let (mut cx, window) = open();
    let button = open_menu(&mut cx, window);
    common::next_frame(&mut cx, window.into());
    let opened = state(&mut cx, window);
    assert!(opened.open, "{opened:?}");
    assert!(opened.item_focused, "the first item takes the focus");
    common::click(&mut cx, window, button);
    let closed = state(&mut cx, window);
    assert!(
        !closed.open,
        "the press closes it, the release does not open it again"
    );
    assert!(closed.button_focused, "{closed:?}");
    common::click(&mut cx, window, button);
    assert!(
        state(&mut cx, window).open,
        "a click on a closed menu opens it"
    );
}

#[test]
fn a_click_outside_closes_the_menu_and_leaves_no_focus_on_its_items() {
    let (mut cx, window) = open();
    let button = open_menu(&mut cx, window);
    common::next_frame(&mut cx, window.into());
    assert!(state(&mut cx, window).item_focused);
    // Low in the body, under the card: nothing there takes the focus.
    common::click(&mut cx, window, point(px(4.), button.y + px(120.)));
    let closed = state(&mut cx, window);
    assert!(!closed.open, "{closed:?}");
    assert!(!closed.item_focused, "{closed:?}");
}
