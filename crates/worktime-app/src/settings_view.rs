//! The « Paramètres » window opened by the flyout. Its state lives in [`SettingsForm`]; this
//! module draws it, turns keys and clicks into its methods and saves through the host.

mod appointments;
mod data;
mod dialog;
mod json;
mod language;

use dialog::Dialog;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, App, AppContext as _, Bounds, Context, Entity, FocusHandle,
    Focusable as _, FontWeight, Global, HighlightStyle, InteractiveText, KeyBinding, KeyDownEvent,
    MouseButton, Pixels, Role, ScrollHandle, SharedString, StyledText, Subscription, Toggled,
    Transformation, Window, WindowBounds, WindowControlArea, WindowHandle, WindowOptions, actions,
    div, percentage, prelude::*, px, size, transparent_black,
};
use worktime_core::settings::{EditGroup, Pending, SettingsPort};
use worktime_domain::configuration::{AppointmentProposal, RecurringAppointment, WorkTimeSettings};
use worktime_domain::usage_ping::{Build, Channel};
use worktime_store::settings::SaveSettingsError;
use worktime_windows::startup::StartupRegistrationState;

use crate::adjust::{TimeField, TimePart};
use crate::host::{self, Today};
use crate::settings::data::DataPage;
use crate::settings::startup::{self, StartupPort};
use crate::settings::{
    AboutView, CalloutView, Clock, Control, Details, Field, LanguageChoice, Leaving, Link, Page,
    PingDetails, RowTone, RowView, SectionView, SettingsForm, SourceCallout, Switch, Tone,
    UsagePingFacts,
};
use crate::shell;
use crate::strings::{Key, Language};
use crate::text_input::TextInput;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, motion, radius, size as token, tabular_figures, text};
use crate::transfer;
use crate::widgets::{self, ClockFocus, Hue};

actions!(
    settings,
    [Commit, StepUp, StepDown, Escape, FocusNext, FocusPrevious]
);

const KEY_CONTEXT: &str = "Settings";
/// A field Enter commits.
const FIELD_CONTEXT: &str = "SettingsField";
/// A number the arrow keys step.
const NUMBER_CONTEXT: &str = "SettingsNumber";
/// GPUI keeps one context per element, so a number field names both.
const NUMBER_FIELD_CONTEXT: &str = "SettingsField SettingsNumber";

pub const WINDOW_WIDTH: Pixels = token::WEEK_WINDOW_WIDTH;
pub const WINDOW_HEIGHT: Pixels = token::WEEK_WINDOW_HEIGHT;
const NAV_WIDTH: Pixels = px(240.);
const FIELD_WIDTH: Pixels = px(104.);
/// « Aucune limite » fits in the debt limit.
const DEBT_FIELD_WIDTH: Pixels = px(128.);
const SELECT_WIDTH: Pixels = px(280.);
const SWITCH_WIDTH: Pixels = px(28.);
const SWITCH_HEIGHT: Pixels = px(16.);
const KNOB: Pixels = px(10.);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys(bindings());
}

fn bindings() -> [KeyBinding; 6] {
    [
        KeyBinding::new("enter", Commit, Some(FIELD_CONTEXT)),
        KeyBinding::new("up", StepUp, Some(NUMBER_CONTEXT)),
        KeyBinding::new("down", StepDown, Some(NUMBER_CONTEXT)),
        KeyBinding::new("escape", Escape, Some(KEY_CONTEXT)),
        KeyBinding::new("tab", FocusNext, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(KEY_CONTEXT)),
    ]
}

#[derive(Default)]
struct SettingsWindow(Option<WindowHandle<SettingsView>>);

impl Global for SettingsWindow {}

/// The registration « Démarrage Windows » changes. Without it, as in a demo, start with
/// Windows is unavailable.
pub struct Startup(pub Arc<dyn StartupPort>);

impl Global for Startup {}

/// Opens « Paramètres » on the settings in use, or brings the open window to the front.
pub fn open(cx: &mut App) {
    if let Some(window) = cx.default_global::<SettingsWindow>().0
        && window
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let Some(today) = cx.try_global::<Today>() else {
        return;
    };
    let language = cx.try_global::<Language>().copied().unwrap_or_default();
    let (date, last_sent) = today.usage_ping_dates();
    let usage_ping = UsagePingFacts {
        build: build(),
        released: released(),
        today: date,
        last_sent,
    };
    let form = SettingsForm::new(today.settings().clone(), usage_ping, language);
    let settings_file = today.settings_file().map(PathBuf::from);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(WINDOW_WIDTH, WINDOW_HEIGHT),
            cx,
        ))),
        window_min_size: Some(size(px(640.), px(480.))),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(language.text(Key::SettingsWindowTitle).into()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        focus: true,
        show: true,
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        cx.new(|cx| SettingsView::new(form, settings_file, window, cx))
    }) {
        Ok(window) => cx.default_global::<SettingsWindow>().0 = Some(window),
        Err(error) => log::error!("cannot open « Paramètres »: {error:#}"),
    }
}

/// Opens « Paramètres » on `page`, or brings the open window to the front on it.
pub fn open_on(page: Page, cx: &mut App) {
    open(cx);
    if let Some(window) = cx.default_global::<SettingsWindow>().0 {
        window
            .update(cx, |view, window, cx| view.go_to(page, window, cx))
            .ok();
    }
}

/// « Compter chaque jeudi… »: the settings on « Comptabilisation », with the editor of a new
/// appointment filled in from `proposal`; `false` while their dialog holds the window.
pub fn open_with_proposal(proposal: &AppointmentProposal, cx: &mut App) -> bool {
    open(cx);
    cx.default_global::<SettingsWindow>()
        .0
        .and_then(|window| {
            window
                .update(cx, |view, window, cx| {
                    view.open_proposal(proposal, window, cx)
                })
                .ok()
        })
        .unwrap_or(false)
}

/// « Ne plus proposer pour les jeudis »: through the settings window while open, so that its
/// form keeps the refusal; `false` while their dialog holds the window.
pub fn dismiss_proposal(proposal: &AppointmentProposal, cx: &mut App) -> bool {
    let dismissal = proposal.dismissal(None);
    if let Some(window) = cx.default_global::<SettingsWindow>().0
        && let Ok(recorded) = window.update(cx, |view, window, cx| {
            view.dismiss_proposal(dismissal, window, cx)
        })
    {
        return recorded;
    }
    if cx.has_global::<Today>() {
        host::change(cx, |today| today.dismiss_proposal(dismissal));
    }
    true
}

/// « Compter automatiquement à l’avenir » of the flyout: through the settings window while
/// open, so that its form keeps the change; `false` when nothing was changed, the dialog holding
/// the window, the appointment gone or the save failed.
pub fn count_automatically(appointment_id: &str, cx: &mut App) -> bool {
    if let Some(window) = cx.default_global::<SettingsWindow>().0
        && let Ok(changed) = window.update(cx, |view, window, cx| {
            view.count_automatically(appointment_id, window, cx)
        })
    {
        return changed;
    }
    let mut saved = false;
    if cx.has_global::<Today>() {
        host::change(cx, |today| {
            saved = today.count_automatically(appointment_id)
        });
    }
    saved
}

/// The build the usage signal describes. A Store package is built with `WORKTIME_CHANNEL=store`,
/// the Velopack release with `WORKTIME_CHANNEL=direct` and the `WORKTIME_VERSION` of its package.
pub fn build() -> Build {
    let channel = match option_env!("WORKTIME_CHANNEL") {
        Some("store") => Channel::Store,
        Some("direct") => Channel::Direct,
        _ => Channel::Zip,
    };
    let version = option_env!("WORKTIME_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
    Build::new(version, channel).expect("the build version is one the usage service accepts")
}

/// The day the release was published, which scripts/release-direct.sh passes as
/// `WORKTIME_RELEASE_DATE`; `None` for any other build.
pub fn released() -> Option<jiff::civil::Date> {
    option_env!("WORKTIME_RELEASE_DATE").map(|date| {
        date.parse()
            .expect("release-direct.sh checks that WORKTIME_RELEASE_DATE is a date")
    })
}

/// « Utiliser la détection intégrée » of the flyout. An open « Paramètres » saves it, so that
/// its form goes on from the settings it saved; else the day saves it.
pub fn use_built_in_detection(cx: &mut App) {
    if let Some(window) = cx.default_global::<SettingsWindow>().0
        && window
            .update(cx, |view, window, cx| view.use_built_in(window, cx))
            .is_ok()
    {
        return;
    }
    if !cx.has_global::<Today>() {
        return;
    }
    let mut failure = None;
    host::change(cx, |today| failure = today.use_built_in_detection());
    shell::show_switch(failure, cx);
}

/// Saves through the app's day; without one, as in the screenshots, every save succeeds.
struct HostPort<'a>(&'a mut App);

impl SettingsPort for HostPort<'_> {
    fn save(&mut self, settings: &WorkTimeSettings) -> Result<(), SaveSettingsError> {
        if !self.0.has_global::<Today>() {
            return Ok(());
        }
        let today = self.0.global_mut::<Today>();
        // The form never edits the last Store check, and may predate the one recorded since.
        let mut settings = settings.clone();
        settings.updates.last_checked_at = today.settings().updates.last_checked_at;
        today.save_settings(&settings)
    }

    fn refresh(&mut self) -> bool {
        if !self.0.has_global::<Today>() {
            return true;
        }
        host::change(self.0, Today::refresh);
        self.0.global::<Today>().recalculated()
    }
}

/// What a key did to a time field.
pub(crate) enum TimeKey {
    /// A digit or an arrow changed it; `next` once a digit completed the hours.
    Changed {
        next: bool,
    },
    /// Enter.
    Commit,
    Ignored,
}

/// ↑ and ↓ step `part` of `field`, a digit types into it and Enter commits it.
pub(crate) fn time_key(field: &mut TimeField, part: TimePart, event: &KeyDownEvent) -> TimeKey {
    if event.keystroke.modifiers.modified() {
        return TimeKey::Ignored;
    }
    match event.keystroke.key.as_str() {
        "up" => field.step(part, 1),
        "down" => field.step(part, -1),
        "enter" => return TimeKey::Commit,
        key => {
            let Some(digit) = key.parse::<u8>().ok().filter(|digit| *digit <= 9) else {
                return TimeKey::Ignored;
            };
            let next = field.type_digit(part, digit) && part == TimePart::Hours;
            return TimeKey::Changed { next };
        }
    }
    TimeKey::Changed { next: false }
}

pub struct SettingsView {
    form: SettingsForm,
    language: Language,
    focus_handle: FocusHandle,
    inputs: Vec<Entity<TextInput>>,
    clocks: [ClockFocus; 5],
    /// The name in the editor of « Rendez-vous réguliers ».
    appointment_name: Entity<TextInput>,
    /// The lunch break and the departure: leaving either saves the times of the day.
    lunch: FocusHandle,
    departure: FocusHandle,
    /// A time changed since the times of the day were last committed.
    times_changed: bool,
    language_open: bool,
    /// Starts over each time the language list opens.
    language_entrance: motion::Entrance,
    /// The dialog's button that changes nothing, focused when it shows: « Continuer la
    /// modification », or « Annuler » before an import.
    safe_focus: FocusHandle,
    /// The dialog's button that acts: « Ignorer les changements », or « Remplacer et
    /// redémarrer ».
    action_focus: FocusHandle,
    /// Starts over each time the dialog opens.
    dialog_entrance: motion::Entrance,
    /// Closing waits for the save in progress.
    close_requested: bool,
    /// The box drawing the focus ring of the field inside it, as of this frame.
    focused_field: Option<Field>,
    focused_clock: Option<Clock>,
    json: json::JsonPage,
    data: DataPage,
    /// The import dialog just showed: the next frame focuses it.
    focus_dialog: bool,
    content_scroll: ScrollHandle,
    startup: Option<Arc<dyn StartupPort>>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    /// `settings_file` is `None` for settings kept in memory, as a demo's.
    pub fn new(
        mut form: SettingsForm,
        settings_file: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let startup = cx.try_global::<Startup>().map(|startup| startup.0.clone());
        match startup.clone() {
            Some(port) => {
                cx.spawn(async move |view, cx| {
                    let state = cx
                        .background_executor()
                        .spawn(async move { port.state() })
                        .await;
                    view.update(cx, |view, cx| {
                        view.form.show_startup(state);
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            None => form.show_startup(StartupRegistrationState::Unavailable),
        }
        let language = cx.try_global::<Language>().copied().unwrap_or_default();
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let inputs: Vec<_> = Field::ALL
            .into_iter()
            .map(|field| {
                cx.new(|cx| {
                    let mut input =
                        TextInput::new(field.name(language), field.placeholder(language), None, cx);
                    input.set_text(form.text(field), cx);
                    input.frameless = field.spin().is_some();
                    input.dim_border_when_disabled = true;
                    input
                })
            })
            .collect();
        let mut subscriptions = Vec::new();
        for (field, input) in Field::ALL.into_iter().zip(&inputs) {
            let handle = input.focus_handle(cx);
            subscriptions.push(cx.on_blur(&handle, window, move |view, window, cx| {
                view.commit_field(field, false, window, cx);
            }));
        }
        let clocks = Clock::ALL.map(|_| ClockFocus::new(cx));
        // A digit typed after the focus came back starts the part over.
        for clock in Clock::ALL {
            let focus = &clocks[clock as usize];
            for handle in [&focus.hours, &focus.minutes] {
                subscriptions.push(cx.on_blur(handle, window, move |view, _, _| {
                    view.form.time_mut(clock).leave();
                }));
            }
        }
        // Groups around the time fields, whose parts are the tab stops.
        let (lunch, departure) = (cx.focus_handle(), cx.focus_handle());
        for handle in [&lunch, &departure] {
            subscriptions.push(cx.on_focus_out(handle, window, |view, _, window, cx| {
                view.commit_times(false, window, cx);
            }));
        }
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            view.update(cx, |view, cx| view.should_close(window, cx))
                .unwrap_or(true)
        });
        Self {
            form,
            language,
            focus_handle,
            inputs,
            clocks,
            appointment_name: cx.new(|cx| {
                TextInput::new(
                    language.text(Key::SettingsAppointmentNameLabel),
                    "",
                    Some(RecurringAppointment::MAXIMUM_NAME_LENGTH),
                    cx,
                )
            }),
            lunch,
            departure,
            times_changed: false,
            language_open: false,
            language_entrance: motion::Entrance::default(),
            safe_focus: cx.focus_handle(),
            action_focus: cx.focus_handle(),
            dialog_entrance: motion::Entrance::default(),
            close_requested: false,
            focused_field: None,
            focused_clock: None,
            json: json::JsonPage::new(settings_file, widgets::tab_stop(cx)),
            data: {
                let directory = transfer::data_directory(cx);
                let backup = directory.as_ref().and_then(transfer::latest_backup);
                DataPage::new(language, directory, backup)
            },
            focus_dialog: false,
            content_scroll: ScrollHandle::new(),
            startup,
            _subscriptions: subscriptions,
        }
    }

    /// Shows the end of the page, for the screenshots.
    pub fn scroll_to_end(&mut self) {
        self.content_scroll.scroll_to_bottom();
        self.json.scroll_to_end();
    }

    /// Names `folder` in the header of « settings.json » in place of the file's own, for the
    /// screenshots.
    pub fn show_folder_as(&mut self, folder: &str) {
        self.json.folder = Some(folder.to_owned());
    }

    /// The form, read by the headless tests.
    pub fn form(&self) -> &SettingsForm {
        &self.form
    }

    /// Opens the language list under its button, or closes it.
    pub fn toggle_language_list(&mut self, cx: &mut Context<Self>) {
        self.language_open = !self.language_open;
        cx.notify();
    }

    /// The form of a window not tied to the app's day, for the screenshots.
    pub fn form_mut(&mut self) -> &mut SettingsForm {
        &mut self.form
    }

    /// Types `text` in `field` as the user would, for the screenshots.
    pub fn type_text(&mut self, field: Field, text: &str, cx: &mut Context<Self>) {
        self.input(field)
            .update(cx, |input, cx| input.set_text(text, cx));
        self.form.set_text(field, text);
    }

    /// Shows the drafts of the form again in the fields, after a change made on the form.
    pub fn show_drafts(&mut self, cx: &mut Context<Self>) {
        for field in Field::ALL {
            let draft = self.form.text(field).to_owned();
            self.input(field).update(cx, |input, cx| {
                if input.text() != draft {
                    input.set_text(&draft, cx);
                }
            });
        }
        cx.notify();
    }

    fn input(&self, field: Field) -> &Entity<TextInput> {
        &self.inputs[field as usize]
    }

    fn text(&self, key: Key) -> &'static str {
        self.language.text(key)
    }

    /// Runs the save `pending` asks for once the saving state has been drawn.
    fn submit(&mut self, pending: Option<Pending>, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        let Some(pending) = pending else {
            return;
        };
        if let Some(requested) = pending.startup() {
            self.change_startup(pending, requested, window, cx);
            return;
        }
        cx.spawn_in(window, async move |view, cx| {
            view.update_in(cx, |view, window, cx| {
                view.form.run(pending, &mut HostPort(cx));
                view.saved(window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Asks Windows for the change off the window's thread, then saves what it settled on.
    fn change_startup(
        &mut self,
        pending: Pending,
        requested: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let port = self.startup.clone();
        cx.spawn_in(window, async move |view, cx| {
            let settled = match port {
                Some(port) => {
                    cx.background_executor()
                        .spawn(async move { startup::apply(&*port, requested) })
                        .await
                }
                None => StartupRegistrationState::Unavailable,
            };
            view.update_in(cx, |view, window, cx| {
                view.form
                    .finish_startup(pending, settled, &mut HostPort(cx));
                view.saved(window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Closes the window once the last save it waited for is done.
    fn saved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_requested && !self.form.is_saving() {
            self.close_requested = false;
            self.close(window, cx);
        }
        cx.notify();
    }

    /// Commits the text of `field`: on Enter, or when it loses the focus with a changed text.
    fn commit_field(
        &mut self,
        field: Field,
        always: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.form.is_field_enabled(field) {
            return;
        }
        let text = self.input(field).read(cx).text().to_owned();
        if !always && text == self.form.text(field) {
            return;
        }
        self.form.set_text(field, &text);
        let pending = self.form.commit(field.group());
        self.submit(pending, window, cx);
    }

    fn commit_times(&mut self, always: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !always && !self.times_changed {
            return;
        }
        self.times_changed = false;
        let pending = self.form.commit(EditGroup::DayTimes);
        self.submit(pending, window, cx);
    }

    /// Commits the field that has the focus, as leaving it would.
    fn commit_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for field in Field::ALL {
            if self.input(field).focus_handle(cx).is_focused(window) {
                self.commit_field(field, false, window, cx);
            }
        }
        if self.lunch.contains_focused(window, cx) || self.departure.contains_focused(window, cx) {
            self.commit_times(false, window, cx);
        }
    }

    fn spin(&mut self, field: Field, steps: i64, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input(field).read(cx).text().to_owned();
        self.form.set_text(field, &text);
        let pending = self.form.spin(field, steps);
        let draft = self.form.text(field).to_owned();
        self.input(field)
            .update(cx, |input, cx| input.set_text(&draft, cx));
        self.submit(pending, window, cx);
    }

    fn toggle(&mut self, switch: Switch, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.form.toggle(switch);
        self.submit(pending, window, cx);
    }

    fn choose_language(
        &mut self,
        choice: LanguageChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.language_open = false;
        let pending = self.form.choose_language(choice);
        self.submit(pending, window, cx);
    }

    fn retry(&mut self, group: EditGroup, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.form.retry(group);
        self.submit(pending, window, cx);
    }

    fn use_built_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.form.use_built_in();
        self.submit(pending, window, cx);
    }

    fn go_to(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_focused(window, cx);
        self.language_open = false;
        if !self.form.go_to(page) {
            window.focus(&self.safe_focus, cx);
        }
        cx.notify();
    }

    /// Whether the window may close now; else it closes after the save in progress, or the
    /// unsaved-changes dialog shows.
    pub fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.commit_focused(window, cx);
        cx.notify();
        if self.form.is_saving() {
            self.close_requested = true;
            return false;
        }
        let may = self.form.may_close();
        if !may {
            window.focus(&self.safe_focus, cx);
        }
        may
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.should_close(window, cx) {
            window.remove_window();
        }
    }

    fn keep_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form.keep_editing();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let leaving = self.form.discard();
        self.times_changed = false;
        self.show_drafts(cx);
        window.focus(&self.focus_handle, cx);
        if leaving == Some(Leaving::Window) {
            window.remove_window();
        }
    }

    /// Tab moves through the window, or between the two buttons of the dialog while it shows.
    fn cycle_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.dialog().is_some() || self.data.dialog().is_some() {
            let next = if self.safe_focus.is_focused(window) {
                &self.action_focus
            } else {
                &self.safe_focus
            };
            window.focus(next, cx);
        } else if forward {
            window.focus_next(cx);
        } else {
            window.focus_prev(cx);
        }
    }

    fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.data.dialog().is_some() {
            self.cancel_import(window, cx);
        } else if self.form.dialog().is_some() {
            self.keep_editing(window, cx);
        } else if self.language_open {
            self.language_open = false;
            cx.notify();
        }
    }

    fn time_key(
        &mut self,
        clock: Clock,
        part: TimePart,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match time_key(self.form.time_mut(clock), part, event) {
            TimeKey::Ignored => return,
            // The editor's hours save with it.
            TimeKey::Commit if clock.is_appointment() => self.save_appointment(window, cx),
            TimeKey::Commit => self.commit_times(true, window, cx),
            TimeKey::Changed { next } => {
                if next {
                    let minutes = self.clocks[clock as usize].minutes.clone();
                    window.focus(&minutes, cx);
                }
                self.times_changed |= !clock.is_appointment();
                cx.notify();
            }
        }
        cx.stop_propagation();
    }
}

pub(crate) fn spinner(theme: &Theme) -> impl IntoElement {
    widgets::icon("loader-circle", theme.text_muted)
        .size(token::ICON_XS)
        .with_animation(
            "saving",
            Animation::new(Duration::from_millis(900)).repeat(),
            |icon, delta| icon.with_transformation(Transformation::rotate(percentage(delta))),
        )
}

pub(crate) fn field_error(message: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .id("field-error")
        .role(Role::Alert)
        .aria_label(message)
        .mt(BASE_04)
        .flex()
        .items_start()
        .gap(BASE_06)
        .type_style(text::LABEL_SM)
        .text_color(theme.error)
        .child(
            div()
                .h(text::LABEL_SM.line_height)
                .flex()
                .items_center()
                .child(widgets::icon("circle-alert", theme.error).size(token::ICON_SM)),
        )
        .child(div().flex_1().min_w_0().child(message))
}

/// The outlined button of a callout or of the dialog, `height` tall.
pub(crate) fn outlined(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    height: Pixels,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let (hover, active, focused) = (
        theme.element_hover,
        theme.element_active,
        theme.border_focused,
    );
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .flex_none()
        .h(height)
        .px(BASE_12)
        .flex()
        .items_center()
        .rounded(radius::SM)
        .border_1()
        .border_color(theme.border)
        .bg(theme.editor)
        .whitespace_nowrap()
        .tab_index(0)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(active))
        .focus_visible(move |style| style.border_color(focused))
        .child(label)
}

/// The button a form or a dialog is for, filled with the accent.
pub(crate) fn filled(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    height: Pixels,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let focused = theme.border_focused;
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .flex_none()
        .h(height)
        .px(BASE_12)
        .flex()
        .items_center()
        .rounded(radius::SM)
        .border_1()
        .border_color(theme.accent)
        .bg(theme.accent)
        .text_color(theme.tl_on_fill)
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .tab_index(0)
        .cursor_pointer()
        .hover(move |style| style.border_color(focused))
        .focus_visible(move |style| style.border_color(focused))
        .child(label)
}

/// [`outlined`] with a control border that stays visible on a callout tint, where `border` all
/// but disappears in the dark theme.
pub(crate) fn outlined_on_tint(
    id: &'static str,
    label: &'static str,
    height: Pixels,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    outlined(id, label, height, theme).border_color(theme.border_on_tint)
}

impl SettingsView {
    fn title_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        widgets::title_bar(
            self.text(Key::SettingsWindowTitle),
            [
                widgets::control("minimize", "minus", WindowControlArea::Min, theme)
                    .on_click(|_, window, _| window.minimize_window()),
                widgets::control("maximize", "square", WindowControlArea::Max, theme)
                    .on_click(|_, window, _| window.zoom_window()),
            ],
            cx.listener(|view, _, window, cx| view.close(window, cx)),
            theme,
        )
    }

    fn nav(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.form.page();
        let mut item = |page: Page| {
            let selected = page == current;
            let label = page.label(self.language);
            let (hover, focused) = (theme.element_hover, theme.border_focused);
            div()
                .id(("page", page as usize))
                .role(Role::Tab)
                .aria_label(label)
                .aria_selected(selected)
                .flex_none()
                .h(token::BUTTON_MEDIUM)
                .px(BASE_08)
                .flex()
                .items_center()
                .gap(BASE_08)
                .rounded(radius::SM)
                .border_1()
                .border_color(transparent_black())
                .when(selected, |item| item.bg(theme.element_active))
                .tab_index(0)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .focus_visible(move |style| style.border_color(focused))
                .child(widgets::icon(page.icon(), theme.text_muted).size(token::ICON_MD))
                .child(div().flex_1().min_w_0().child(label))
                .when(self.form.warns(page), |item| {
                    item.child(div().size(px(6.)).rounded_full().bg(theme.warning))
                })
                .on_click(cx.listener(move |view, _, window, cx| view.go_to(page, window, cx)))
        };
        let items: Vec<_> = Page::ALL.into_iter().map(&mut item).collect();
        let json = div()
            .mt_auto()
            .pt(BASE_08)
            .border_t_1()
            .border_color(theme.border_variant)
            .flex()
            .flex_col()
            .gap(BASE_02)
            .child(item(Page::Data))
            .child(item(Page::Json));
        div()
            .id("pages")
            .role(Role::TabList)
            .aria_label(self.text(Key::SettingsWindowTitle))
            .w(NAV_WIDTH)
            .flex_none()
            .h_full()
            .py(BASE_12)
            .px(BASE_08)
            .flex()
            .flex_col()
            .gap(BASE_02)
            .bg(theme.surface)
            .border_r_1()
            .border_color(theme.border_variant)
            .children(items)
            .child(json)
    }

    fn section(
        &mut self,
        position: usize,
        section: SectionView,
        language_time: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let count = section.rows.len();
        let rows: Vec<_> = section
            .rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                self.row(row, index + 1 == count, language_time, theme, cx)
                    .into_any_element()
            })
            .collect();
        let callouts: Vec<_> = section
            .callouts
            .into_iter()
            .map(|callout| self.callout(callout, theme, cx).into_any_element())
            .collect();
        group(("section", position), section.heading, rows, theme).children(callouts)
    }

    fn row(
        &mut self,
        row: RowView,
        last: bool,
        language_time: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let control = self.control(row.control.clone(), language_time, theme, cx);
        let details = row.details.clone().map(|details| {
            ping_details(details, theme, cx, |view: &mut Self| {
                view.form.toggle_details()
            })
        });
        row_frame((&row).into(), control, details, last, theme)
    }
}

/// What a row of a settings group says beside its control.
pub(crate) struct RowText {
    pub title: SharedString,
    pub description: SharedString,
    pub tone: RowTone,
    pub link: Option<Link>,
    pub status: Option<SharedString>,
    pub error: Option<&'static str>,
}

impl From<&RowView> for RowText {
    fn from(row: &RowView) -> Self {
        Self {
            title: row.title.into(),
            description: row.description.into(),
            tone: row.tone,
            link: row.link,
            status: row.status.map(SharedString::from),
            error: row.error,
        }
    }
}

/// The bordered surface of a settings group, which holds its rows.
pub(crate) fn card(theme: &Theme) -> gpui::Div {
    div()
        .px(BASE_16)
        .rounded(radius::LG)
        .border_1()
        .border_color(theme.border_variant)
        .bg(theme.surface)
}

/// A group of « Paramètres »: its heading over its rows in their card.
fn group(
    id: impl Into<gpui::ElementId>,
    heading: Option<&'static str>,
    rows: impl IntoIterator<Item = gpui::AnyElement>,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(BASE_08)
        .children(heading.map(|heading| {
            div()
                .id("heading")
                .role(Role::Heading)
                .type_style(text::HEADLINE_SM)
                .child(heading)
        }))
        .child(card(theme).children(rows))
}

/// The line of a row: its title and description beside its control.
fn row_line(text: gpui::Div, control: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(BASE_16)
        .child(text)
        .child(control)
}

/// The title of a row over its description, beside the row's control.
fn row_text(
    title: impl Into<SharedString>,
    title_color: gpui::Hsla,
    description: Option<gpui::AnyElement>,
    description_color: gpui::Hsla,
) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(BASE_02)
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(title_color)
                .child(title.into()),
        )
        .children(description.map(|description| {
            div()
                .type_style(text::LABEL_SM)
                .text_color(description_color)
                .child(description)
        }))
}

/// A row of a settings group: its title, description and status beside its control, then its
/// error and its details.
pub(crate) fn row_frame(
    row: RowText,
    control: gpui::AnyElement,
    details: Option<gpui::Div>,
    last: bool,
    theme: &Theme,
) -> gpui::Div {
    let (title_color, description_color) = match row.tone {
        RowTone::Normal => (theme.text, theme.text_muted),
        RowTone::Unavailable => (theme.text_muted, theme.text_muted),
        RowTone::Disabled => (theme.text_placeholder, theme.text_placeholder),
    };
    div()
        .py(BASE_12)
        .flex()
        .flex_col()
        .when(!last, |row| {
            row.border_b_1().border_color(theme.border_variant)
        })
        .child(row_line(
            row_text(
                row.title,
                title_color,
                Some(row_description(row.description, row.link, theme)),
                description_color,
            )
            .children(row.status.map(|status| {
                div()
                    .id("startup-status")
                    .role(Role::Status)
                    .aria_label(status.clone())
                    .pt(BASE_04)
                    .type_style(text::LABEL_SM)
                    .text_color(theme.text_muted)
                    .child(status)
            })),
            control,
        ))
        .children(row.error.map(|error| field_error(error, theme)))
        .children(details)
}

/// A description, its link in the accent colour of « Voir ce qui est envoyé » and wrapping with
/// the sentence. GPUI cannot focus a range of text, so Tab reaches the whole description and
/// Enter opens the link; a click opens it only on the link's word.
fn row_description(
    description: SharedString,
    link: Option<Link>,
    theme: &Theme,
) -> gpui::AnyElement {
    let Some(link) = link else {
        return description.into_any_element();
    };
    let start = description
        .find(link.label)
        .expect("the description names its link");
    let range = start..start + link.label.len();
    let accent = HighlightStyle {
        color: Some(theme.accent),
        ..HighlightStyle::default()
    };
    let url = link.url;
    let text = InteractiveText::new(
        "description-text",
        StyledText::new(description.clone()).with_highlights([(range.clone(), accent)]),
    )
    .on_click(vec![range], move |_, _, cx| cx.open_url(url));
    let focused = theme.border_focused;
    div()
        .id("description")
        .role(Role::Link)
        .aria_label(description)
        .m(px(-1.))
        .rounded(radius::SM)
        .border_1()
        .border_color(transparent_black())
        .tab_index(0)
        .focus_visible(move |style| style.border_color(focused))
        .on_click(move |event, _, cx| {
            if event.is_keyboard() {
                cx.open_url(url);
            }
        })
        .child(text)
        .into_any_element()
}

/// « Voir ce qui est envoyé », and what it shows once open; `toggle` opens or closes it.
pub(crate) fn ping_details<V: 'static>(
    details: Details,
    theme: &Theme,
    cx: &mut Context<V>,
    toggle: fn(&mut V),
) -> gpui::Div {
    let open = details.shown.is_some();
    let button = widgets::disclosure("ping-details-toggle", details.label, open, theme).on_click(
        cx.listener(move |view, _, _, cx| {
            toggle(view);
            cx.notify();
        }),
    );
    let shown = details.shown.map(|shown| {
        let note = |text: String| {
            div()
                .type_style(text::LABEL_SM)
                .text_color(theme.text_muted)
                .child(text)
        };
        let body = |body: String| {
            div().type_style(text::WT_VALUE_SM).children(
                body.lines()
                    .map(|line| div().whitespace_nowrap().child(json::line(line, theme))),
            )
        };
        let content = div().flex().flex_col().gap(BASE_08);
        let content = match shown {
            PingDetails::On {
                body: json,
                fields,
                server,
                last_sent,
            } => content
                .child(body(json))
                .child(note(fields))
                .child(note(server.to_owned()))
                .child(note(last_sent).font_features(tabular_figures())),
            PingDetails::Brief {
                body: json,
                note: text,
            } => content.child(body(json)).child(note(text)),
            PingDetails::Off(text) => content.child(note(text.to_owned())),
        };
        div()
            .id("ping-details")
            .role(Role::Group)
            .aria_label(details.label)
            .p(BASE_12)
            .rounded(radius::MD)
            .border_1()
            .border_color(theme.border_variant)
            .bg(theme.editor)
            .child(content)
    });
    div()
        .pt(BASE_04)
        .flex()
        .flex_col()
        .items_start()
        .gap(BASE_08)
        .child(button)
        .children(shown.map(|shown| shown.w_full()))
}

impl SettingsView {
    /// ActivityWatch is no longer read: the callout above the groups of « Général ».
    fn source_callout(
        &self,
        callout: SourceCallout,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(BASE_08)
            .child(
                widgets::callout_box(Hue::Warning, theme)
                    .id("activitywatch")
                    .role(Role::Status)
                    .aria_label(callout.title)
                    .aria_description(callout.message)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(BASE_02)
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(callout.title))
                            .child(div().type_style(text::LABEL_SM).child(callout.message)),
                    )
                    .child(
                        outlined_on_tint(
                            "use-built-in",
                            callout.action,
                            token::BUTTON_MEDIUM,
                            theme,
                        )
                        .self_center()
                        .on_click(cx.listener(|view, _, window, cx| view.use_built_in(window, cx))),
                    ),
            )
            .children(
                callout
                    .failure
                    .map(|failure| self.callout(failure, theme, cx)),
            )
    }

    fn control(
        &mut self,
        control: Control,
        language_time: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match control {
            Control::Language { name, value } => {
                self.language_select(name, value, language_time, theme, cx)
            }
            Control::Text {
                field,
                invalid,
                enabled,
            } => self
                .text_field(field, invalid, enabled, cx)
                .into_any_element(),
            Control::Number {
                field,
                unit,
                invalid,
                enabled,
            } => self
                .number_field(field, unit, invalid, enabled, theme, cx)
                .into_any_element(),
            Control::Time { clock, invalid } => div()
                .id("departure")
                .track_focus(&self.departure)
                .child(self.time_field(clock, invalid, theme, cx))
                .into_any_element(),
            Control::Range {
                start,
                end,
                invalid,
            } => div()
                .id("lunch")
                .role(Role::Group)
                .track_focus(&self.lunch)
                .flex()
                .items_center()
                .gap(BASE_08)
                .child(self.time_field(start, invalid, theme, cx))
                .child(div().text_color(theme.text_muted).child("–"))
                .child(self.time_field(end, invalid, theme, cx))
                .into_any_element(),
            Control::Switch {
                switch,
                name,
                on,
                enabled,
            } => self
                .switch(switch, name, on, enabled, theme, cx)
                .into_any_element(),
            Control::Rule {
                field,
                unit,
                invalid,
                switch,
                name,
                on,
            } => div()
                .flex()
                .items_center()
                .gap(BASE_16)
                .child(self.number_field(field, unit, invalid, on, theme, cx))
                .child(self.switch(switch, name, on, true, theme, cx))
                .into_any_element(),
        }
    }

    fn text_field(
        &mut self,
        field: Field,
        invalid: bool,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let input = self.input(field).clone();
        input.update(cx, |input, _| {
            input.disabled = !enabled;
            input.invalid = invalid;
        });
        div()
            .id(("field", field as usize))
            .key_context(FIELD_CONTEXT)
            .on_action(cx.listener(move |view, _: &Commit, window, cx| {
                view.commit_field(field, true, window, cx)
            }))
            .w(if field == Field::DebtLimit {
                DEBT_FIELD_WIDTH
            } else {
                FIELD_WIDTH
            })
            .flex_none()
            .child(input)
    }

    fn number_field(
        &mut self,
        field: Field,
        unit: &'static str,
        invalid: bool,
        enabled: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let input = self.input(field).clone();
        input.update(cx, |input, _| {
            input.disabled = !enabled;
            input.invalid = invalid;
        });
        let focused = enabled && self.focused_field == Some(field);
        let border = if invalid {
            theme.error
        } else if focused {
            theme.border_focused
        } else if enabled {
            theme.border
        } else {
            theme.border_variant
        };
        let step = |id: &'static str, icon: &'static str, steps: i64| {
            let hover = theme.element_hover;
            let color = if enabled {
                theme.text_muted
            } else {
                theme.text_placeholder
            };
            let button = div()
                .id(id)
                .w(px(14.))
                .h(px(12.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(radius::XS)
                .child(widgets::icon(icon, color).size(px(10.)));
            if !enabled {
                return button;
            }
            button
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(
                    cx.listener(move |view, _, window, cx| view.spin(field, steps, window, cx)),
                )
        };
        div()
            .id(("number", field as usize))
            .key_context(if enabled {
                NUMBER_FIELD_CONTEXT
            } else {
                FIELD_CONTEXT
            })
            .when(enabled, |number| {
                number
                    .on_action(cx.listener(move |view, _: &Commit, window, cx| {
                        view.commit_field(field, true, window, cx)
                    }))
                    .on_action(cx.listener(move |view, _: &StepUp, window, cx| {
                        view.spin(field, 1, window, cx)
                    }))
                    .on_action(cx.listener(move |view, _: &StepDown, window, cx| {
                        view.spin(field, -1, window, cx)
                    }))
            })
            .w(FIELD_WIDTH)
            .flex_none()
            .h(token::BUTTON_MEDIUM)
            .pl(BASE_08)
            .pr(BASE_04)
            .flex()
            .items_center()
            .gap(BASE_04)
            .rounded(radius::SM)
            .border_1()
            .border_color(border)
            .bg(if enabled { theme.editor } else { theme.element })
            .child(div().flex_1().min_w_0().child(input))
            .child(
                div()
                    .type_style(text::LABEL_SM)
                    .text_color(if enabled {
                        theme.text_muted
                    } else {
                        theme.text_placeholder
                    })
                    .child(unit),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(step(spin_id(field, true), "chevron-up", 1))
                    .child(step(spin_id(field, false), "chevron-down", -1)),
            )
    }

    fn time_field(
        &self,
        clock: Clock,
        invalid: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        time_field(
            clock,
            self.form.time(clock),
            &self.clocks[clock as usize],
            self.focused_clock == Some(clock),
            invalid,
            self.language,
            theme,
            cx,
            Self::time_key,
        )
    }

    fn switch(
        &self,
        switch: Switch,
        name: &'static str,
        on: bool,
        enabled: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        switch_control(switch, name, on, enabled, theme, cx, Self::toggle)
    }
}

/// A time of day as hours and minutes, each a spin button; `on_key` takes their keys. A press
/// beside them gives the hours the focus.
#[allow(clippy::too_many_arguments)]
pub(crate) fn time_field<V: 'static>(
    clock: Clock,
    value: TimeField,
    focus: &ClockFocus,
    focused: bool,
    invalid: bool,
    language: Language,
    theme: &Theme,
    cx: &mut Context<V>,
    on_key: fn(&mut V, Clock, TimePart, &KeyDownEvent, &mut Window, &mut Context<V>),
) -> gpui::Stateful<gpui::Div> {
    let segment = |part: TimePart, handle: &FocusHandle, label: Key| {
        let (accent, accent_background) = (theme.accent, theme.accent_background);
        div()
            .id(segment_id(clock, part))
            .role(Role::SpinButton)
            .aria_label(language.text(label))
            .aria_numeric_value(f64::from(value.value(part)))
            .aria_min_numeric_value(0.)
            .aria_max_numeric_value(match part {
                TimePart::Hours => 23.,
                TimePart::Minutes => 59.,
            })
            .rounded(radius::XS)
            .track_focus(handle)
            .focus(move |style| style.bg(accent_background).text_color(accent))
            .on_key_down(cx.listener(move |view, event, window, cx| {
                on_key(view, clock, part, event, window, cx)
            }))
            .child(value.part_text(part))
    };
    let hours = focus.hours.clone();
    div()
        .id(clock_id(clock))
        .role(Role::Group)
        .aria_label(clock.name(language))
        .w(FIELD_WIDTH)
        .flex_none()
        .h(token::BUTTON_MEDIUM)
        .px(BASE_08)
        .flex()
        .items_center()
        .rounded(radius::SM)
        .border_1()
        .border_color(if invalid {
            theme.error
        } else if focused {
            theme.border_focused
        } else {
            theme.border
        })
        .bg(theme.editor)
        .font_features(tabular_figures())
        .cursor_text()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            widgets::focus_unless_taken(&hours, window, cx)
        })
        .child(segment(
            TimePart::Hours,
            &focus.hours,
            Key::TimeFieldHoursAutomationPropertiesName,
        ))
        .child(":")
        .child(segment(
            TimePart::Minutes,
            &focus.minutes,
            Key::TimeFieldMinutesAutomationPropertiesName,
        ))
}

/// A switch; a click on it while `enabled` calls `toggle`.
pub(crate) fn switch_control<V: 'static>(
    switch: Switch,
    name: &'static str,
    on: bool,
    enabled: bool,
    theme: &Theme,
    cx: &mut Context<V>,
    toggle: fn(&mut V, Switch, &mut Window, &mut Context<V>),
) -> gpui::Stateful<gpui::Div> {
    let control = switch_frame(switch_id(switch), name, on, enabled, theme);
    if !enabled {
        return control;
    }
    control.on_click(cx.listener(move |view, _, window, cx| toggle(view, switch, window, cx)))
}

/// A switch as drawn, focusable and clickable while `enabled`.
pub(crate) fn switch_frame(
    id: impl Into<gpui::ElementId>,
    name: impl Into<SharedString>,
    on: bool,
    enabled: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let (track, border, knob) = match (enabled, on) {
        (true, true) => (theme.accent, theme.accent, theme.tl_on_fill),
        (true, false) => (transparent_black(), theme.text_muted, theme.text_muted),
        (false, true) => (theme.element_active, theme.border, theme.text_placeholder),
        (false, false) => (theme.element, theme.border, theme.text_placeholder),
    };
    let focused = theme.border_focused;
    let control = div()
        .id(id)
        .role(Role::Switch)
        .aria_label(name)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .w(SWITCH_WIDTH)
        .h(SWITCH_HEIGHT)
        .flex_none()
        .px(px(2.))
        .flex()
        .items_center()
        .when(on, |control| control.justify_end())
        .rounded_full()
        .border_1()
        .border_color(border)
        .bg(track)
        .child(div().size(KNOB).rounded_full().bg(knob));
    if !enabled {
        return control;
    }
    control
        .tab_index(0)
        .cursor_pointer()
        .focus_visible(move |style| style.border_color(focused))
}

impl From<Tone> for Hue {
    fn from(tone: Tone) -> Self {
        match tone {
            Tone::Error => Hue::Error,
            Tone::Warning => Hue::Warning,
        }
    }
}

impl SettingsView {
    fn callout(
        &self,
        callout: CalloutView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let role = match callout.tone {
            Tone::Error => Role::Alert,
            Tone::Warning => Role::Status,
        };
        let group = callout.group;
        widgets::inline_callout_box(callout.tone.into(), theme)
            .id(("callout", group as usize))
            .role(role)
            .aria_label(callout.message)
            .child(div().flex_1().min_w_0().child(callout.message))
            .child(
                outlined_on_tint("retry", callout.retry, token::BUTTON_MEDIUM, theme).on_click(
                    cx.listener(move |view, _, window, cx| view.retry(group, window, cx)),
                ),
            )
    }
}

fn spin_id(field: Field, up: bool) -> &'static str {
    match (field, up) {
        (Field::Margin, true) => "margin-up",
        (Field::Margin, false) => "margin-down",
        (Field::LongLock, true) => "long-lock-up",
        (Field::LongLock, false) => "long-lock-down",
        (Field::CompletionLead, true) => "completion-up",
        (Field::CompletionLead, false) => "completion-down",
        (Field::IdleThreshold, true) => "idle-threshold-up",
        (Field::IdleThreshold, false) => "idle-threshold-down",
        (_, true) => "departure-up",
        (_, false) => "departure-down",
    }
}

fn clock_id(clock: Clock) -> &'static str {
    match clock {
        Clock::LunchStart => "lunch-start",
        Clock::LunchEnd => "lunch-end",
        Clock::Departure => "departure-time",
        Clock::AppointmentStart => "appointment-start",
        Clock::AppointmentEnd => "appointment-end",
    }
}

fn segment_id(clock: Clock, part: TimePart) -> &'static str {
    match (clock, part) {
        (Clock::LunchStart, TimePart::Hours) => "lunch-start-hours",
        (Clock::LunchStart, TimePart::Minutes) => "lunch-start-minutes",
        (Clock::LunchEnd, TimePart::Hours) => "lunch-end-hours",
        (Clock::LunchEnd, TimePart::Minutes) => "lunch-end-minutes",
        (Clock::Departure, TimePart::Hours) => "departure-hours",
        (Clock::Departure, TimePart::Minutes) => "departure-minutes",
        (Clock::AppointmentStart, TimePart::Hours) => "appointment-start-hours",
        (Clock::AppointmentStart, TimePart::Minutes) => "appointment-start-minutes",
        (Clock::AppointmentEnd, TimePart::Hours) => "appointment-end-hours",
        (Clock::AppointmentEnd, TimePart::Minutes) => "appointment-end-minutes",
    }
}

fn switch_id(switch: Switch) -> &'static str {
    match switch {
        Switch::Startup => "startup",
        Switch::Credit => "credit",
        Switch::Debt => "debt",
        Switch::Completion => "completion",
        Switch::DepartureNotification => "departure-notification",
        Switch::DayComplete => "day-complete",
        Switch::Updates => "updates",
        Switch::UsagePing => "usage-ping",
        Switch::AppointmentProposals => "appointment-proposals",
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        self.focused_field = Field::ALL
            .into_iter()
            .find(|&field| self.input(field).focus_handle(cx).is_focused(window));
        self.focused_clock = Clock::ALL
            .into_iter()
            .find(|&clock| self.clocks[clock as usize].is_focused(window));
        let language_time = if self.language_open {
            self.language_entrance.time(motion::MENU, window, cx)
        } else {
            self.language_entrance.reset();
            1.
        };
        let on_json = self.form.page() == Page::Json;
        self.json.show(on_json);
        let body = match self.form.page() {
            Page::Json => self.json.render(self.language, &theme, window, cx),
            Page::Data => self.data_page(&theme, cx).into_any_element(),
            _ => {
                let page = self.form.view();
                self.form_page(page, language_time, &theme, cx)
                    .into_any_element()
            }
        };
        let shown = match self.data.dialog() {
            Some(dialog) => Some(Dialog::Import(dialog)),
            None => self.form.dialog().map(Dialog::Unsaved),
        };
        let dialog = match shown {
            Some(dialog) => {
                if std::mem::take(&mut self.focus_dialog) {
                    window.focus(&self.safe_focus, cx);
                }
                let time = self.dialog_entrance.time(motion::DIALOG, window, cx);
                Some(self.dialog(dialog, time, &theme, cx).into_any_element())
            }
            None => {
                self.dialog_entrance.reset();
                None
            }
        };
        div()
            .id("settings")
            .role(Role::Window)
            .aria_label(self.text(Key::SettingsWindowTitle))
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::escape))
            .on_action(
                cx.listener(|view, _: &FocusNext, window, cx| view.cycle_focus(true, window, cx)),
            )
            .on_action(cx.listener(|view, _: &FocusPrevious, window, cx| {
                view.cycle_focus(false, window, cx)
            }))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.editor)
            .text_color(theme.text)
            .type_style(text::LABEL_MD)
            .child(self.title_bar(&theme, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.nav(&theme, cx))
                    .child(body),
            )
            .children(dialog)
    }
}

impl SettingsView {
    /// `language_time` is how far the open language list has come in.
    fn form_page(
        &mut self,
        page: crate::settings::PageView,
        language_time: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let source = page
            .source
            .map(|callout| self.source_callout(callout, theme, cx).into_any_element());
        let sections: Vec<_> = page
            .sections
            .into_iter()
            .enumerate()
            .map(|(position, section)| {
                self.section(position, section, language_time, theme, cx)
                    .into_any_element()
            })
            .collect();
        let appointments = self
            .form
            .appointments()
            .map(|appointments| self.appointments_section(appointments, theme, cx));
        self.page_body(
            page.title,
            page.status.map(SharedString::from),
            page.saving,
            source
                .into_iter()
                .chain(sections)
                .chain(appointments)
                .chain(
                    page.about
                        .map(|about| about_section(about, theme).into_any_element()),
                ),
            theme,
        )
    }

    /// A page: its header, then `content` scrolling under it.
    fn page_body(
        &self,
        title: &'static str,
        status: Option<SharedString>,
        busy: bool,
        content: impl IntoIterator<Item = gpui::AnyElement>,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id("page")
            .role(Role::TabPanel)
            .aria_label(title)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(page_header(title, status, busy, theme))
            .child(
                div()
                    .id("content")
                    .track_scroll(&self.content_scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(BASE_24)
                    .pb(BASE_24)
                    .flex()
                    .flex_col()
                    .gap(BASE_20)
                    .children(content),
            )
    }
}

/// « À propos »: the version where a row has its control, the day it was published as its
/// description, and no description when the build does not know it.
fn about_section(about: AboutView, theme: &Theme) -> impl IntoElement {
    let version = div()
        .id("version")
        .role(Role::Label)
        .aria_label(about.label)
        .flex_none()
        .whitespace_nowrap()
        .child(about.version);
    let text = row_text(
        about.title,
        theme.text,
        about.released.map(IntoElement::into_any_element),
        theme.text_muted,
    );
    let row = div().py(BASE_12).child(row_line(text, version));
    group(
        "about",
        Some(about.heading),
        [row.into_any_element()],
        theme,
    )
}

/// A page's title, and the status line under it with a spinner while `busy`.
pub(crate) fn page_header(
    title: &'static str,
    status: Option<SharedString>,
    busy: bool,
    theme: &Theme,
) -> gpui::Div {
    let status = div()
        .id("status")
        .role(Role::Status)
        .aria_label(status.clone().unwrap_or_default())
        .min_h(text::LABEL_SM.line_height)
        .flex()
        .items_center()
        .gap(BASE_06)
        .type_style(text::LABEL_SM)
        .text_color(theme.text_muted)
        .when(busy, |status| status.child(spinner(theme)))
        .children(status);
    div()
        .flex_none()
        .pt(BASE_20)
        .px(BASE_24)
        .pb(BASE_16)
        .flex()
        .flex_col()
        .gap(BASE_02)
        .child(
            div()
                .id("page-title")
                .role(Role::Heading)
                .aria_label(title)
                .type_style(text::HEADLINE_LG)
                .child(title),
        )
        .child(status)
}

#[cfg(test)]
mod tests {
    use gpui::{KeyContext, Keymap, Keystroke};

    use super::*;

    fn binds(key: &str, contexts: &[&str], action: &dyn gpui::Action) -> bool {
        let keymap = Keymap::new(bindings().into());
        let stack: Vec<_> = contexts
            .iter()
            .map(|context| KeyContext::parse(context).unwrap())
            .collect();
        let (bound, _) = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &stack);
        bound
            .first()
            .is_some_and(|binding| binding.action().partial_eq(action))
    }

    #[test]
    fn enter_commits_and_arrows_step_a_number_field() {
        let number = [KEY_CONTEXT, NUMBER_FIELD_CONTEXT];
        assert!(binds("enter", &number, &Commit));
        assert!(binds("up", &number, &StepUp));
        assert!(binds("down", &number, &StepDown));
    }

    #[test]
    fn enter_commits_a_text_field_the_arrows_do_not_step() {
        let text = [KEY_CONTEXT, FIELD_CONTEXT];
        assert!(binds("enter", &text, &Commit));
        assert!(!binds("up", &text, &StepUp));
    }
}
