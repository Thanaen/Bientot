//! The « Bienvenue dans WorkTime » window, shown before anything else on a first run. Its state
//! lives in [`Onboarding`]; this module draws it in the frame of the usage-signal mockup, turns
//! keys and clicks into its methods and saves the end off the window's thread.

use std::sync::Arc;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable as _, Global, KeyBinding,
    KeyDownEvent, Pixels, Role, ScrollHandle, Subscription, Toggled, Window, WindowBounds,
    WindowHandle, WindowOptions, actions, div, prelude::*, px, size, transparent_black,
};
use worktime_domain::configuration::WorkTimeSettings;
use worktime_store::settings::SaveSettingsError;
use worktime_windows::startup::StartupRegistrationState;

use crate::adjust::TimePart;
use crate::onboarding::{self, Body, LanguageOption, Onboarding, PageView, Step};
use crate::settings::startup::StartupPort;
use crate::settings::{Clock, Control, Field, LanguageChoice, RowView, Switch, UsagePingFacts};
use crate::settings_view::{
    self, Startup, TimeKey, outlined, ping_details, row_frame, switch_control, time_field, time_key,
};
use crate::strings::{Key, Language};
use crate::text_input::TextInput;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, radius, size as token, text};
use crate::week_view::tinted;
use crate::widgets::{self, ClockFocus, Notice};

actions!(onboarding, [Commit, FocusNext, FocusPrevious]);

const KEY_CONTEXT: &str = "Onboarding";
/// The daily target, which Enter commits.
const FIELD_CONTEXT: &str = "OnboardingField";

/// The frame of the usage-signal mockup.
pub const WINDOW_WIDTH: Pixels = px(640.);
pub const WINDOW_HEIGHT: Pixels = px(560.);
const TARGET_WIDTH: Pixels = px(104.);
/// The settings group among the children of the scrolled page, after the heading.
const BODY_ITEM: usize = 1;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys(bindings());
}

fn bindings() -> [KeyBinding; 3] {
    [
        KeyBinding::new("enter", Commit, Some(FIELD_CONTEXT)),
        KeyBinding::new("tab", FocusNext, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(KEY_CONTEXT)),
    ]
}

/// Writes the settings of « Commencer ». It runs off the window's thread.
pub type SaveSettings =
    Arc<dyn Fn(&WorkTimeSettings) -> Result<(), SaveSettingsError> + Send + Sync>;

/// Starts WorkTime on the settings saved.
pub type OnComplete = Box<dyn FnOnce(WorkTimeSettings, &mut App)>;

#[derive(Default)]
struct OnboardingWindow(Option<WindowHandle<OnboardingView>>);

impl Global for OnboardingWindow {}

/// Opens the onboarding on `settings`, the texts in `forced` if the launch fixed it. Closing it
/// before « Commencer » saves nothing and quits, so that it shows again at the next start; once
/// saved, `on_complete` starts WorkTime and it closes.
pub fn open(
    settings: WorkTimeSettings,
    forced: Option<Language>,
    save: SaveSettings,
    on_complete: OnComplete,
    cx: &mut App,
) {
    // A first run has sent no signal yet.
    let usage_ping = UsagePingFacts {
        build: settings_view::build(),
        released: settings_view::released(),
        today: jiff::Zoned::now().date(),
        last_sent: None,
    };
    let onboarding = Onboarding::new(settings, usage_ping, forced);
    cx.set_global(onboarding.language());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(WINDOW_WIDTH, WINDOW_HEIGHT),
            cx,
        ))),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(
                onboarding
                    .language()
                    .text(Key::OnboardingWindowTitle)
                    .into(),
            ),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        focus: true,
        show: true,
        is_resizable: false,
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        cx.new(|cx| OnboardingView::new(onboarding, save, on_complete, window, cx))
    }) {
        Ok(window) => cx.default_global::<OnboardingWindow>().0 = Some(window),
        Err(error) => {
            log::error!("cannot open the onboarding: {error:#}");
            cx.quit();
        }
    }
}

/// Brings the open onboarding to the front, as a later launch does; whether one is open.
pub fn activate(cx: &mut App) -> bool {
    cx.default_global::<OnboardingWindow>()
        .0
        .is_some_and(|window| {
            window
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        })
}

pub struct OnboardingView {
    onboarding: Onboarding,
    /// The language the window was last drawn in.
    language: Language,
    focus_handle: FocusHandle,
    target: Entity<TextInput>,
    clocks: [ClockFocus; 3],
    /// The lunch break and the departure: leaving either checks the schedule.
    lunch: FocusHandle,
    departure: FocusHandle,
    focused_clock: Option<Clock>,
    content_scroll: ScrollHandle,
    startup: Option<Arc<dyn StartupPort>>,
    save: SaveSettings,
    on_complete: Option<OnComplete>,
    _subscriptions: Vec<Subscription>,
}

impl OnboardingView {
    pub fn new(
        mut onboarding: Onboarding,
        save: SaveSettings,
        on_complete: OnComplete,
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
                        view.onboarding.show_startup(state);
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            None => onboarding.show_startup(StartupRegistrationState::Unavailable),
        }
        let language = onboarding.language();
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let target = cx.new(|cx| {
            let mut input = TextInput::new(Field::Target.name(language), "", None, cx);
            input.set_text(onboarding.target(), cx);
            input
        });
        let mut subscriptions =
            vec![cx.on_blur(&target.focus_handle(cx), window, |view, _, cx| {
                view.commit_target(cx)
            })];
        let clocks = [
            ClockFocus::new(cx),
            ClockFocus::new(cx),
            ClockFocus::new(cx),
        ];
        // A digit typed after the focus came back starts the part over.
        for clock in [Clock::LunchStart, Clock::LunchEnd, Clock::Departure] {
            let focus = &clocks[clock as usize];
            for handle in [&focus.hours, &focus.minutes] {
                subscriptions.push(cx.on_blur(handle, window, move |view, _, _| {
                    view.onboarding.time_mut(clock).leave();
                }));
            }
        }
        // Groups around the time fields, whose parts are the tab stops.
        let (lunch, departure) = (cx.focus_handle(), cx.focus_handle());
        for handle in [&lunch, &departure] {
            subscriptions.push(cx.on_focus_out(handle, window, |view, _, _, cx| {
                view.onboarding.commit_schedule();
                cx.notify();
            }));
        }
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            view.update(cx, |view, cx| view.should_close(cx))
                .unwrap_or(true)
        });
        Self {
            onboarding,
            language,
            focus_handle,
            target,
            clocks,
            lunch,
            departure,
            focused_clock: None,
            content_scroll: ScrollHandle::new(),
            startup,
            save,
            on_complete: Some(on_complete),
            _subscriptions: subscriptions,
        }
    }

    /// The model, for the screenshots.
    pub fn onboarding_mut(&mut self) -> &mut Onboarding {
        &mut self.onboarding
    }

    /// Types `text` as the daily target, as the user would, for the screenshots.
    pub fn type_target(&mut self, text: &str, cx: &mut Context<Self>) {
        self.target.update(cx, |input, cx| input.set_text(text, cx));
        self.onboarding.set_target(text);
    }

    /// Opens or closes « Voir ce qui est envoyé ». Opening it scrolls the page only as far as
    /// the end of the settings group, so that the details show under as much of the heading as
    /// fits.
    pub fn toggle_details(&mut self) {
        if self.onboarding.toggle_details() {
            self.content_scroll.scroll_to_item(BODY_ITEM);
        }
    }

    /// Takes the typed target into the schedule, and shows it as the schedule reads it.
    fn commit_target(&mut self, cx: &mut Context<Self>) {
        if self.onboarding.step() != Step::Schedule {
            return;
        }
        let text = self.target.read(cx).text().to_owned();
        self.onboarding.set_target(&text);
        self.onboarding.commit_schedule();
        self.show_target(cx);
    }

    fn show_target(&mut self, cx: &mut Context<Self>) {
        let draft = self.onboarding.target().to_owned();
        self.target.update(cx, |input, cx| {
            if input.text() != draft {
                input.set_text(&draft, cx);
            }
        });
        cx.notify();
    }

    fn previous(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_target(cx);
        self.onboarding.back();
        self.stepped(window, cx);
    }

    /// « Suivant », or « Commencer », which asks Windows and saves off the window's thread.
    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_target(cx);
        let Some(settings) = self.onboarding.advance() else {
            self.stepped(window, cx);
            return;
        };
        cx.notify();
        let (port, save) = (self.startup.clone(), self.save.clone());
        cx.spawn_in(window, async move |view, cx| {
            let saved = cx
                .background_executor()
                .spawn(async move {
                    onboarding::complete(settings, port.as_deref(), |settings| save(settings))
                })
                .await;
            view.update_in(cx, |view, window, cx| view.saved(saved, window, cx))
                .ok();
        })
        .detach();
    }

    fn saved(
        &mut self,
        saved: Result<WorkTimeSettings, SaveSettingsError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.onboarding.finished(saved.is_ok());
        match saved {
            Ok(settings) => {
                if let Some(on_complete) = self.on_complete.take() {
                    on_complete(settings, cx);
                }
                cx.default_global::<OnboardingWindow>().0 = None;
                window.remove_window();
            }
            Err(error) => {
                log::error!("cannot save the onboarding: {error}");
                cx.notify();
            }
        }
    }

    /// Focuses the daily target when the schedule shows, as .NET does.
    fn stepped(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_target(cx);
        self.content_scroll.scroll_to_top_of_item(0);
        if self.onboarding.step() == Step::Schedule {
            let handle = self.target.focus_handle(cx);
            window.focus(&handle, cx);
        } else {
            window.focus(&self.focus_handle, cx);
        }
    }

    fn choose_language(&mut self, choice: LanguageChoice, cx: &mut Context<Self>) {
        self.onboarding.choose_language(choice);
        cx.notify();
    }

    fn toggle(&mut self, switch: Switch, _: &mut Window, cx: &mut Context<Self>) {
        self.onboarding.toggle(switch);
        cx.notify();
    }

    fn time_key(
        &mut self,
        clock: Clock,
        part: TimePart,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match time_key(self.onboarding.time_mut(clock), part, event) {
            TimeKey::Ignored => return,
            TimeKey::Commit => {
                self.onboarding.commit_schedule();
            }
            TimeKey::Changed { next } => {
                if next {
                    let minutes = self.clocks[clock as usize].minutes.clone();
                    window.focus(&minutes, cx);
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// The close button: nothing is saved and WorkTime quits, unless the end is being saved.
    fn should_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.onboarding.is_saving() {
            return false;
        }
        cx.default_global::<OnboardingWindow>().0 = None;
        cx.quit();
        true
    }

    fn cycle_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if forward {
            window.focus_next(cx);
        } else {
            window.focus_prev(cx);
        }
    }
}

impl OnboardingView {
    fn title_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        widgets::title_bar(
            self.language.text(Key::OnboardingWindowTitle),
            [],
            cx.listener(|view, _, window, cx| {
                if view.should_close(cx) {
                    window.remove_window();
                }
            }),
            theme,
        )
    }

    /// The step's counter, heading and intro.
    fn header(&self, page: &PageView, theme: &Theme) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(BASE_04)
            .child(
                div()
                    .type_style(text::LABEL_XS)
                    .text_color(theme.text_muted)
                    .child(page.counter.to_uppercase()),
            )
            .child(
                div()
                    .id("heading")
                    .role(Role::Heading)
                    .aria_label(page.heading)
                    .type_style(text::HEADLINE_LG)
                    .child(page.heading),
            )
            .child(div().text_color(theme.text_muted).child(page.intro))
    }

    fn languages(
        &self,
        name: &'static str,
        options: Vec<LanguageOption>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let count = options.len();
        let rows = options.into_iter().enumerate().map(|(position, option)| {
            let choice = option.choice;
            let (ring, dot) = if option.selected {
                (theme.accent, theme.accent)
            } else {
                (theme.text_placeholder, transparent_black())
            };
            let (hover, focused) = (theme.element_hover, theme.border_focused);
            let last = position + 1 == count;
            let radio = div()
                .id(("language", position))
                .role(Role::RadioButton)
                .aria_label(option.label)
                .aria_toggled(if option.selected {
                    Toggled::True
                } else {
                    Toggled::False
                })
                .h(px(40.))
                .mx(-BASE_16)
                .px(BASE_16)
                .flex()
                .items_center()
                .gap(px(10.))
                .border_1()
                .border_color(transparent_black())
                .when(position == 0, |row| row.rounded_t(radius::LG))
                .when(last, |row| row.rounded_b(radius::LG))
                .tab_index(0)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .focus_visible(move |style| style.border_color(focused))
                .child(
                    div()
                        .size(token::ICON_SM)
                        .flex_none()
                        .rounded_full()
                        .border_1()
                        .border_color(ring)
                        .bg(theme.editor)
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(option.selected, |radio| {
                            radio.child(div().size(px(6.)).rounded_full().bg(dot))
                        }),
                )
                .child(option.label)
                .on_click(cx.listener(move |view, _, _, cx| view.choose_language(choice, cx)));
            div()
                .when(!last, |row| {
                    row.border_b_1().border_color(theme.border_variant)
                })
                .child(radio)
                .into_any_element()
        });
        settings_view::card(theme)
            .children(rows)
            .id("languages")
            .role(Role::RadioGroup)
            .aria_label(name)
    }

    fn rows(&mut self, rows: Vec<RowView>, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let count = rows.len();
        let rows = rows.into_iter().enumerate().map(|(index, row)| {
            let control = self.control(row.control.clone(), theme, cx);
            let details = row
                .details
                .clone()
                .map(|details| ping_details(details, theme, cx, Self::toggle_details));
            row_frame((&row).into(), control, details, index + 1 == count, theme)
        });
        settings_view::card(theme).children(rows)
    }

    fn control(
        &mut self,
        control: Control,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let time = |clock: Clock, invalid: bool, cx: &mut Context<Self>| {
            time_field(
                clock,
                self.onboarding.time(clock),
                &self.clocks[clock as usize],
                self.focused_clock == Some(clock),
                invalid,
                self.language,
                theme,
                cx,
                Self::time_key,
            )
        };
        match control {
            Control::Text { invalid, .. } => {
                self.target.update(cx, |input, _| input.invalid = invalid);
                div()
                    .id("target")
                    .key_context(FIELD_CONTEXT)
                    .on_action(cx.listener(|view, _: &Commit, _, cx| view.commit_target(cx)))
                    .w(TARGET_WIDTH)
                    .flex_none()
                    .child(self.target.clone())
                    .into_any_element()
            }
            Control::Range {
                start,
                end,
                invalid,
            } => {
                let start = time(start, invalid, cx);
                let end = time(end, invalid, cx);
                div()
                    .id("lunch")
                    .role(Role::Group)
                    .track_focus(&self.lunch)
                    .flex()
                    .items_center()
                    .gap(BASE_08)
                    .child(start)
                    .child(div().text_color(theme.text_muted).child("–"))
                    .child(end)
                    .into_any_element()
            }
            Control::Time { clock, invalid } => {
                let field = time(clock, invalid, cx);
                div()
                    .id("departure")
                    .track_focus(&self.departure)
                    .child(field)
                    .into_any_element()
            }
            Control::Switch {
                switch,
                name,
                on,
                enabled,
            } => switch_control(switch, name, on, enabled, theme, cx, Self::toggle)
                .into_any_element(),
            control => unreachable!("the onboarding shows no {control:?}"),
        }
    }

    fn failure(message: &'static str, theme: &Theme) -> impl IntoElement {
        widgets::notice_box("save-failed", &Notice::error(message), theme)
            .child(div().flex_1().min_w_0().child(message))
    }

    /// The step dots, « Précédent » and « Suivant » or « Commencer ».
    fn footer(&self, page: &PageView, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let dots = Step::ALL.map(|step| {
            div().size(px(6.)).rounded_full().bg(if step == page.step {
                theme.accent
            } else {
                theme.text_placeholder
            })
        });
        let previous = page.previous.map(|label| {
            let button = outlined("previous", label, token::BUTTON_LARGE, theme);
            if page.saving {
                return button.text_color(theme.text_placeholder);
            }
            button.on_click(cx.listener(|view, _, window, cx| view.previous(window, cx)))
        });
        let next = tinted("next", page.next.into(), theme)
            .h(token::BUTTON_LARGE)
            .px(BASE_12);
        let next = if page.saving {
            next.child(settings_view::spinner(theme)).child(page.next)
        } else {
            next.child(page.next)
                .on_click(cx.listener(|view, _, window, cx| view.next(window, cx)))
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(BASE_08)
            .pt(BASE_12)
            .pb(BASE_16)
            .px(BASE_32)
            .border_t_1()
            .border_color(theme.border_variant)
            .child(div().flex().items_center().gap(BASE_06).children(dots))
            .child(div().flex_1())
            .children(previous)
            .child(next)
    }
}

impl Render for OnboardingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let language = self.onboarding.language();
        if language != self.language {
            self.language = language;
            window.set_window_title(language.text(Key::OnboardingWindowTitle));
            self.target
                .update(cx, |input, _| input.set_name(Field::Target.name(language)));
        }
        self.focused_clock = [Clock::LunchStart, Clock::LunchEnd, Clock::Departure]
            .into_iter()
            .find(|&clock| self.clocks[clock as usize].is_focused(window));
        let page = self.onboarding.view();
        let body = match page.body.clone() {
            Body::Language { name, options } => {
                self.languages(name, options, &theme, cx).into_any_element()
            }
            Body::Rows { rows, .. } => self.rows(rows, &theme, cx).into_any_element(),
        };
        let footnote = match &page.body {
            Body::Rows {
                footnote: Some(footnote),
                ..
            } => Some(widgets::footnote(footnote, &theme)),
            _ => None,
        };
        let content = div()
            .id("content")
            .track_scroll(&self.content_scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py(BASE_24)
            .px(BASE_32)
            .flex()
            .flex_col()
            .gap(BASE_20)
            .child(self.header(&page, &theme))
            .child(body)
            .children(footnote)
            .children(page.failure.map(|message| Self::failure(message, &theme)));
        let scrolled = div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(content)
            .child(widgets::scrolled_edge(self.content_scroll.clone(), &theme));
        div()
            .id("onboarding")
            .role(Role::Window)
            .aria_label(language.text(Key::OnboardingWindowTitle))
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(
                cx.listener(|view, _: &FocusNext, window, cx| view.cycle_focus(true, window, cx)),
            )
            .on_action(cx.listener(|view, _: &FocusPrevious, window, cx| {
                view.cycle_focus(false, window, cx)
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.editor)
            .text_color(theme.text)
            .type_style(text::LABEL_MD)
            .child(self.title_bar(&theme, cx))
            .child(scrolled)
            .child(self.footer(&page, &theme, cx))
    }
}

#[cfg(test)]
mod tests {
    use gpui::{KeyContext, Keymap, Keystroke};

    use super::*;

    fn bound(key: &str, contexts: &[&str]) -> Vec<KeyBinding> {
        let keymap = Keymap::new(bindings().into());
        let stack: Vec<_> = contexts
            .iter()
            .map(|context| KeyContext::parse(context).unwrap())
            .collect();
        keymap
            .bindings_for_input(&[Keystroke::parse(key).unwrap()], &stack)
            .0
            .into_vec()
    }

    #[test]
    fn enter_commits_the_daily_target_and_tab_moves_through_the_window() {
        let field = bound("enter", &[KEY_CONTEXT, FIELD_CONTEXT]);
        assert!(field[0].action().partial_eq(&Commit));
        assert!(bound("enter", &[KEY_CONTEXT]).is_empty());
        let tab = bound("shift-tab", &[KEY_CONTEXT, FIELD_CONTEXT]);
        assert!(tab[0].action().partial_eq(&FocusPrevious));
    }
}
