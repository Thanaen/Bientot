//! The one « Corriger le temps… » window: when a request opens it, shows it or leaves it, and
//! which view shows the correction it saved.

use gpui::{
    App, AppContext as _, Bounds, Global, WindowBounds, WindowHandle, WindowOptions, px, size,
};
use worktime_domain::accounting::AdjustmentId;
use worktime_domain::time::LocalWorkday;

use super::{AdjustView, WINDOW_WIDTH, typed};
use crate::adjust::window::{OpenOutcome, OpenWindow, Opener, WindowRequest};
use crate::adjust::{AdjustForm, Closed, Seed};
use crate::host::Today;
use crate::shell;
use crate::strings::{Key, Language};
use crate::week_view;
use crate::widgets;

#[derive(Default)]
struct AdjustWindow(Option<WindowHandle<AdjustView>>);

impl Global for AdjustWindow {}

/// Opens « Corriger le temps… » on `workday`, else on today by the calendar, with the form on
/// `seed`, as [`WindowRequest`] decides. An opener that does not look at what became of the
/// request ignores the outcome.
pub fn open(
    workday: Option<LocalWorkday>,
    seed: Seed,
    opener: Opener,
    cx: &mut App,
) -> OpenOutcome {
    let Some(today) = cx.try_global::<Today>() else {
        return OpenOutcome::Refused;
    };
    let language = cx.try_global::<Language>().copied().unwrap_or_default();
    let form = today.adjust_target(workday).map(|target| {
        let id =
            AdjustmentId::new(format!("{:032x}", crate::random_id())).expect("the id is not blank");
        AdjustForm::new(target, seed, id, today.adjustment_time(), language)
    });
    let open = cx.default_global::<AdjustWindow>().0;
    let request = open
        .and_then(|window| {
            window
                .update(cx, |view, window, cx| {
                    let shown = typed(&view.form, &view.duration, &view.reason, cx);
                    let open = OpenWindow {
                        opened: &view.opened,
                        shown: &shown,
                    };
                    let request = WindowRequest::of(Some(open), form.as_ref());
                    view.opener = request.opener(view.opener, opener);
                    match request {
                        WindowRequest::Open => window.remove_window(),
                        WindowRequest::Show | WindowRequest::Keep => window.activate_window(),
                        WindowRequest::Refuse => {}
                    }
                    request
                })
                .ok()
        })
        .unwrap_or_else(|| WindowRequest::of(None, form.as_ref()));
    if let Some(outcome) = request.outcome() {
        return outcome;
    }
    let Some(form) = form else {
        log::warn!("no working day to correct");
        return OpenOutcome::Refused;
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(WINDOW_WIDTH), px(form.window_height())),
            cx,
        ))),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(form.page().title.into()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        focus: true,
        show: true,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        cx.new(|cx| AdjustView::new(form, opener, window, cx))
    }) {
        Ok(window) => {
            cx.default_global::<AdjustWindow>().0 = Some(window);
            OpenOutcome::Fresh
        }
        Err(error) => {
            log::error!("cannot open « Corriger le temps… »: {error:#}");
            OpenOutcome::Refused
        }
    }
}

/// Shows a saved correction in the view that opened the window. The flyout stands in for a
/// week closed meanwhile, and for the tray, which has no view of its own.
pub(super) fn deliver(closed: Closed, opener: Opener, cx: &mut App) {
    if opener == Opener::Week && week_view::adjusted(&closed, cx) {
        return;
    }
    let language = cx.try_global::<Language>().copied().unwrap_or_default();
    let notice = if closed.refreshed {
        widgets::Notice::success(closed.message)
    } else {
        widgets::Notice::refresh_pending(language.text(Key::AdjustmentSavedRefreshPendingMessage))
    };
    shell::open_flyout_with(notice, cx);
}
