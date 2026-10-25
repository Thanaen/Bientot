//! « Détails de l’activité et du crédit » open inside the card of the open day, which scrolls
//! into view whole when they open, and closing them leaves the days where they are. In a window
//! at the week's minimum width, their credit sits above the corrections. It needs the headless
//! GPUI of the `screenshots` feature.
#![cfg(all(feature = "screenshots", target_os = "linux"))]

use std::sync::Arc;

use bientot_app::assets::{self, Assets};
use bientot_app::demo::{self, DemoHost};
use bientot_app::strings::Language;
use bientot_app::theme::size as token;
use bientot_app::theme::{ThemeMode, text};
use bientot_app::week_view::{self, WeekView};
use bientot_core::demo::DemoScenario;
use gpui::{
    AppContext as _, Bounds, HeadlessAppContext, Pixels, ScrollHandle, Size, WindowHandle, point,
    px, size,
};
use jiff::civil::Date;

fn thursday() -> Date {
    jiff::civil::date(2026, 9, 3)
}

/// The history week in a window `height` tall, too short for all its days, open on `day`.
fn open(day: Date, height: Pixels) -> (HeadlessAppContext, WindowHandle<WeekView>) {
    open_sized(day, size(token::WEEK_WINDOW_WIDTH, height))
}

/// The history week in a window of `window_size`, open on `day`.
fn open_sized(
    day: Date,
    window_size: Size<Pixels>,
) -> (HeadlessAppContext, WindowHandle<WeekView>) {
    let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(
        assets::FONT_FAMILY,
    ));
    let mut cx = HeadlessAppContext::with_asset_source(text_system, Arc::new(Assets));
    cx.update(|cx| {
        cx.set_global(ThemeMode::Light);
        cx.set_global(Language::French);
        assets::load_fonts(cx).unwrap();
        week_view::bind_keys(cx);
    });
    let directory = demo::throwaway_directory();
    let week = DemoHost::open(DemoScenario::HistoryWeek, &directory).and_then(|mut host| {
        host.refresh(Language::French)?;
        host.week()
    });
    std::fs::remove_dir_all(&directory).ok();
    let week = week.unwrap();
    let window = cx
        .open_window(window_size, |window, cx| {
            cx.new(|cx| {
                let mut view = WeekView::new(Some(week), Language::French, window, cx);
                view.state_mut().unwrap().select(day);
                view
            })
        })
        .unwrap();
    redraw(&mut cx, window);
    (cx, window)
}

fn redraw(cx: &mut HeadlessAppContext, window: WindowHandle<WeekView>) {
    cx.update_window(window.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
}

fn toggle(cx: &mut HeadlessAppContext, window: WindowHandle<WeekView>) {
    cx.update(|cx| {
        window
            .update(cx, |view, _, cx| view.toggle_details(cx))
            .unwrap();
    });
    redraw(cx, window);
}

fn days(cx: &mut HeadlessAppContext, window: WindowHandle<WeekView>) -> ScrollHandle {
    window
        .read_with(cx, |view, _| view.days_scroll().clone())
        .unwrap()
}

/// The item of the list that is the focused day's card: the hours come first, then one card per
/// day of the page.
fn focused_card_item(cx: &mut HeadlessAppContext, window: WindowHandle<WeekView>) -> usize {
    let day = window
        .update(cx, |view, _, _| {
            let page = view.state_mut().expect("the week was read").page();
            page.days.iter().position(|row| row.focused)
        })
        .expect("the week window is open")
        .expect("a day of the page is focused");
    day + 1
}

/// The focused day's card in the list, where it shows now.
fn focused_card(cx: &mut HeadlessAppContext, window: WindowHandle<WeekView>) -> Bounds<Pixels> {
    let item = focused_card_item(cx, window);
    let days = days(cx, window);
    let card = days
        .bounds_for_item(item)
        .expect("the list laid out the focused day's card");
    Bounds::new(card.origin + point(px(0.), days.offset().y), card.size)
}

#[test]
fn the_details_open_inside_the_open_days_card_and_bring_it_into_view() {
    let (mut cx, window) = open(thursday(), px(600.));
    assert_eq!(
        focused_card_item(&mut cx, window),
        4,
        "Thursday's card follows the hours and the cards of Monday to Wednesday"
    );
    let items = days(&mut cx, window).children_count();
    let closed = focused_card(&mut cx, window);
    let viewport = days(&mut cx, window).bounds();
    toggle(&mut cx, window);
    assert_eq!(
        days(&mut cx, window).children_count(),
        items,
        "the details open inside the card, not as an item of their own"
    );
    let opened = focused_card(&mut cx, window);
    assert!(
        opened.size.height > closed.size.height,
        "Thursday's card does not hold the details: {closed:?}, then {opened:?}"
    );
    assert!(
        closed.bottom() + opened.size.height - closed.size.height > viewport.bottom(),
        "the details open in view already"
    );
    assert!(
        opened.top() >= viewport.top() - px(0.5) && opened.bottom() <= viewport.bottom() + px(0.5),
        "Thursday's card {opened:?} is not shown whole in {viewport:?}"
    );
}

#[test]
fn closing_the_details_leaves_the_days_where_they_are() {
    let (mut cx, window) = open(jiff::civil::date(2026, 8, 31), px(400.));
    toggle(&mut cx, window);
    let days = days(&mut cx, window);
    let below_monday = point(px(0.), px(-120.));
    days.set_offset(below_monday);
    toggle(&mut cx, window);
    assert_eq!(days.offset(), below_monday);
}

/// How much taller Tuesday's card grows when its details open, in a window `width` wide and
/// tall enough for every day.
fn growth_of_tuesday(width: Pixels) -> Pixels {
    let (mut cx, window) = open_sized(jiff::civil::date(2026, 9, 1), size(width, px(1400.)));
    let closed = focused_card(&mut cx, window);
    toggle(&mut cx, window);
    focused_card(&mut cx, window).size.height - closed.size.height
}

#[test]
fn at_the_minimum_width_the_credit_sits_above_the_corrections() {
    let beside = growth_of_tuesday(token::WEEK_WINDOW_WIDTH);
    let above = growth_of_tuesday(token::WEEK_WINDOW_MIN_WIDTH);
    // Gagné, Appliqué, Disponible and Expiré.
    let credit_lines = text::LABEL_SM.line_height * 4.;
    assert!(
        above - beside > credit_lines,
        "the details grow Tuesday's card by {beside:?} beside the corrections, then by \
         {above:?}: less than its four credit lines more"
    );
}
