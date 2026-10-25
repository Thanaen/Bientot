//! The « Bienvenue dans WorkTime » window, shown before anything else on a first run. Its state
//! lives in [`Onboarding`]; this module draws it in the frame of the usage-signal mockup, turns
//! keys and clicks into its methods, and saves the end, or stages an import, off the window's
//! thread.

use std::sync::Arc;

use bientot_domain::configuration::Settings;
use bientot_store::data_directory::DataDirectory;
use bientot_store::settings::SaveSettingsError;
use bientot_windows::startup::StartupRegistrationState;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable as _, Global, KeyBinding,
    KeyDownEvent, Pixels, Role, ScrollHandle, Subscription, Toggled, Window, WindowBounds,
    WindowHandle, WindowOptions, actions, div, prelude::*, px, size, transparent_black,
};

use crate::adjust::TimePart;
use crate::onboarding::{self, Body, ImportOffer, LanguageOption, Onboarding, PageView, Step};
use crate::settings::data::{Action, Chosen, DataPage, ImportDialog, Refusal};
use crate::settings::startup::StartupPort;
use crate::settings::{Clock, Control, Field, LanguageChoice, RowView, Switch, UsagePingFacts};
use crate::settings_view::{
    self, Startup, TimeKey, field_error, filled, outlined, ping_details, row_frame, switch_control,
    time_field, time_key, warning,
};
use crate::strings::{Key, Language};
use crate::text_input::TextInput;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, radius, size as token, text};
use crate::transfer;
use crate::week_view::tinted;
use crate::widgets::{self, ClockFocus, Notice};

actions!(onboarding, [Commit, Escape, FocusNext, FocusPrevious]);

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

fn bindings() -> [KeyBinding; 4] {
    [
        KeyBinding::new("enter", Commit, Some(FIELD_CONTEXT)),
        KeyBinding::new("escape", Escape, Some(KEY_CONTEXT)),
        KeyBinding::new("tab", FocusNext, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(KEY_CONTEXT)),
    ]
}

/// Writes the settings of « Commencer ». It runs off the window's thread.
pub type SaveSettings = Arc<dyn Fn(&Settings) -> Result<(), SaveSettingsError> + Send + Sync>;

/// Starts WorkTime on the settings saved.
pub type OnComplete = Box<dyn FnOnce(Settings, &mut App)>;

#[derive(Default)]
struct OnboardingWindow(Option<WindowHandle<OnboardingView>>);

impl Global for OnboardingWindow {}

/// Opens the onboarding on `settings`, the texts in `forced` if the launch fixed it. Closing it
/// before « Commencer » saves nothing and quits, so that it shows again at the next start; once
/// saved, `on_complete` starts WorkTime and it closes. Its first step offers to import into
/// `directory`, and an import restarts WorkTime on it instead.
pub fn open(
    settings: Settings,
    forced: Option<Language>,
    directory: Option<DataDirectory>,
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
    let onboarding = Onboarding::new(settings, usage_ping, forced, directory);
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
    /// « Importer des données… », and « Annuler » of its confirmation.
    import_focus: FocusHandle,
    cancel_focus: FocusHandle,
    /// The import changed what shows: the next frame focuses « Annuler », or the action again.
    refocus_import: bool,
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
            import_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            refocus_import: false,
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
        saved: Result<Settings, SaveSettingsError>,
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

    /// « Importer des données… »: the same file dialog as « Paramètres › Données », then the
    /// confirmation in the page.
    fn choose_import(&mut self, cx: &mut Context<Self>) {
        let Some(directory) = self
            .onboarding
            .import_mut()
            .and_then(|page| page.start(Action::Import))
        else {
            return;
        };
        let view = cx.entity().downgrade();
        transfer::choose_import(
            directory,
            move |read, cx| {
                view.update(cx, |view, cx| {
                    view.read_import(read);
                    cx.notify();
                })
                .ok();
            },
            cx,
        );
        cx.notify();
    }

    /// Shows what the file dialog and the file gave, `None` once the dialog was cancelled: the
    /// confirmation with « Annuler » focused, or the action again. Public for the screenshots.
    pub fn read_import(&mut self, read: Option<Result<Chosen, Refusal>>) {
        if let Some(page) = self.onboarding.import_mut() {
            page.read(read);
        }
        self.refocus_import = true;
    }

    /// « Importer et redémarrer »: WorkTime restarts on the imported data, which skips the
    /// onboarding.
    fn confirm_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((file, directory)) = self.onboarding.import_mut().and_then(DataPage::confirm)
        else {
            return;
        };
        window.focus(&self.import_focus, cx);
        cx.notify();
        let view = cx.entity().downgrade();
        transfer::import(
            directory,
            file,
            move |cx| {
                view.update(cx, |view, cx| {
                    if let Some(page) = view.onboarding.import_mut() {
                        page.import_failed();
                    }
                    cx.notify();
                })
                .ok();
            },
            cx,
        );
    }

    /// « Annuler », or Escape: back to the languages, the action focused.
    fn cancel_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.onboarding.import_mut() {
            page.cancel();
        }
        window.focus(&self.import_focus, cx);
        cx.notify();
    }

    fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        let confirming = self
            .onboarding
            .import_mut()
            .is_some_and(|page| page.dialog().is_some());
        if confirming {
            self.cancel_import(window, cx);
        }
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

    /// The choices, dimmed and out of the tab order while an import is `locked` in.
    fn languages(
        &self,
        name: &'static str,
        options: Vec<LanguageOption>,
        locked: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let count = options.len();
        let rows = options.into_iter().enumerate().map(|(position, option)| {
            let choice = option.choice;
            let (ring, dot) = match (option.selected, locked) {
                (true, false) => (theme.accent, theme.accent),
                (true, true) => (theme.text_placeholder, theme.text_placeholder),
                (false, _) => (theme.text_placeholder, transparent_black()),
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
                .focus_visible(move |style| style.border_color(focused))
                .map(|radio| {
                    if locked {
                        radio
                            // GPUI has no setter for this state, only this hook on the node.
                            .a11y_synthetic_children(|children| {
                                children.parent_node().set_disabled()
                            })
                            .tab_stop(false)
                            .text_color(theme.text_placeholder)
                            .on_any_mouse_down(|_, window, _| window.prevent_default())
                    } else {
                        radio
                            .cursor_pointer()
                            .hover(move |style| style.bg(hover))
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.choose_language(choice, cx)),
                            )
                    }
                })
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
                .child(option.label);
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

    /// Under the languages: « Importer des données… » after its lead, or the confirmation of the
    /// file read, in the page since the onboarding has no dialog.
    fn import(
        &self,
        offer: ImportOffer,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if let Some(confirmation) = offer.confirmation {
            return self
                .import_confirmation(confirmation, theme, cx)
                .into_any_element();
        }
        // Compact, so that « Suivant » stays the step forward.
        let button = if offer.status.is_some() {
            widgets::disabled_control("import-data", offer.action.into(), theme)
                .h(token::BUTTON_DEFAULT)
                .px(BASE_08)
                .flex()
                .flex_none()
                .items_center()
                .type_style(text::LABEL_SM)
                .border_color(theme.border_variant)
                .bg(theme.element)
                .whitespace_nowrap()
                .child(offer.action)
        } else {
            outlined("import-data", offer.action, token::BUTTON_DEFAULT, theme)
                .px(BASE_08)
                .type_style(text::LABEL_SM)
                .on_click(cx.listener(|view, _, _, cx| view.choose_import(cx)))
        };
        let row = div()
            .flex()
            .items_start()
            .gap(BASE_16)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .type_style(text::LABEL_SM)
                    .text_color(theme.text_muted)
                    .child(offer.lead),
            )
            // Centred on the two lines of the lead.
            .child(
                div()
                    .h(text::LABEL_SM.line_height * 2.)
                    .flex()
                    .items_center()
                    .child(button.track_focus(&self.import_focus)),
            );
        // The status and the error tell about the whole action: they take the full width.
        div()
            .flex()
            .flex_col()
            .type_style(text::LABEL_SM)
            .text_color(theme.text_muted)
            .child(row)
            .children(offer.status.map(|status| {
                div()
                    .id("import-status")
                    .role(Role::Status)
                    .aria_label(status)
                    .mt(BASE_08)
                    .flex()
                    .items_center()
                    .gap(BASE_06)
                    .child(settings_view::spinner(theme))
                    .child(status)
            }))
            .children(
                offer
                    .error
                    .map(|error| div().pt(BASE_04).child(field_error(error, theme))),
            )
            .into_any_element()
    }

    /// What the file holds and what this PC would lose, « Annuler » focused as it shows.
    fn import_confirmation(
        &self,
        dialog: ImportDialog,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focused = theme.border_focused;
        let mut description = dialog.paragraphs.clone();
        description.extend(dialog.loss.clone());
        description.push(dialog.message.to_owned());
        // Apart from the languages above. Over the title, the room its line height leaves above
        // its capitals comes off, so that they sit as far from the edge as the buttons do.
        settings_view::card(theme)
            .id("import")
            .role(Role::Group)
            .aria_label(dialog.title)
            .aria_description(description.join(" "))
            .border_color(theme.accent_border)
            .pt(BASE_16 - BASE_06)
            .pb(BASE_16)
            .flex()
            .flex_col()
            .gap(BASE_08)
            .child(
                div()
                    .id("import-title")
                    .role(Role::Heading)
                    .type_style(text::HEADLINE_SM)
                    .child(dialog.title),
            )
            .children(
                dialog
                    .paragraphs
                    .into_iter()
                    .map(|paragraph| div().child(paragraph)),
            )
            .children(dialog.loss.map(|loss| warning(loss.into(), theme)))
            .child(dialog.message)
            .child(
                div()
                    .pt(BASE_08)
                    .flex()
                    .justify_end()
                    .gap(BASE_08)
                    .child(
                        outlined("cancel-import", dialog.cancel, token::BUTTON_LARGE, theme)
                            .focus(move |style| style.border_color(focused))
                            .track_focus(&self.cancel_focus)
                            .on_click(
                                cx.listener(|view, _, window, cx| view.cancel_import(window, cx)),
                            ),
                    )
                    .child(
                        filled("confirm-import", dialog.confirm, token::BUTTON_LARGE, theme)
                            .focus(move |style| style.border_color(focused))
                            .on_click(
                                cx.listener(|view, _, window, cx| view.confirm_import(window, cx)),
                            ),
                    ),
            )
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
        let next = if page.importing {
            widgets::disabled_control("next", page.next.into(), theme)
                .h(token::BUTTON_LARGE)
                .px(BASE_12)
                .flex()
                .items_center()
                .border_color(theme.border_variant)
                .bg(theme.element)
                .child(page.next)
        } else if page.saving {
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
            // The file dialog of an import names its file type in it.
            cx.set_global(language);
            window.set_window_title(language.text(Key::OnboardingWindowTitle));
            self.target
                .update(cx, |input, _| input.set_name(Field::Target.name(language)));
        }
        self.focused_clock = [Clock::LunchStart, Clock::LunchEnd, Clock::Departure]
            .into_iter()
            .find(|&clock| self.clocks[clock as usize].is_focused(window));
        let page = self.onboarding.view();
        let (body, import) = match page.body.clone() {
            Body::Language {
                name,
                options,
                import,
            } => {
                if std::mem::take(&mut self.refocus_import) {
                    let confirming = import
                        .as_ref()
                        .is_some_and(|offer| offer.confirmation.is_some());
                    let handle = if confirming {
                        &self.cancel_focus
                    } else {
                        &self.import_focus
                    };
                    window.focus(handle, cx);
                }
                (
                    self.languages(name, options, page.importing, &theme, cx)
                        .into_any_element(),
                    import.map(|offer| self.import(offer, &theme, cx)),
                )
            }
            Body::Rows { rows, .. } => (self.rows(rows, &theme, cx).into_any_element(), None),
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
            .children(import)
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
            .on_action(cx.listener(Self::escape))
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

    #[test]
    fn escape_reaches_the_window_from_anywhere_in_it() {
        let escape = bound("escape", &[KEY_CONTEXT, FIELD_CONTEXT]);
        assert!(escape[0].action().partial_eq(&Escape));
    }
}
