//! The flyout that opens from the notification-area icon: the day's status, the planned
//! departure, the time worked, the Store update on offer, the notices and the tracking actions.

use std::rc::Rc;

use crate::adjust::Unrecorded;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, FontWeight, Hsla, KeyBinding, Pixels, Role,
    ScrollHandle, SharedString, Subscription, Text, Window, actions, div, prelude::*, px,
};
use worktime_domain::accounting::{AdjustmentId, TrackingState};

use crate::live::Tracking;
use crate::palette::{Host, Situation};
use crate::palette_view::{Opened, PaletteView};
use crate::shell;
use crate::strings::{Key, Language};
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, motion, radius, size, text};
use crate::tray_model::ReviewCommand;
use crate::updates::Updates;
use crate::widgets::disabled_control;
use crate::widgets::{Notice, NoticeKind, icon, scroll_thumb};
use crate::workday::{self, DayView, TrackingStatus};

mod body;
mod callout;

pub use body::Alerts;

actions!(
    worktime,
    [
        OpenWeek,
        OpenSettings,
        OpenGeneralSettings,
        TogglePause,
        AdjustTime,
        FinishDay,
        Refresh,
        UseBuiltInDetection,
        InstallUpdate,
        OpenSettingsJson,
        ExportDiagnostics,
        OpenCommands,
        CloseFlyout,
        FocusNext,
        FocusPrevious,
        Quit
    ]
);

/// « Renseigner un créneau travaillé » of the source hint, on the time the hint reported, in its
/// day.
#[derive(Clone, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = worktime, no_json)]
pub struct DeclareWork(pub Option<Unrecorded>);

/// « Oui, je travaillais » or « Ce n’était pas du travail » of the appointment the flyout asks
/// about.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = worktime, no_json)]
pub struct ResolveReview(pub ReviewCommand);

/// « Ce n’était pas du travail » on an absence counted automatically.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = worktime, no_json)]
pub struct UndoCount(pub AdjustmentId);

/// The cross of the callout of an absence counted automatically.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = worktime, no_json)]
pub struct HideCount(pub AdjustmentId);

/// « Compter automatiquement à l’avenir » on the appointment of this id.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = worktime, no_json)]
pub struct CountAutomatically(pub String);

/// The cross of « Compter automatiquement à l’avenir » on the appointment of this id.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = worktime, no_json)]
pub struct HideTrustOffer(pub String);

/// Inside the scrolling body, around its parts; a part or control revealed after Tab keeps it
/// too.
pub const BODY_PADDING: Pixels = BASE_16;

const KEY_CONTEXT: &str = "Flyout";

/// The least height of the flyout while the palette is open over it, so that the palette shows
/// six commands of its list rather than three of a short card. A shorter work area wins.
const PALETTE_ROOM: Pixels = px(360.);

/// The keystrokes of the flyout's actions. The key caps and accessible names are read from
/// them, so what the flyout shows is what it binds.
pub(crate) mod keystroke {
    pub const OPEN_WEEK: &str = "ctrl-shift-h";
    pub const OPEN_SETTINGS: &str = "ctrl-,";
    pub const TOGGLE_PAUSE: &str = "ctrl-alt-p";
    pub const ADJUST_TIME: &str = "ctrl-e";
    pub const FINISH_DAY: &str = "ctrl-shift-f";
    pub const REFRESH: &str = "f5";
    pub const OPEN_COMMANDS: &str = "ctrl-shift-p";
    pub const QUIT: &str = "ctrl-q";
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(keystroke::OPEN_WEEK, OpenWeek, Some(KEY_CONTEXT)),
        KeyBinding::new(keystroke::OPEN_SETTINGS, OpenSettings, Some(KEY_CONTEXT)),
        KeyBinding::new(keystroke::TOGGLE_PAUSE, TogglePause, Some(KEY_CONTEXT)),
        KeyBinding::new(keystroke::ADJUST_TIME, AdjustTime, Some(KEY_CONTEXT)),
        KeyBinding::new(keystroke::FINISH_DAY, FinishDay, Some(KEY_CONTEXT)),
        KeyBinding::new(keystroke::REFRESH, Refresh, Some(KEY_CONTEXT)),
        KeyBinding::new(keystroke::OPEN_COMMANDS, OpenCommands, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", CloseFlyout, Some(KEY_CONTEXT)),
        KeyBinding::new("tab", FocusNext, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(KEY_CONTEXT)),
        KeyBinding::new(keystroke::QUIT, Quit, Some(KEY_CONTEXT)),
    ]);
}

/// The cap of one key of a keystroke in `language`: `shift` is « Maj », `h` is « H ».
fn key_cap(key: &str, language: Language) -> SharedString {
    match key {
        "ctrl" => language.text(Key::KeyControl).into(),
        "shift" => language.text(Key::KeyShift).into(),
        "alt" => language.text(Key::KeyAlt).into(),
        "enter" => language.text(Key::KeyEnter).into(),
        "escape" => language.text(Key::KeyEscape).into(),
        "delete" => language.text(Key::KeyDelete).into(),
        "up" => language.text(Key::KeyUp).into(),
        "down" => language.text(Key::KeyDown).into(),
        other => other.to_uppercase().into(),
    }
}

/// The key caps of `keystroke` in `language`, « Ctrl Maj H » for `ctrl-shift-h`.
pub(crate) fn key_caps(keystroke: &str, language: Language) -> Vec<SharedString> {
    keystroke
        .split('-')
        .map(|key| key_cap(key, language))
        .collect()
}

/// `label` and its shortcut as a screen reader reads them, « Ouvrir la semaine, Ctrl+Maj+H ».
pub(crate) fn shortcut_name(label: &str, keystroke: &str, language: Language) -> SharedString {
    let keys = keystroke
        .split('-')
        .map(|key| match key {
            "," => language.text(Key::KeyComma).into(),
            _ => key_cap(key, language),
        })
        .collect::<Vec<SharedString>>()
        .join("+");
    language
        .format(Key::FlyoutShortcutNameFormat, &[&label, &keys])
        .into()
}

/// The notice once « Utiliser la détection intégrée » answered `failure`. A switch that
/// succeeded ends the failure an earlier one left; the day it shows can still offer the switch,
/// for the part of the day ActivityWatch was the source.
pub(crate) fn after_switch(notice: Option<Notice>, failure: Option<Notice>) -> Option<Notice> {
    failure.or_else(|| notice.filter(|notice| notice.kind != NoticeKind::SourceSwitchFailed))
}

pub struct Flyout {
    day: DayView,
    notice: Option<Notice>,
    /// Comes in each time an answer brings a notice to the open flyout.
    notice_entrance: motion::Entrance,
    alerts: Alerts,
    /// Comes in when a refresh fails in the open flyout.
    alert_entrance: motion::Entrance,
    language: Language,
    tracking: Tracking,
    focus_handle: FocusHandle,
    palette: Option<Opened>,
    fitted: shell::Fitted,
    /// Set by [`Self::with_max_height`].
    max_height: Option<Pixels>,
    /// The body, between the header and the actions, scrolls when the panel cannot hold it.
    body_scroll: ScrollHandle,
    /// One for each part of the body, so that Tab brings the part that takes the focus into
    /// view.
    body_parts: Vec<FocusHandle>,
    /// The controls of the body as last laid out, so that Tab brings the one that takes the
    /// focus into view in a part the body cannot hold. A frame behind: the controls add
    /// themselves as the body is prepainted, and the next render of the body reads them to
    /// reveal the focus Tab moved, then empties the list for that frame's controls.
    body_controls: body::LaidOutControls,
    reveal_focus: bool,
    _close_on_deactivate: Subscription,
    _show_updates: Subscription,
    _keep_focus: Subscription,
}

impl Flyout {
    /// The flyout's actions reach the day through the app: see `host::install`. A `notice` it
    /// opens with shows at rest.
    pub fn new(
        day: DayView,
        tracking: Tracking,
        notice: Option<Notice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let close_on_deactivate = cx.observe_window_activation(window, |_, window, cx| {
            if !window.is_window_active() {
                shell::flyout_closed(cx);
                window.remove_window();
            }
        });
        let show_updates = cx.observe_global::<Updates>(|_, cx| cx.notify());
        // A control that goes away with the focus, as the update callout once installed,
        // hands it back to the flyout: without it Tab and Échap fall to the root.
        let keep_focus = cx.on_focus_lost(window, |flyout, window, cx| {
            window.focus(&flyout.focus_handle, cx);
        });
        Self {
            day,
            notice,
            notice_entrance: motion::Entrance::at_rest(),
            alerts: Alerts::default(),
            alert_entrance: motion::Entrance::at_rest(),
            language: cx.try_global::<Language>().copied().unwrap_or_default(),
            tracking,
            focus_handle,
            palette: None,
            fitted: shell::Fitted::default(),
            max_height: None,
            body_scroll: ScrollHandle::new(),
            body_parts: Vec::new(),
            body_controls: Rc::default(),
            reveal_focus: false,
            _close_on_deactivate: close_on_deactivate,
            _show_updates: show_updates,
            _keep_focus: keep_focus,
        }
    }

    /// Opens on `alerts`, at rest.
    pub fn with_alerts(mut self, alerts: Alerts) -> Self {
        self.alerts = alerts;
        self
    }

    /// Overrides the limit taken from the work area: a panel no taller than `height`, for the
    /// screenshots and the tests.
    pub fn with_max_height(mut self, height: Pixels) -> Self {
        self.max_height = Some(height);
        self
    }

    /// Shows a new state of the day. A refresh that `recalculated` it answers the notice
    /// that asked for one; a `refresh_failure` comes in, and the next refresh that succeeds
    /// ends it.
    pub fn show(
        &mut self,
        day: DayView,
        tracking: Tracking,
        recalculated: bool,
        refresh_failure: Option<Notice>,
        cx: &mut Context<Self>,
    ) {
        self.day = day;
        self.tracking = tracking;
        self.notice = self
            .notice
            .take()
            .filter(|notice| notice.outlives(recalculated));
        if self.alerts.refresh_failure.is_none() && refresh_failure.is_some() {
            self.alert_entrance.reset();
        }
        self.alerts.refresh_failure = refresh_failure;
        if let Some(palette) = &self.palette {
            palette.set_situation(self.situation(), cx);
        }
        cx.notify();
    }

    /// Brings `notice` into the open flyout. It comes in even when it repeats the notice shown:
    /// each answer is acknowledged.
    pub fn set_notice(&mut self, notice: Notice, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        self.notice_entrance.reset();
        cx.notify();
    }

    /// Shows what « Utiliser la détection intégrée » answered: see [`after_switch`]. A failure
    /// comes in as [`Self::set_notice`]'s notice does; a success leaves a kept notice still.
    pub fn switched(&mut self, failure: Option<Notice>, cx: &mut Context<Self>) {
        if failure.is_some() {
            self.notice_entrance.reset();
        }
        self.notice = after_switch(self.notice.take(), failure);
        cx.notify();
    }

    /// Ctrl+Maj+P: the command palette over the flyout.
    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_none() {
            let situation = self.situation();
            let host = self.focus_handle.clone();
            self.palette = Some(Opened::new(
                situation,
                host,
                |flyout: &mut Self| &mut flyout.palette,
                window,
                cx,
            ));
            cx.notify();
        }
    }

    pub fn palette(&self) -> Option<&Entity<PaletteView>> {
        self.palette.as_ref().map(Opened::view)
    }

    /// What scrolls between the header and the actions, for the tests.
    #[cfg(feature = "screenshots")]
    pub fn body_scroll(&self) -> &ScrollHandle {
        &self.body_scroll
    }

    /// Where the control of the body that holds the focus shows, scrolled, for the tests.
    #[cfg(feature = "screenshots")]
    pub fn focused_body_control(&self, window: &Window, cx: &App) -> Option<gpui::Bounds<Pixels>> {
        let scrolled = gpui::point(px(0.), self.body_scroll.offset().y);
        self.focused_control(window, cx)
            .map(|control| control + scrolled)
    }

    fn situation(&self) -> Situation {
        Situation {
            host: Host::Flyout,
            tracking: self.tracking,
            can_correct_time: self.tracking.can_correct_time(),
        }
    }

    fn focus_next(&mut self, _: &FocusNext, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_next(cx);
        self.reveal_focus = true;
        cx.notify();
    }

    fn focus_previous(&mut self, _: &FocusPrevious, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_prev(cx);
        self.reveal_focus = true;
        cx.notify();
    }

    fn close(&mut self, _: &CloseFlyout, window: &mut Window, cx: &mut Context<Self>) {
        shell::flyout_closed(cx);
        window.remove_window();
    }

    fn text(&self, key: Key) -> &'static str {
        self.language.text(key)
    }
}

struct ToneStyle {
    color: Hsla,
    icon: &'static str,
}

fn tone_style(tone: workday::Tone, theme: &Theme) -> ToneStyle {
    let (color, icon) = match tone {
        workday::Tone::OnTrack => (theme.success, "circle-check"),
        workday::Tone::NearLimit => (theme.warning, "triangle-alert"),
        workday::Tone::Late => (theme.error, "circle-x"),
        workday::Tone::Complete => (theme.accent, "circle-arrow-up"),
        workday::Tone::NonWorkingDay => (theme.text_muted, "calendar-off"),
        workday::Tone::SourceProblem => (theme.warning, "activity"),
        // A pause is the user's choice, not a fault.
        workday::Tone::Paused => (theme.text_muted, "pause"),
        workday::Tone::Finished => (theme.text_muted, "flag"),
    };
    ToneStyle { color, icon }
}

/// Text that screen readers read. Plain strings are not in the accessibility tree; use them
/// only where a parent's label already says the same thing.
pub(crate) fn readable(id: impl Into<gpui::ElementId>, text: impl Into<SharedString>) -> Text {
    Text::new(id.into(), text.into())
}

pub(crate) fn keys(caps: Vec<SharedString>, theme: &Theme) -> impl IntoElement {
    tinted_keys(caps, theme.text_muted, theme)
}

fn tinted_keys(caps: Vec<SharedString>, color: Hsla, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .gap(BASE_02)
        .children(caps.into_iter().map(|cap| {
            div()
                .min_w(size::KEY_CAP)
                .px(BASE_04)
                .rounded(radius::XS)
                .border_1()
                .border_color(theme.border_variant)
                .bg(theme.element)
                .type_style(text::LABEL_SM)
                .text_color(color)
                .flex()
                .justify_center()
                .child(cap)
        }))
}

/// The tracking state in a status bar: a dot of its colour, then its name. A pause, a
/// finished day and a day off take the colour the flyout's header gives them.
pub(crate) fn tracking_status(
    tracking: TrackingStatus,
    language: Language,
    theme: &Theme,
) -> impl IntoElement {
    let tone = |tone| tone_style(tone, theme).color;
    let (dot, status) = match tracking {
        TrackingStatus::Tracking(TrackingState::Running) => (theme.success, Key::ShellRunning),
        TrackingStatus::Tracking(TrackingState::Paused) => {
            (tone(workday::Tone::Paused), Key::FlyoutPaused)
        }
        TrackingStatus::Tracking(TrackingState::Finished) => {
            (tone(workday::Tone::Finished), Key::FlyoutFinished)
        }
        TrackingStatus::DayOff => (tone(workday::Tone::NonWorkingDay), Key::ShellWeekend),
    };
    let status = language.text(status);
    div()
        .id("tracking")
        .role(Role::Status)
        .aria_label(status)
        .flex()
        .items_center()
        .gap(BASE_06)
        .child(div().size(px(6.)).rounded_full().bg(dot))
        .child(status)
}

/// A control reachable with Tab and Enter: focus ring, hover and pressed states. Its icon
/// brightens on hover through the `id` group.
fn control(id: &'static str, name: SharedString, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let (hover, active, focused) = (
        theme.element_hover,
        theme.element_active,
        theme.border_focused,
    );
    div()
        .id(id)
        .group(id)
        .role(Role::Button)
        .aria_label(name)
        .tab_index(0)
        .rounded(radius::SM)
        .border_1()
        .border_color(gpui::transparent_black())
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(active))
        .focus_visible(move |style| style.border_color(focused))
}

fn control_icon(group: &'static str, name: &str, theme: &Theme) -> impl IntoElement {
    let hover = theme.text;
    icon(name, theme.text_muted).group_hover(group, move |style| style.text_color(hover))
}

impl Flyout {
    fn icon_button(
        &self,
        id: &'static str,
        icon_name: &str,
        label: Key,
        keystroke: &str,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        control(
            id,
            shortcut_name(self.text(label), keystroke, self.language),
            theme,
        )
        .size(size::BUTTON_MEDIUM)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(control_icon(id, icon_name, theme))
    }

    /// One of the three tracking buttons, sharing the row equally. Its short `label` is read
    /// first, with the shortcut, and the command's full `name` describes it. A disabled button
    /// keeps its place, dimmed, as the .NET app greys a command it cannot run.
    fn action_button(
        &self,
        id: &'static str,
        label: Key,
        name: Key,
        keystroke: &str,
        enabled: bool,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        let label = self.text(label);
        let accessible = shortcut_name(label, keystroke, self.language);
        // `control` already brightens the fill on hover.
        let button = if enabled {
            control(id, accessible, theme)
        } else {
            disabled_control(id, accessible, theme)
        };
        button
            .bg(theme.element)
            .border_color(theme.border_variant)
            .aria_description(self.text(name))
            .flex_1()
            .min_w_0()
            .h(size::BUTTON_LARGE)
            .px(BASE_08)
            .flex()
            .items_center()
            .justify_center()
            .whitespace_nowrap()
            .overflow_hidden()
            .text_ellipsis()
            .child(label)
    }

    fn header(&self, theme: &Theme) -> impl IntoElement {
        let shown = self.day.shown_while(self.tracking.state());
        let tone = tone_style(shown.tone, theme);
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(BASE_08)
            .py(BASE_08)
            .pr(BASE_08)
            .pl(BASE_16)
            .border_b_1()
            .border_color(theme.border_variant)
            .child(
                div()
                    .id("status")
                    .role(Role::Status)
                    .aria_label(shown.status.clone())
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(BASE_06)
                    .type_style(text::LABEL_LG)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(tone.color)
                    .child(icon(tone.icon, tone.color))
                    .child(shown.status.clone()),
            )
            .child(
                self.icon_button(
                    "open-week",
                    "calendar",
                    Key::FlyoutWeek,
                    keystroke::OPEN_WEEK,
                    theme,
                )
                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenWeek), cx)),
            )
            .child(
                self.icon_button(
                    "open-settings",
                    "sliders-horizontal",
                    Key::SettingsWindowTitle,
                    keystroke::OPEN_SETTINGS,
                    theme,
                )
                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx)),
            )
    }

    /// The tracking buttons, or nothing on a day off, when none of them can run.
    fn actions(&self, theme: &Theme) -> Option<impl IntoElement> {
        let (pause_label, pause_name) = match self.tracking.state() {
            TrackingState::Running => (Key::FlyoutPauseButton, Key::TrayPauseCommand),
            TrackingState::Paused | TrackingState::Finished => {
                (Key::FlyoutResumeButton, Key::TrayResumeCommand)
            }
        };
        let (can_toggle, can_correct, can_finish) = (
            self.tracking.can_toggle(),
            self.tracking.can_correct_time(),
            self.tracking.can_finish(),
        );
        if !(can_toggle || can_correct || can_finish) {
            return None;
        }
        let row = div()
            .flex_none()
            .flex()
            .gap(BASE_04)
            .pt(BASE_08)
            .px(BASE_08)
            .pb(BASE_08)
            .border_t_1()
            .border_color(theme.border_variant)
            .child(
                self.action_button(
                    "toggle-pause",
                    pause_label,
                    pause_name,
                    keystroke::TOGGLE_PAUSE,
                    can_toggle,
                    theme,
                )
                .when(can_toggle, |button| {
                    button
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(TogglePause), cx))
                }),
            )
            .child(
                self.action_button(
                    "adjust-time",
                    Key::FlyoutAdjustButton,
                    Key::FlyoutAdjustTime,
                    keystroke::ADJUST_TIME,
                    can_correct,
                    theme,
                )
                .when(can_correct, |button| {
                    button
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(AdjustTime), cx))
                }),
            )
            .child(
                self.action_button(
                    "finish-day",
                    Key::FlyoutFinishButton,
                    Key::TrayFinishCommand,
                    keystroke::FINISH_DAY,
                    can_finish,
                    theme,
                )
                .when(can_finish, |button| {
                    button.on_click(|_, window, cx| window.dispatch_action(Box::new(FinishDay), cx))
                }),
            );
        Some(row)
    }
}

impl Focusable for Flyout {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Flyout {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let max_height = self.max_height.or_else(|| {
            shell::work_area_height()
                .map(|height| shell::panel_max_height(height, window.scale_factor()))
        });
        let body = self.body(window, cx, &theme);
        let content = div()
            .id("flyout")
            .role(Role::Dialog)
            .aria_label(self.text(Key::ShellWindowTitle))
            // The open palette takes the keys, so that Tab stays in it.
            .when(self.palette.is_none(), |content| {
                content.key_context(KEY_CONTEXT)
            })
            .track_focus(&self.focus_handle)
            .on_action(
                cx.listener(|flyout, _: &OpenCommands, window, cx| flyout.open_palette(window, cx)),
            )
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::focus_next))
            .on_action(cx.listener(Self::focus_previous))
            .w(size::FLYOUT_WIDTH)
            .flex()
            .flex_col()
            .type_style(text::LABEL_MD)
            .text_color(theme.text)
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .rounded(radius::LG)
            .overflow_hidden()
            .when(self.palette.is_some(), |content| {
                content.min_h(max_height.map_or(PALETTE_ROOM, |height| PALETTE_ROOM.min(height)))
            })
            .when_some(max_height, |content, height| content.max_h(height))
            .child(self.header(&theme))
            .child(
                div()
                    .relative()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("flyout-body")
                            .track_scroll(&self.body_scroll)
                            .min_h_0()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(BASE_16)
                            .p(BODY_PADDING)
                            .children(body),
                    )
                    .child(scroll_thumb(self.body_scroll.clone(), &theme)),
            )
            .children(self.actions(&theme))
            .children(
                self.palette
                    .as_ref()
                    .map(|palette| palette.overlay(&theme).rounded(radius::LG - px(1.))),
            );

        let fitted = self.fitted.clone();
        div()
            .size_full()
            // GPUI clears the opaque window to white, and Windows rounds its corners a little
            // outside the panel's rounding: the pixels in between take the border's color.
            .when(cfg!(windows), |root| root.bg(theme.border))
            .on_children_prepainted(move |bounds, window, _| {
                if let Some(content) = bounds.first()
                    && fitted.refit(content.size, window.scale_factor())
                {
                    shell::fit_window(window, content.size);
                }
            })
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_read_as_their_bindings_in_each_language() {
        assert_eq!(
            key_caps(keystroke::OPEN_WEEK, Language::French),
            ["Ctrl", "Maj", "H"]
        );
        assert_eq!(
            key_caps(keystroke::TOGGLE_PAUSE, Language::English),
            ["Ctrl", "Alt", "P"]
        );
        assert_eq!(
            shortcut_name("Paramètres", keystroke::OPEN_SETTINGS, Language::French),
            "Paramètres, Ctrl+Virgule"
        );
        assert_eq!(
            shortcut_name("Refresh", keystroke::REFRESH, Language::English),
            "Refresh, F5"
        );
    }

    #[test]
    fn a_recalculated_day_ends_the_refresh_warning_but_not_the_success() {
        let warning = Notice::refresh_pending("Réessayez l’actualisation.");
        let success = Notice::success("09:00–10:00 compté comme travail (1 h).");
        assert!(warning.outlives(false));
        assert!(!warning.outlives(true));
        assert!(success.outlives(false));
        assert!(success.outlives(true));
    }

    #[test]
    fn a_switch_that_succeeds_keeps_a_correction_s_notice() {
        let success = Notice::success("09:00–10:00 compté comme travail (1 h).");
        assert_eq!(after_switch(Some(success.clone()), None), Some(success));
    }
}
