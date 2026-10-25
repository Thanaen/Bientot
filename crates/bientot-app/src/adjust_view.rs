//! The « Corriger le temps… » window opened by the tray, the flyout and the week. Its state
//! lives in [`AdjustForm`]; this module draws it, turns keys and clicks into its methods, saves
//! through the host and hands the result to the view that opened it.

use gpui::{
    App, Context, Entity, FocusHandle, Focusable as _, KeyBinding, KeyDownEvent, Role,
    SharedString, Subscription, Window, actions, div, prelude::*,
};

use crate::adjust::window::Opener;
use crate::adjust::{AdjustForm, Closed, REASON_MAX_LENGTH, TimeField, TimePart};
use crate::flyout;
use crate::host::{self, Today};
use crate::strings::{Key, Language};
use crate::text_input::TextInput;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, text};
use crate::widgets::ClockFocus;
use window::deliver;

mod chrome;
mod context;
mod fields;
pub mod window;

actions!(adjust_time, [Save, Cancel, FocusNext, FocusPrevious]);

const KEY_CONTEXT: &str = "AdjustTime";
/// Enter on the focused « Annuler » cancels instead of saving.
const CANCEL_CONTEXT: &str = "AdjustCancel";

pub const WINDOW_WIDTH: f32 = 440.;

mod keystroke {
    pub const SAVE: &str = "enter";
    pub const CANCEL: &str = "escape";
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(keystroke::SAVE, Save, Some(KEY_CONTEXT)),
        KeyBinding::new(keystroke::SAVE, Cancel, Some(CANCEL_CONTEXT)),
        KeyBinding::new(keystroke::CANCEL, Cancel, Some(KEY_CONTEXT)),
        KeyBinding::new("tab", FocusNext, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(KEY_CONTEXT)),
    ]);
}

/// `form` with what « Durée » and « Motif » show, which it reads only when saved.
fn typed(
    form: &AdjustForm,
    duration: &Entity<TextInput>,
    reason: &Entity<TextInput>,
    cx: &App,
) -> AdjustForm {
    let mut typed = form.clone();
    typed.duration = duration.read(cx).text().to_owned();
    typed.reason = reason.read(cx).text().to_owned();
    typed
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bound {
    Start,
    End,
}

pub struct AdjustView {
    form: AdjustForm,
    /// The form as it opened, with what its text fields showed then.
    opened: AdjustForm,
    opener: Opener,
    language: Language,
    focus_handle: FocusHandle,
    duration: Entity<TextInput>,
    reason: Entity<TextInput>,
    start: ClockFocus,
    end: ClockFocus,
    _blurs: Vec<Subscription>,
}

impl AdjustView {
    pub fn new(
        form: AdjustForm,
        opener: Opener,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let language = cx.try_global::<Language>().copied().unwrap_or_default();
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let duration = cx.new(|cx| {
            TextInput::new(
                language.text(Key::AdjustTimeDurationHeader),
                language.text(Key::AdjustTimeDurationPlaceholderText),
                None,
                cx,
            )
        });
        let reason = cx.new(|cx| {
            TextInput::new(
                language.text(Key::AdjustTimeReasonHeader),
                language.text(Key::AdjustTimeReasonPlaceholderText),
                Some(REASON_MAX_LENGTH),
                cx,
            )
        });
        // An edit opens on the correction's duration and reason.
        duration.update(cx, |input, cx| input.set_text(&form.duration, cx));
        reason.update(cx, |input, cx| input.set_text(&form.reason, cx));
        let opened = typed(&form, &duration, &reason, cx);
        let (start, end) = (ClockFocus::new(cx), ClockFocus::new(cx));
        // A digit typed after the focus came back starts the part over.
        let mut blurs = Vec::new();
        for (bound, handle) in [
            (Bound::Start, &start.hours),
            (Bound::Start, &start.minutes),
            (Bound::End, &end.hours),
            (Bound::End, &end.minutes),
        ] {
            blurs.push(cx.on_blur(handle, window, move |view, _, _| {
                view.field_mut(bound).leave();
            }));
        }
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            view.update(cx, |view, cx| view.may_close(window, cx))
                .unwrap_or(true)
        });
        Self {
            form,
            opened,
            opener,
            language,
            focus_handle,
            duration,
            reason,
            start,
            end,
            _blurs: blurs,
        }
    }

    /// The form of a window not tied to the app's day, for the screenshots.
    pub fn form_mut(&mut self) -> &mut AdjustForm {
        &mut self.form
    }

    /// Fills « Durée » as the user would, for the screenshots.
    pub fn set_duration(&mut self, text: &str, cx: &mut Context<Self>) {
        self.duration
            .update(cx, |input, cx| input.set_text(text, cx));
        self.form.duration = text.to_owned();
    }

    /// Fills « Motif » as the user would, the caret at its end, for the screenshots.
    pub fn set_reason(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.reason.update(cx, |input, cx| input.set_text(text, cx));
        self.form.reason = self.reason.read(cx).text().to_owned();
        window.focus(&self.reason.focus_handle(cx), cx);
    }

    fn field_mut(&mut self, bound: Bound) -> &mut TimeField {
        match bound {
            Bound::Start => &mut self.form.start,
            Bound::End => &mut self.form.end,
        }
    }

    fn text(&self, key: Key) -> &'static str {
        self.language.text(key)
    }

    fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.is_refresh_pending() {
            self.retry_refresh(window, cx);
            return;
        }
        self.form.duration = self.duration.read(cx).text().to_owned();
        self.form.reason = self.reason.read(cx).text().to_owned();
        let Some(adjustment) = self.form.submit() else {
            cx.notify();
            return;
        };
        window.focus(&self.focus_handle, cx);
        cx.notify();
        // Saves once the locked form has been drawn.
        cx.spawn_in(window, async move |view, cx| {
            view.update_in(cx, |view, window, cx| {
                let mut result = Err(Key::AdjustTimeErrorSubmissionFailed);
                if cx.try_global::<Today>().is_some() {
                    host::change(cx, |today| result = today.save_adjustment(&adjustment));
                }
                let closed = view
                    .form
                    .finished(result.map(|recorded| recorded.recalculated));
                view.closed(closed, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn retry_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut recalculated = false;
        if cx.try_global::<Today>().is_some() {
            host::change(cx, |today| {
                today.refresh();
                recalculated = today.recalculated();
            });
        }
        let closed = self.form.refreshed(recalculated);
        self.closed(closed, window, cx);
    }

    /// Closes the window once a correction is saved; else draws the form again.
    fn closed(&mut self, closed: Option<Closed>, window: &mut Window, cx: &mut Context<Self>) {
        match closed {
            Some(closed) => {
                let opener = self.opener;
                cx.defer(move |cx| deliver(closed, opener, cx));
                window.remove_window();
            }
            None => cx.notify(),
        }
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.may_close(window, cx) {
            window.remove_window();
        }
    }

    /// Whether the window may close now, handing a correction still to refresh to its opener.
    fn may_close(&mut self, _: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.form.can_close() {
            return false;
        }
        if let Some(closed) = self.form.close() {
            let opener = self.opener;
            cx.defer(move |cx| deliver(closed, opener, cx));
        }
        true
    }

    fn change(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut AdjustForm)) {
        change(&mut self.form);
        cx.notify();
    }

    fn time_key(
        &mut self,
        bound: Bound,
        part: TimePart,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.form.is_locked() || event.keystroke.modifiers.modified() {
            return;
        }
        let key = event.keystroke.key.as_str();
        let field = self.field_mut(bound);
        match key {
            "up" => field.step(part, 1),
            "down" => field.step(part, -1),
            _ => {
                let Some(digit) = key.parse::<u8>().ok().filter(|digit| *digit <= 9) else {
                    return;
                };
                if field.type_digit(part, digit) && part == TimePart::Hours {
                    let minutes = match bound {
                        Bound::Start => self.start.minutes.clone(),
                        Bound::End => self.end.minutes.clone(),
                    };
                    window.focus(&minutes, cx);
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }
}

fn label(text: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .type_style(text::LABEL_SM)
        .text_color(theme.text_muted)
        .child(text)
}

impl Render for AdjustView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let page = self.form.page();
        let locked = page.locked;
        self.duration.update(cx, |input, _| {
            input.disabled = locked;
            input.invalid = page.duration_error.is_some();
        });
        self.reason.update(cx, |input, _| input.disabled = locked);
        let body = div()
            .id("body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(BASE_20)
            .py(BASE_20)
            .px(BASE_24)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(BASE_04)
                    .child(
                        div()
                            .id("adjust-title")
                            .role(Role::Heading)
                            .aria_label(page.title)
                            .type_style(text::HEADLINE_SM)
                            .child(page.title),
                    )
                    .child(
                        div()
                            .text_color(theme.text_muted)
                            .child(flyout::readable("adjust-day", page.day.clone())),
                    )
                    .children(page.declaration.as_ref().map(|declaration| {
                        div()
                            .pt(BASE_04)
                            .text_color(theme.text_muted)
                            .child(flyout::readable(
                                "adjust-explanation",
                                declaration.explanation,
                            ))
                    })),
            )
            .children(
                page.declaration
                    .as_ref()
                    .and_then(|declaration| declaration.context.as_ref())
                    .map(|context| self.day_context(context, locked, &theme, cx)),
            )
            .when(!page.intents.is_empty(), |body| {
                body.child(self.intents(&page, &theme, cx))
            })
            .child(self.times(&page, &theme, cx))
            .when(page.shows_classification, |body| {
                body.child(self.classification(&page, &theme, cx))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(BASE_04)
                    .child(label(self.text(Key::AdjustTimeReasonHeader), &theme))
                    .child(self.reason.clone()),
            );
        div()
            .id("adjust-time")
            .role(Role::Dialog)
            .aria_label(page.title)
            .aria_description(SharedString::from(page.day.clone()))
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, cx| window.focus_prev(cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.editor)
            .text_color(theme.text)
            .type_style(text::LABEL_MD)
            .child(self.title_bar(&theme, cx))
            .child(body)
            .children(self.callout(&page, &theme))
            .child(self.footer(&page, &theme, cx))
    }
}
