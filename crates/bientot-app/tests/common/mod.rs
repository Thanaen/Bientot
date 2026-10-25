//! The headless app the GPUI tests drive, on a clock that only moves when a test steps it, and
//! the keys and mouse presses they send its windows.
//!
//! What a frame draws cannot be read back without a GPU, so the motion tests check that an
//! entrance runs, rests and asks for nothing more; how it looks is the pure `motion` functions'
//! tests and the screenshots' job.

// Each test binary builds its own copy of this module and uses only part of it.
#![allow(dead_code)]

use std::sync::Arc;

use bientot_app::assets::{self, Assets};
use bientot_app::strings::Language;
use bientot_app::theme::{ThemeMode, motion};
use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, HeadlessAppContext, KeyUpEvent, Keystroke,
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, PlatformInput, Point, Render,
    Size, Window, WindowHandle,
};

/// The app in French and light, with `bind_keys`' keys.
pub fn app(reduce_motion: bool, bind_keys: fn(&mut App)) -> HeadlessAppContext {
    let text_system = Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(
        assets::FONT_FAMILY,
    ));
    let mut cx = HeadlessAppContext::with_asset_source(text_system, Arc::new(Assets));
    cx.update(|cx| {
        cx.set_global(ThemeMode::Light);
        cx.set_global(Language::French);
        cx.set_reduce_motion(reduce_motion);
        assets::load_fonts(cx).unwrap();
        bind_keys(cx);
    });
    cx
}

/// A window of `size` showing the view `build` makes, drawn and settled.
pub fn open<V: Render>(
    cx: &mut HeadlessAppContext,
    size: Size<Pixels>,
    build: impl FnOnce(&mut Window, &mut Context<V>) -> V,
) -> WindowHandle<V> {
    let window = cx
        .open_window(size, |window, cx| cx.new(|cx| build(window, cx)))
        .unwrap();
    redraw(cx, window);
    window
}

pub fn redraw<V: 'static>(cx: &mut HeadlessAppContext, window: WindowHandle<V>) {
    cx.update_window(window.into(), |_, window, _| window.refresh())
        .unwrap();
    cx.run_until_parked();
}

/// Presses and releases `key`, as a keyboard would: a character it types reaches a text field.
pub fn press<V: 'static>(cx: &mut HeadlessAppContext, window: WindowHandle<V>, key: &str) {
    let keystroke = Keystroke::parse(key).unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_keystroke(keystroke.clone(), cx);
        window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke }), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Types `text` one key at a time.
pub fn type_text<V: 'static>(cx: &mut HeadlessAppContext, window: WindowHandle<V>, text: &str) {
    for key in text.chars() {
        press(cx, window, &key.to_string());
    }
}

pub fn mouse_down_event(position: Point<Pixels>) -> PlatformInput {
    PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Left,
        position,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    })
}

pub fn mouse_down<V: 'static>(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<V>,
    position: Point<Pixels>,
) {
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(mouse_down_event(position), cx);
    })
    .unwrap();
}

pub fn mouse_up<V: 'static>(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<V>,
    position: Point<Pixels>,
) {
    cx.update_window(window.into(), |_, window, cx| {
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
}

/// A press and release of the left button at `position`, then a redraw.
pub fn click<V: 'static>(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<V>,
    position: Point<Pixels>,
) {
    mouse_down(cx, window, position);
    mouse_up(cx, window, position);
    redraw(cx, window);
}

/// Delivers the next frame and returns how many redraws the window had asked for.
pub fn next_frame(cx: &mut HeadlessAppContext, window: AnyWindowHandle) -> usize {
    let asked = cx
        .update_window(window, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
    asked
}

/// Steps through an entrance of `timing` that starts on the next frame: it asks for frames
/// until its duration, then stops. `which` names it in the failures.
pub fn assert_entrance_runs(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    timing: motion::Timing,
    which: &str,
) {
    assert!(next_frame(cx, window) > 0, "{which} does not come in");
    cx.advance_clock(timing.duration() / 2);
    assert!(next_frame(cx, window) > 0, "{which} stops halfway");
    cx.advance_clock(timing.duration() / 2);
    next_frame(cx, window);
    assert_eq!(next_frame(cx, window), 0, "{which} outlasts its duration");
}
