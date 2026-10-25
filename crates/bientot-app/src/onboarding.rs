//! The first-run « Bienvenue dans WorkTime » window as a model: its steps, their drafts, what
//! each page shows and what the end saves. Ported from `Services/Onboarding/OnboardingState.cs`,
//! `OnboardingCompletion.cs` and `Presentation/Onboarding/OnboardingPresentation.cs`, without
//! the activity source step: v1 reads the built-in detection only. The first step also offers
//! the import of « Paramètres › Données », for a PC that takes over from another one. Nothing
//! here knows GPUI.

use bientot_core::settings::EditError;
use bientot_core::settings::number::{self, format_target_duration};
use bientot_domain::configuration::{Settings, SettingsError};
use bientot_domain::sources::ActivitySourceKind;
use bientot_store::data_directory::DataDirectory;
use bientot_store::settings::SaveSettingsError;
use bientot_windows::startup::StartupRegistrationState;
use jiff::SignedDuration;
use jiff::civil::Time;

use crate::adjust::TimeField;
use crate::settings::data::{DataPage, ImportDialog, Row};
use crate::settings::startup::{self, StartupPort};
use crate::settings::{
    Clock, Control, Details, Field, LanguageChoice, PingDetails, RowView, Setting, Switch,
    UsagePingFacts, error_text, ping_body,
};
use crate::strings::{Key, Language};

/// The argument that shows the onboarding on settings already saved, as in .NET.
pub const ARGUMENT: &str = "--onboarding";

/// Whether a launch opens the onboarding before anything else, as `OnboardingLaunch.ShouldShow`
/// decides. A demo starts without saved settings, so it shows the onboarding only when asked.
pub fn should_show(settings_saved: bool, demo: bool, requested: bool) -> bool {
    requested || (!settings_saved && !demo)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Language,
    Schedule,
    Connections,
}

impl Step {
    pub const ALL: [Step; 3] = [Step::Language, Step::Schedule, Step::Connections];

    fn previous(self) -> Option<Step> {
        match self {
            Self::Language => None,
            Self::Schedule => Some(Self::Language),
            Self::Connections => Some(Self::Schedule),
        }
    }

    fn next(self) -> Option<Step> {
        match self {
            Self::Language => Some(Self::Schedule),
            Self::Schedule => Some(Self::Connections),
            Self::Connections => None,
        }
    }

    /// From 1.
    fn number(self) -> usize {
        self as usize + 1
    }
}

/// Where the end of the onboarding stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ending {
    Editing,
    Saving,
    Failed,
}

/// What the page of the current step shows.
#[derive(Clone, Debug, PartialEq)]
pub struct PageView {
    /// « Étape 2 sur 3 ».
    pub counter: String,
    pub step: Step,
    pub heading: &'static str,
    pub intro: &'static str,
    pub body: Body,
    /// « Précédent », from the second step on.
    pub previous: Option<&'static str>,
    /// « Suivant », or « Commencer » on the last step.
    pub next: &'static str,
    /// The end is being saved: nothing can be changed.
    pub saving: bool,
    /// The end could not be saved; « Commencer » tries again.
    pub failure: Option<&'static str>,
    /// An import is being chosen, confirmed or staged: « Suivant » waits for it.
    pub importing: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Language {
        /// The name of the group of choices.
        name: &'static str,
        options: Vec<LanguageOption>,
        /// `None` in a demo, which has no data of its own to replace.
        import: Option<ImportOffer>,
    },
    Rows {
        rows: Vec<RowView>,
        /// The line under the group.
        footnote: Option<&'static str>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageOption {
    pub choice: LanguageChoice,
    pub label: &'static str,
    pub selected: bool,
}

/// « Importer des données… » under the languages, with what the import of « Paramètres ›
/// Données » tells along the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportOffer {
    pub lead: &'static str,
    pub action: &'static str,
    /// What runs, the file being read or the restart, while the action waits.
    pub status: Option<&'static str>,
    /// Why the file was refused, or why the import failed.
    pub error: Option<&'static str>,
    /// The file read, to import once confirmed, in place of the action.
    pub confirmation: Option<ImportDialog>,
}

pub struct Onboarding {
    step: Step,
    /// The choices so far.
    settings: Settings,
    /// `--french` or `--english`: the texts keep that language whatever is chosen.
    forced: Option<Language>,
    /// The minimum target the settings had: the schedule does not ask for it, so it stays
    /// unless the daily target becomes shorter.
    preferred_minimum: SignedDuration,
    target: String,
    lunch_start: TimeField,
    lunch_end: TimeField,
    departure: TimeField,
    /// Why the schedule was refused when last committed.
    schedule_error: Option<EditError>,
    details_open: bool,
    usage_ping: UsagePingFacts,
    /// `None` until the registration has been read.
    startup: Option<StartupRegistrationState>,
    ending: Ending,
    /// The import of the first step, on the data folder this PC would use.
    import: Option<DataPage>,
}

fn time_field(time: Time) -> TimeField {
    TimeField::new(time.hour(), time.minute())
}

impl Onboarding {
    /// Starts on `settings`, the defaults on a first run. A new installation reads the built-in
    /// detection (FR-044). The first step offers to import into `directory`, unless it is `None`.
    pub fn new(
        mut settings: Settings,
        usage_ping: UsagePingFacts,
        forced: Option<Language>,
        directory: Option<DataDirectory>,
    ) -> Self {
        settings
            .activity_source
            .get_or_insert(ActivitySourceKind::BuiltIn);
        let language = Language::resolve(forced, settings.language_override.as_deref());
        Self {
            step: Step::Language,
            preferred_minimum: settings.minimum_effective_target,
            target: format_target_duration(settings.nominal_daily_target),
            lunch_start: time_field(settings.lunch_start),
            lunch_end: time_field(settings.lunch_end),
            departure: time_field(settings.preferred_latest_departure),
            settings,
            forced,
            schedule_error: None,
            details_open: false,
            usage_ping,
            startup: None,
            ending: Ending::Editing,
            import: directory.map(|directory| DataPage::new(language, Some(directory), None)),
        }
    }

    pub fn step(&self) -> Step {
        self.step
    }

    /// The language of the texts: the one chosen, as soon as it is chosen.
    pub fn language(&self) -> Language {
        Language::resolve(self.forced, self.settings.language_override.as_deref())
    }

    pub fn is_saving(&self) -> bool {
        self.ending == Ending::Saving
    }

    /// An import is being chosen, confirmed or staged: the steps wait for it.
    fn is_importing(&self) -> bool {
        self.import.as_ref().is_some_and(|page| !page.ready())
    }

    /// The import, while the first step shows and nothing is being saved.
    pub fn import_mut(&mut self) -> Option<&mut DataPage> {
        if self.step != Step::Language || self.is_saving() {
            return None;
        }
        self.import.as_mut()
    }

    /// The registration as read.
    pub fn show_startup(&mut self, state: StartupRegistrationState) {
        self.startup = Some(state);
    }

    /// Ignored while an import is chosen, confirmed or staged, as « Suivant » is.
    pub fn choose_language(&mut self, choice: LanguageChoice) {
        if self.step == Step::Language && !self.is_saving() && !self.is_importing() {
            self.settings.language_override = choice.language_override();
            let language = self.language();
            if let Some(page) = &mut self.import {
                page.set_language(language);
            }
        }
    }

    /// The daily target as typed.
    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn set_target(&mut self, text: &str) {
        text.clone_into(&mut self.target);
    }

    pub fn time(&self, clock: Clock) -> TimeField {
        match clock {
            Clock::LunchStart => self.lunch_start,
            Clock::LunchEnd => self.lunch_end,
            Clock::Departure => self.departure,
            Clock::AppointmentStart | Clock::AppointmentEnd => {
                unreachable!("the onboarding asks for no appointment")
            }
        }
    }

    pub fn time_mut(&mut self, clock: Clock) -> &mut TimeField {
        match clock {
            Clock::LunchStart => &mut self.lunch_start,
            Clock::LunchEnd => &mut self.lunch_end,
            Clock::Departure => &mut self.departure,
            Clock::AppointmentStart | Clock::AppointmentEnd => {
                unreachable!("the onboarding asks for no appointment")
            }
        }
    }

    /// Takes the schedule's drafts into the choices, or says why it cannot: when a field is
    /// left, and before the step changes. Returns whether the schedule is valid.
    pub fn commit_schedule(&mut self) -> bool {
        if self.step != Step::Schedule {
            return true;
        }
        self.schedule_error = match number::parse_target_duration(&self.target) {
            Err(error) => Some(EditError::Input(error)),
            Ok(target) => {
                self.target = format_target_duration(target);
                let settings = &mut self.settings;
                settings.nominal_daily_target = target;
                settings.minimum_effective_target = self.preferred_minimum.min(target);
                settings.lunch_start = self.lunch_start.time();
                settings.lunch_end = self.lunch_end.time();
                settings.preferred_latest_departure = self.departure.time();
                settings
                    .validate()
                    .into_iter()
                    .find(|error| {
                        matches!(
                            error,
                            SettingsError::Target
                                | SettingsError::Minimum
                                | SettingsError::Lunch
                                | SettingsError::Departure
                        )
                    })
                    .map(EditError::Settings)
            }
        };
        self.schedule_error.is_none()
    }

    /// « Précédent »: the schedule keeps its drafts, valid or not.
    pub fn back(&mut self) {
        if self.is_saving() || self.is_importing() {
            return;
        }
        self.commit_schedule();
        if let Some(previous) = self.step.previous() {
            self.step = previous;
        }
    }

    /// « Suivant »: the next step once this one is valid. On the last step, « Commencer » returns
    /// the choices to save through [`complete`], then [`Self::finished`] says how it went.
    pub fn advance(&mut self) -> Option<Settings> {
        if self.is_saving() || self.is_importing() || !self.commit_schedule() {
            return None;
        }
        match self.step.next() {
            Some(next) => {
                self.step = next;
                None
            }
            None => {
                self.ending = Ending::Saving;
                Some(self.settings.clone())
            }
        }
    }

    /// The save of the end succeeded, or failed and may be tried again.
    pub fn finished(&mut self, saved: bool) {
        self.ending = if saved {
            Ending::Editing
        } else {
            Ending::Failed
        };
    }

    pub fn toggle(&mut self, switch: Switch) {
        if self.is_saving() || !self.is_enabled(switch) {
            return;
        }
        let settings = &mut self.settings;
        let on = match switch {
            Switch::Startup => &mut settings.start_with_windows,
            Switch::Updates => &mut settings.updates.check_for_updates,
            Switch::UsagePing => &mut settings.usage_ping,
            _ => return,
        };
        *on = !*on;
    }

    /// Opens or closes « Voir ce qui est envoyé »; whether it is now open.
    pub fn toggle_details(&mut self) -> bool {
        self.details_open = !self.details_open;
        self.details_open
    }

    /// Start with Windows waits for the registration, and is left alone when Windows Settings
    /// or a policy decides it.
    fn is_enabled(&self, switch: Switch) -> bool {
        match switch {
            Switch::Startup => self.startup.is_some_and(startup::can_change),
            _ => true,
        }
    }

    /// Start with Windows shows what will happen, not the request, when it is not WorkTime's
    /// to change; and why, then or when another copy holds the registration.
    fn startup_row(&self) -> (bool, Option<Key>) {
        match self.startup {
            None => (self.settings.start_with_windows, None),
            Some(state) if !startup::can_change(state) => {
                (startup::is_on(state), Some(startup::status(state)))
            }
            Some(state) => (
                self.settings.start_with_windows,
                (state == StartupRegistrationState::RegistrationMismatch)
                    .then(|| startup::status(state)),
            ),
        }
    }

    pub fn view(&self) -> PageView {
        let language = self.language();
        let (heading, intro, body) = match self.step {
            Step::Language => (
                Key::OnboardingLanguageTitleText,
                Key::OnboardingLanguageIntroText,
                self.language_body(language),
            ),
            Step::Schedule => (
                Key::OnboardingScheduleTitleText,
                Key::OnboardingScheduleIntroText,
                Body::Rows {
                    rows: self.schedule_rows(language),
                    footnote: None,
                },
            ),
            Step::Connections => (
                Key::OnboardingConnectionsTitle,
                Key::OnboardingConnectionsIntro,
                Body::Rows {
                    rows: self.connection_rows(language),
                    footnote: Some(language.text(Key::OnboardingConnectionsPrivacy)),
                },
            ),
        };
        PageView {
            counter: language.format(
                Key::OnboardingStepFormat,
                &[&self.step.number(), &Step::ALL.len()],
            ),
            step: self.step,
            heading: language.text(heading),
            intro: language.text(intro),
            body,
            previous: self
                .step
                .previous()
                .map(|_| language.text(Key::OnboardingPreviousButton)),
            next: language.text(match self.step.next() {
                Some(_) => Key::OnboardingNextButtonContent,
                None => Key::OnboardingStartButton,
            }),
            saving: self.is_saving(),
            failure: (self.ending == Ending::Failed)
                .then(|| language.text(Key::OnboardingSaveFailed)),
            importing: self.is_importing(),
        }
    }

    fn language_body(&self, language: Language) -> Body {
        let saved = LanguageChoice::saved(self.settings.language_override.as_deref());
        Body::Language {
            name: language.text(Key::OnboardingLanguageInputAutomationPropertiesName),
            options: LanguageChoice::ALL
                .into_iter()
                .map(|choice| LanguageOption {
                    choice,
                    label: language.text(match choice {
                        // The Windows display languages are not read yet: the label names none.
                        LanguageChoice::System => Key::OnboardingLanguageSystem,
                        LanguageChoice::French => Key::OnboardingLanguageFrenchContent,
                        LanguageChoice::English => Key::OnboardingLanguageEnglishContent,
                    }),
                    selected: choice == saved,
                })
                .collect(),
            import: self
                .import
                .as_ref()
                .map(|page| import_offer(page, language)),
        }
    }

    /// The row an error of the schedule is about.
    fn error_row(error: EditError) -> Setting {
        match error {
            EditError::Settings(SettingsError::Lunch) => Setting::Lunch,
            EditError::Settings(SettingsError::Departure) => Setting::Departure,
            _ => Setting::Target,
        }
    }

    fn schedule_rows(&self, language: Language) -> Vec<RowView> {
        [Setting::Target, Setting::Lunch, Setting::Departure]
            .into_iter()
            .map(|setting| {
                let error = self
                    .schedule_error
                    .filter(|&error| Self::error_row(error) == setting);
                let invalid = error.is_some();
                let control = match setting {
                    Setting::Target => Control::Text {
                        field: Field::Target,
                        invalid,
                        enabled: true,
                    },
                    Setting::Lunch => Control::Range {
                        start: Clock::LunchStart,
                        end: Clock::LunchEnd,
                        invalid,
                    },
                    _ => Control::Time {
                        clock: Clock::Departure,
                        invalid,
                    },
                };
                let mut row = setting.row(language, control);
                row.error = error.map(|error| error_text(error, language));
                row
            })
            .collect()
    }

    fn connection_rows(&self, language: Language) -> Vec<RowView> {
        let switch = |switch: Switch, on: bool| Control::Switch {
            switch,
            name: switch.name(language),
            on,
            enabled: self.is_enabled(switch) && !self.is_saving(),
        };
        let (startup_on, status) = self.startup_row();
        let mut startup = Setting::Startup.row(language, switch(Switch::Startup, startup_on));
        startup.description = language.text(Key::OnboardingStartupIntroText);
        startup.status = status.map(|key| language.text(key));
        let updates = self.usage_ping.checks_updates().then(|| {
            let mut updates = Setting::Updates.row(
                language,
                switch(Switch::Updates, self.settings.updates.check_for_updates),
            );
            updates.description = language.text(self.usage_ping.updates_description());
            updates
        });
        let mut usage_ping = Setting::UsagePing.row(
            language,
            switch(Switch::UsagePing, self.settings.usage_ping),
        );
        usage_ping.details = Some(self.details(language));
        [Some(startup), updates, Some(usage_ping)]
            .into_iter()
            .flatten()
            .collect()
    }

    /// « Voir ce qui est envoyé », closed at first, without the channel's line nor the last
    /// send (spec §5).
    fn details(&self, language: Language) -> Details {
        Details {
            label: language.text(Key::SettingsUsagePingDetails),
            shown: self.details_open.then(|| {
                if !self.settings.usage_ping {
                    return PingDetails::Off(language.text(Key::SettingsUsagePingOffText));
                }
                PingDetails::Brief {
                    body: ping_body(self.usage_ping).0,
                    note: format!(
                        "{} {}",
                        language.text(Key::OnboardingUsagePingAllSent),
                        language.text(Key::SettingsUsagePingServerText)
                    ),
                }
            }),
        }
    }
}

/// The import of « Paramètres › Données » as the first step offers it: its confirmation asks to
/// import rather than to replace, since a PC being set up has usually nothing to replace, and
/// says the remaining steps are skipped; when this PC holds days, it says what it would lose.
fn import_offer(page: &DataPage, language: Language) -> ImportOffer {
    let view = page.view();
    ImportOffer {
        lead: language.text(Key::OnboardingImportLead),
        action: language.text(Key::OnboardingImportAction),
        status: view.status,
        error: view
            .rows
            .iter()
            .find(|row| row.row == Row::Import)
            .and_then(|row| row.error),
        confirmation: page.dialog().map(|dialog| ImportDialog {
            title: language.text(Key::OnboardingImportTitle),
            message: language.text(Key::OnboardingImportMessage),
            confirm: language.text(Key::OnboardingImportConfirm),
            ..dialog
        }),
    }
}

/// Saves the choices of « Commencer », as `OnboardingCompletion.CompleteAsync` does: asks
/// Windows for start with Windows, records what it settled on, then saves the settings. A
/// retry after a failed save asks Windows again, which leaves a registration already there.
pub fn complete(
    mut settings: Settings,
    port: Option<&dyn StartupPort>,
    save: impl FnOnce(&Settings) -> Result<(), SaveSettingsError>,
) -> Result<Settings, SaveSettingsError> {
    let requested = settings.start_with_windows;
    let settled = port.map_or(StartupRegistrationState::Unavailable, |port| {
        startup::apply(port, requested)
    });
    settings.start_with_windows = startup::persisted(settled, requested);
    save(&settings)?;
    Ok(settings)
}

#[cfg(test)]
mod tests;
