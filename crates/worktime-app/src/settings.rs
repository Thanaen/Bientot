//! The « Paramètres » window as a form: its pages, the drafts of their fields and what each row
//! shows, over the core's [`SettingsWorkspace`]. Ported from
//! `Views/Settings/SettingsWindow.xaml.cs`; nothing here knows GPUI.
//!
//! As in .NET, a change saves on its own: a switch or the language when it changes, a field
//! when it loses the focus or takes Enter, a number when a spin button steps it. Leaving the
//! window, or the page, with an invalid draft or a failed save asks first.

pub mod appointments;
pub mod data;
pub mod json;
pub mod startup;

use jiff::SignedDuration;
use jiff::civil::Date;
use worktime_core::routing;
use worktime_core::settings::number::{
    self, InputError, format_optional_target_duration, format_target_duration,
};
use worktime_core::settings::{
    EditError, EditGroup, EditStatus, Pending, SettingsEdit, SettingsPort, SettingsWorkspace,
    Summary,
};
use worktime_domain::configuration::{
    AppointmentProposalSettings, NotificationRule, RecurringAppointment, SettingsError,
    WorkTimeSettings,
};
use worktime_domain::sources::ActivitySourceKind;
use worktime_domain::usage_ping::{Build, Channel, usage_ping_due};
use worktime_windows::startup::StartupRegistrationState;

use crate::adjust::TimeField;
use crate::strings::{Key, Language};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    General,
    Schedule,
    Accounting,
    Notifications,
    /// Export and import, under the pages.
    Data,
    /// The file itself, read-only, under the pages.
    Json,
}

impl Page {
    /// The pages of the form, in the navigation's order.
    pub const ALL: [Page; 4] = [
        Page::General,
        Page::Schedule,
        Page::Accounting,
        Page::Notifications,
    ];

    pub fn label(self, language: Language) -> &'static str {
        language.text(match self {
            Self::General => Key::SettingsGeneralNavContent,
            Self::Schedule => Key::SettingsScheduleNavContent,
            Self::Accounting => Key::SettingsAccountingNavContent,
            Self::Notifications => Key::SettingsNotificationsNavContent,
            Self::Data => Key::SettingsDataNavContent,
            Self::Json => Key::SettingsJsonNavContent,
        })
    }

    fn title(self, language: Language) -> &'static str {
        language.text(match self {
            Self::General => Key::SettingsGeneralTitle,
            Self::Schedule => Key::SettingsScheduleTitle,
            Self::Accounting => Key::SettingsAccountingTitle,
            Self::Notifications => Key::SettingsNotificationsTitle,
            Self::Data => Key::SettingsDataTitle,
            Self::Json => Key::SettingsJsonNavContent,
        })
    }

    /// The Lucide icon of its navigation item.
    pub fn icon(self) -> &'static str {
        match self {
            Self::General => "house",
            Self::Schedule => "clock",
            Self::Accounting => "sigma",
            Self::Notifications => "bell",
            Self::Data => "database",
            Self::Json => "braces",
        }
    }

    /// Its groups: an optional heading, then its settings. A page of one group has no heading.
    fn sections(self) -> &'static [(Option<Key>, &'static [Setting])] {
        match self {
            Self::General => &[
                (
                    Some(Key::SettingsApplicationGroup),
                    &[Setting::Language, Setting::Startup],
                ),
                (Some(Key::SettingsDetectionGroup), &[Setting::IdleThreshold]),
                (
                    Some(Key::SettingsConnectionsGroup),
                    &[Setting::Updates, Setting::UsagePing],
                ),
            ],
            Self::Data | Self::Json => &[],
            Self::Schedule => &[
                (
                    Some(Key::SettingsTargetsGroup),
                    &[Setting::Target, Setting::Minimum],
                ),
                (
                    Some(Key::SettingsWorkdayGroup),
                    &[Setting::Lunch, Setting::Departure, Setting::Margin],
                ),
            ],
            Self::Accounting => &[(
                None,
                &[
                    Setting::Credit,
                    Setting::Debt,
                    Setting::DebtLimit,
                    Setting::LongLock,
                ],
            )],
            Self::Notifications => &[(
                None,
                &[
                    Setting::Completion,
                    Setting::DepartureNotification,
                    Setting::DayComplete,
                ],
            )],
        }
    }
}

/// One row of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Language,
    Startup,
    Target,
    Minimum,
    Lunch,
    Departure,
    Margin,
    Credit,
    Debt,
    DebtLimit,
    LongLock,
    Completion,
    DepartureNotification,
    DayComplete,
    IdleThreshold,
    Updates,
    UsagePing,
    /// Under « Rendez-vous réguliers », which [`SettingsForm::appointments`] shows.
    AppointmentProposals,
}

impl Setting {
    fn group(self) -> EditGroup {
        match self {
            Self::Language => EditGroup::Language,
            Self::Startup => EditGroup::Startup,
            Self::IdleThreshold => EditGroup::IdleThreshold,
            Self::Updates => EditGroup::Updates,
            Self::UsagePing => EditGroup::UsagePing,
            Self::AppointmentProposals => EditGroup::AppointmentProposals,
            Self::Target | Self::Minimum => EditGroup::Targets,
            Self::Lunch | Self::Departure => EditGroup::DayTimes,
            Self::Margin => EditGroup::Margin,
            Self::Credit => EditGroup::Credit,
            Self::Debt | Self::DebtLimit => EditGroup::Debt,
            Self::LongLock => EditGroup::LongLock,
            Self::Completion => EditGroup::CompletionApproach,
            Self::DepartureNotification => EditGroup::DepartureApproach,
            Self::DayComplete => EditGroup::DayComplete,
        }
    }

    /// Its card with `control`: no status, no error, no details.
    pub(crate) fn row(self, language: Language, control: Control) -> RowView {
        let (title, description) = self.card();
        RowView {
            setting: self,
            title: language.text(title),
            description: language.text(description),
            tone: RowTone::Normal,
            link: None,
            status: None,
            error: None,
            control,
            details: None,
        }
    }

    /// The header and the description of its .NET `SettingsCard`.
    fn card(self) -> (Key, Key) {
        match self {
            Self::Language => (
                Key::SettingsLanguageCardHeader,
                Key::SettingsLanguageCardDescription,
            ),
            Self::Startup => (
                Key::SettingsStartupCardHeader,
                Key::SettingsStartupCardDescription,
            ),
            Self::Target => (
                Key::SettingsTargetCardHeader,
                Key::SettingsTargetCardDescription,
            ),
            Self::Minimum => (
                Key::SettingsMinimumCardHeader,
                Key::SettingsMinimumCardDescription,
            ),
            Self::Lunch => (
                Key::SettingsLunchCardHeader,
                Key::SettingsLunchCardDescription,
            ),
            Self::Departure => (
                Key::SettingsDepartureCardHeader,
                Key::SettingsDepartureCardDescription,
            ),
            Self::Margin => (
                Key::SettingsMarginCardHeader,
                Key::SettingsMarginCardDescription,
            ),
            Self::Credit => (
                Key::SettingsCreditCardHeader,
                Key::SettingsCreditCardDescription,
            ),
            Self::Debt => (
                Key::SettingsDebtCardHeader,
                Key::SettingsDebtCardDescription,
            ),
            Self::DebtLimit => (
                Key::SettingsDebtLimitCardHeader,
                Key::SettingsDebtLimitCardDescription,
            ),
            Self::LongLock => (
                Key::SettingsLongLockCardHeader,
                Key::SettingsLongLockCardDescription,
            ),
            Self::Completion => (
                Key::SettingsCompletionCardHeader,
                Key::SettingsCompletionCardDescription,
            ),
            Self::DepartureNotification => (
                Key::SettingsDepartureNotificationCardHeader,
                Key::SettingsDepartureNotificationCardDescription,
            ),
            Self::DayComplete => (
                Key::SettingsDayCompleteCardHeader,
                Key::SettingsDayCompleteCardDescription,
            ),
            Self::IdleThreshold => (
                Key::SettingsIdleThresholdCardHeader,
                Key::SettingsIdleThresholdCardDescription,
            ),
            Self::Updates => (
                Key::SettingsUpdatesCardHeader,
                Key::SettingsUpdatesCardDescription,
            ),
            Self::UsagePing => (
                Key::SettingsUsagePingCardHeader,
                Key::SettingsUsagePingCardDescription,
            ),
            Self::AppointmentProposals => (
                Key::SettingsAppointmentProposalsCardHeader,
                Key::SettingsAppointmentProposalsCardDescription,
            ),
        }
    }
}

/// A setting edited as text: a duration as `H:mm`, or a whole number with spin buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Target,
    Minimum,
    DebtLimit,
    Margin,
    LongLock,
    CompletionLead,
    DepartureLead,
    IdleThreshold,
}

/// The spin buttons of a number: the `SmallChange` and the range of its .NET `NumberBox`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spin {
    pub step: i64,
    pub minimum: f64,
    pub maximum: f64,
}

impl Field {
    pub const ALL: [Field; 8] = [
        Field::Target,
        Field::Minimum,
        Field::DebtLimit,
        Field::Margin,
        Field::LongLock,
        Field::CompletionLead,
        Field::DepartureLead,
        Field::IdleThreshold,
    ];

    pub fn group(self) -> EditGroup {
        self.setting().group()
    }

    fn setting(self) -> Setting {
        match self {
            Self::Target => Setting::Target,
            Self::Minimum => Setting::Minimum,
            Self::DebtLimit => Setting::DebtLimit,
            Self::Margin => Setting::Margin,
            Self::LongLock => Setting::LongLock,
            Self::CompletionLead => Setting::Completion,
            Self::DepartureLead => Setting::DepartureNotification,
            Self::IdleThreshold => Setting::IdleThreshold,
        }
    }

    /// The name screen readers give the field.
    pub fn name(self, language: Language) -> &'static str {
        language.text(match self {
            Self::Target => Key::SettingsTargetInputAutomationPropertiesName,
            Self::Minimum => Key::SettingsMinimumInputAutomationPropertiesName,
            Self::DebtLimit => Key::SettingsDebtLimitInputAutomationPropertiesName,
            Self::Margin => Key::SettingsMarginInputAutomationPropertiesName,
            Self::LongLock => Key::SettingsLongLockInputAutomationPropertiesName,
            Self::CompletionLead => Key::SettingsCompletionLeadInputAutomationPropertiesName,
            Self::DepartureLead => Key::SettingsDepartureLeadInputAutomationPropertiesName,
            Self::IdleThreshold => Key::SettingsIdleThresholdInputAutomationPropertiesName,
        })
    }

    /// The unit of a number.
    fn unit(self, language: Language) -> &'static str {
        language.text(match self {
            Self::IdleThreshold => Key::SettingsSecondsUnit,
            _ => Key::SettingsMinutesUnit,
        })
    }

    /// A number's text as the duration it stands for, in its unit.
    fn parse(self, text: &str) -> Result<SignedDuration, InputError> {
        let spin = self.spin().expect("a number has spin buttons");
        match self {
            Self::IdleThreshold => number::parse_seconds(text, spin.minimum, spin.maximum),
            _ => number::parse_minutes(text, spin.minimum, spin.maximum),
        }
    }

    pub fn placeholder(self, language: Language) -> &'static str {
        match self {
            Self::DebtLimit => language.text(Key::SettingsDebtLimitInputPlaceholderText),
            _ => "",
        }
    }

    /// `None` for a duration.
    pub fn spin(self) -> Option<Spin> {
        let (step, minimum, maximum) = match self {
            Self::Target | Self::Minimum | Self::DebtLimit => return None,
            Self::Margin | Self::CompletionLead | Self::DepartureLead => (5, 0., 1440.),
            Self::LongLock => (15, 1., 1440.),
            Self::IdleThreshold => (30, 30., 1800.),
        };
        Some(Spin {
            step,
            minimum,
            maximum,
        })
    }
}

/// A time of day edited as hours and minutes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    LunchStart,
    LunchEnd,
    Departure,
    /// The hours of the appointment the editor of « Rendez-vous réguliers » adds or changes.
    AppointmentStart,
    AppointmentEnd,
}

impl Clock {
    pub const ALL: [Clock; 5] = [
        Clock::LunchStart,
        Clock::LunchEnd,
        Clock::Departure,
        Clock::AppointmentStart,
        Clock::AppointmentEnd,
    ];

    pub fn name(self, language: Language) -> &'static str {
        language.text(match self {
            Self::LunchStart => Key::SettingsLunchStartInputAutomationPropertiesName,
            Self::LunchEnd => Key::SettingsLunchEndInputAutomationPropertiesName,
            Self::Departure => Key::SettingsDepartureInputAutomationPropertiesName,
            Self::AppointmentStart => Key::SettingsAppointmentStartName,
            Self::AppointmentEnd => Key::SettingsAppointmentEndName,
        })
    }

    pub fn is_appointment(self) -> bool {
        matches!(self, Self::AppointmentStart | Self::AppointmentEnd)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Startup,
    Credit,
    Debt,
    Completion,
    DepartureNotification,
    DayComplete,
    Updates,
    UsagePing,
    AppointmentProposals,
}

impl Switch {
    fn group(self) -> EditGroup {
        match self {
            Self::Startup => EditGroup::Startup,
            Self::Credit => EditGroup::Credit,
            Self::Debt => EditGroup::Debt,
            Self::Completion => EditGroup::CompletionApproach,
            Self::DepartureNotification => EditGroup::DepartureApproach,
            Self::DayComplete => EditGroup::DayComplete,
            Self::Updates => EditGroup::Updates,
            Self::UsagePing => EditGroup::UsagePing,
            Self::AppointmentProposals => EditGroup::AppointmentProposals,
        }
    }

    pub(crate) fn name(self, language: Language) -> &'static str {
        language.text(match self {
            Self::Startup => Key::SettingsStartupInputAutomationPropertiesName,
            Self::Credit => Key::SettingsCreditInputAutomationPropertiesName,
            Self::Debt => Key::SettingsDebtInputAutomationPropertiesName,
            Self::Completion => Key::SettingsCompletionInputAutomationPropertiesName,
            Self::DepartureNotification => {
                Key::SettingsDepartureNotificationInputAutomationPropertiesName
            }
            Self::DayComplete => Key::SettingsDayCompleteInputAutomationPropertiesName,
            Self::Updates => Key::SettingsUpdatesInputAutomationPropertiesName,
            Self::UsagePing => Key::SettingsUsagePingInputAutomationPropertiesName,
            Self::AppointmentProposals => Key::SettingsAppointmentProposalsCardHeader,
        })
    }
}

/// The items of « Langue », in the order of the .NET `ComboBox`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LanguageChoice {
    #[default]
    System,
    French,
    English,
}

impl LanguageChoice {
    pub const ALL: [LanguageChoice; 3] = [
        LanguageChoice::System,
        LanguageChoice::French,
        LanguageChoice::English,
    ];

    pub(crate) fn saved(language_override: Option<&str>) -> Self {
        match language_override {
            Some("fr-FR") => Self::French,
            Some("en-US") => Self::English,
            _ => Self::System,
        }
    }

    pub(crate) fn language_override(self) -> Option<String> {
        match self {
            Self::System => None,
            Self::French => Some("fr-FR".to_owned()),
            Self::English => Some("en-US".to_owned()),
        }
    }

    pub fn label(self, language: Language) -> &'static str {
        language.text(match self {
            Self::System => Key::SettingsLanguageSystemContent,
            Self::French => Key::SettingsLanguageFrenchContent,
            Self::English => Key::SettingsLanguageEnglishContent,
        })
    }
}

/// What the unsaved-changes dialog holds back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leaving {
    Window,
    Page(Page),
}

/// The values shown, which may differ from the settings saved while a draft is invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Drafts {
    texts: [String; Field::ALL.len()],
    lunch_start: TimeField,
    lunch_end: TimeField,
    departure: TimeField,
    credit: bool,
    debt: bool,
    completion: bool,
    departure_notification: bool,
    day_complete: bool,
    updates: bool,
    usage_ping: bool,
    language: LanguageChoice,
    appointments: Vec<RecurringAppointment>,
    appointment_proposals: AppointmentProposalSettings,
    appointment_start: TimeField,
    appointment_end: TimeField,
}

pub(super) fn time_field(time: jiff::civil::Time) -> TimeField {
    TimeField::new(time.hour(), time.minute())
}

fn minutes_text(duration: jiff::SignedDuration) -> String {
    (duration.as_secs() / 60).to_string()
}

impl Drafts {
    fn new(settings: &WorkTimeSettings) -> Self {
        let mut drafts = Self {
            texts: Default::default(),
            lunch_start: time_field(settings.lunch_start),
            lunch_end: time_field(settings.lunch_end),
            departure: time_field(settings.preferred_latest_departure),
            credit: settings.weekly_credit_enabled,
            debt: settings.weekly_debt_enabled,
            completion: settings.notifications.completion_approach.enabled,
            departure_notification: settings.notifications.latest_departure_approach.enabled,
            day_complete: settings.notifications.day_complete.enabled,
            updates: settings.updates.check_for_updates,
            usage_ping: settings.usage_ping,
            language: LanguageChoice::saved(settings.language_override.as_deref()),
            appointments: settings.recurring_appointments.clone(),
            appointment_proposals: settings.appointment_proposals.clone(),
            appointment_start: time_field(appointments::NEW_START),
            appointment_end: time_field(appointments::NEW_END),
        };
        for field in Field::ALL {
            drafts.texts[field as usize] = Self::text(field, settings);
        }
        drafts
    }

    fn text(field: Field, settings: &WorkTimeSettings) -> String {
        let notifications = &settings.notifications;
        match field {
            Field::Target => format_target_duration(settings.nominal_daily_target),
            Field::Minimum => format_target_duration(settings.minimum_effective_target),
            Field::DebtLimit => format_optional_target_duration(settings.maximum_weekly_debt),
            Field::Margin => minutes_text(settings.orange_warning_margin),
            Field::LongLock => minutes_text(settings.long_lock_review_threshold),
            Field::CompletionLead => minutes_text(notifications.completion_approach.lead_time),
            Field::DepartureLead => minutes_text(notifications.latest_departure_approach.lead_time),
            Field::IdleThreshold => settings
                .built_in_activity
                .idle_threshold
                .as_secs()
                .to_string(),
        }
    }

    /// Takes the saved values of `group` back.
    fn reload(&mut self, group: EditGroup, settings: &WorkTimeSettings) {
        let saved = Self::new(settings);
        for field in Field::ALL
            .into_iter()
            .filter(|field| field.group() == group)
        {
            self.texts[field as usize] = saved.texts[field as usize].clone();
        }
        match group {
            EditGroup::Language => self.language = saved.language,
            EditGroup::DayTimes => {
                self.lunch_start = saved.lunch_start;
                self.lunch_end = saved.lunch_end;
                self.departure = saved.departure;
            }
            EditGroup::Credit => self.credit = saved.credit,
            EditGroup::Debt => self.debt = saved.debt,
            EditGroup::CompletionApproach => self.completion = saved.completion,
            EditGroup::DepartureApproach => {
                self.departure_notification = saved.departure_notification;
            }
            EditGroup::DayComplete => self.day_complete = saved.day_complete,
            EditGroup::Updates => self.updates = saved.updates,
            EditGroup::UsagePing => self.usage_ping = saved.usage_ping,
            EditGroup::Appointments => self.appointments = saved.appointments,
            EditGroup::AppointmentProposals => {
                self.appointment_proposals = saved.appointment_proposals;
            }
            _ => {}
        }
    }
}

/// What a page shows.
#[derive(Clone, Debug, PartialEq)]
pub struct PageView {
    pub title: &'static str,
    /// The status line under the title, empty while a group shows its own message.
    pub status: Option<&'static str>,
    pub saving: bool,
    /// Above the groups while the settings still name ActivityWatch.
    pub source: Option<SourceCallout>,
    pub sections: Vec<SectionView>,
    /// Under the groups of « Général ».
    pub about: Option<AboutView>,
}

/// « À propos »: the version, and the day it was published when the build knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AboutView {
    pub heading: &'static str,
    pub title: &'static str,
    pub version: &'static str,
    /// What a screen reader names the version: « Version 0.1.0-alpha.242 ».
    pub label: String,
    /// « Publiée le 21 octobre 2026 ».
    pub released: Option<String>,
}

/// ActivityWatch is no longer read: the built-in detection is offered. It cannot be closed.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceCallout {
    pub title: &'static str,
    pub message: &'static str,
    pub action: &'static str,
    /// The switch failed to save.
    pub failure: Option<CalloutView>,
}

/// What « Voir ce qui est envoyé » shows once open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PingDetails {
    On {
        /// The body of the next signal, indented as JSON.
        body: String,
        fields: String,
        server: &'static str,
        last_sent: String,
    },
    /// The onboarding's: the body and one note, without the channel's line nor the last send.
    Brief {
        body: String,
        note: String,
    },
    Off(&'static str),
}

/// The disclosure under the usage signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Details {
    pub label: &'static str,
    /// `None` while closed.
    pub shown: Option<PingDetails>,
}

/// What the usage signal's details and « À propos » are made from, beside the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsagePingFacts {
    pub build: Build,
    /// The day the build was published, `None` for a build made outside a release.
    pub released: Option<Date>,
    /// The local date.
    pub today: Date,
    pub last_sent: Option<Date>,
}

impl UsagePingFacts {
    /// The ZIP build has no feed to ask for updates (D-016): « Rechercher les mises à jour »
    /// stays off and greyed in Paramètres, and the onboarding leaves it out.
    pub(crate) fn checks_updates(self) -> bool {
        self.build.channel() != Channel::Zip
    }

    /// What « Rechercher les mises à jour » does: the installed build also downloads the update
    /// and installs it at the next start.
    pub(crate) fn updates_description(self) -> Key {
        match self.build.channel() {
            Channel::Direct => Key::SettingsUpdatesCardDirectDescription,
            Channel::Store | Channel::Zip => Key::SettingsUpdatesCardDescription,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SectionView {
    pub heading: Option<&'static str>,
    pub rows: Vec<RowView>,
    /// A failed save or a day left to recalculate, under the group.
    pub callouts: Vec<CalloutView>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CalloutView {
    pub group: EditGroup,
    pub tone: Tone,
    pub message: &'static str,
    pub retry: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowView {
    pub setting: Setting,
    pub title: &'static str,
    pub description: &'static str,
    pub tone: RowTone,
    /// A word of the description that opens a page.
    pub link: Option<Link>,
    /// Under the description: what Windows does with start with Windows.
    pub status: Option<&'static str>,
    /// Why the draft was refused.
    pub error: Option<&'static str>,
    pub control: Control,
    pub details: Option<Details>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowTone {
    Normal,
    /// Not in this build: the title as muted as the description, the control disabled.
    Unavailable,
    /// Greyed: the setting it depends on is off.
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    /// Found once in the description.
    pub label: &'static str,
    pub url: &'static str,
}

/// Where the ZIP build sends for new versions (D-016).
pub const RELEASES_LINK: Link = Link {
    label: "GitHub",
    url: "https://github.com/Thanaen/WorkTime/releases",
};

#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Language {
        name: &'static str,
        value: LanguageChoice,
    },
    Text {
        field: Field,
        invalid: bool,
        enabled: bool,
    },
    Number {
        field: Field,
        unit: &'static str,
        invalid: bool,
        enabled: bool,
    },
    Time {
        clock: Clock,
        invalid: bool,
    },
    Range {
        start: Clock,
        end: Clock,
        invalid: bool,
    },
    Switch {
        switch: Switch,
        name: &'static str,
        on: bool,
        enabled: bool,
    },
    /// A notification rule: its lead time, enabled with the rule, then its switch.
    Rule {
        field: Field,
        unit: &'static str,
        invalid: bool,
        switch: Switch,
        name: &'static str,
        on: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogView {
    pub title: &'static str,
    pub message: &'static str,
    pub discard: &'static str,
    pub keep_editing: &'static str,
}

pub struct SettingsForm {
    workspace: SettingsWorkspace,
    language: Language,
    page: Page,
    drafts: Drafts,
    /// Set while the unsaved-changes dialog shows.
    leaving: Option<Leaving>,
    /// The editor of « Rendez-vous réguliers », while open.
    appointment: Option<appointments::AppointmentDraft>,
    usage_ping: UsagePingFacts,
    details_open: bool,
    /// `None` until the registration has been read.
    startup: Option<StartupRegistrationState>,
    /// What the switch was turned to, until Windows answers.
    startup_requested: Option<bool>,
}

impl SettingsForm {
    pub fn new(settings: WorkTimeSettings, usage_ping: UsagePingFacts, language: Language) -> Self {
        Self {
            drafts: Drafts::new(&settings),
            workspace: SettingsWorkspace::new(settings),
            language,
            page: Page::default(),
            leaving: None,
            appointment: None,
            usage_ping,
            details_open: false,
            startup: None,
            startup_requested: None,
        }
    }

    /// The registration as read, or as Windows settled on it.
    pub fn show_startup(&mut self, state: StartupRegistrationState) {
        self.startup = Some(state);
    }

    /// The registration, once read.
    pub fn startup(&self) -> Option<StartupRegistrationState> {
        self.startup
    }

    /// Shows what Windows settled on for `pending`, then saves it if Windows took the change.
    pub fn finish_startup(
        &mut self,
        pending: Pending,
        settled: StartupRegistrationState,
        port: &mut impl SettingsPort,
    ) {
        let requested = pending.startup().expect("a start-with-Windows change");
        self.startup = Some(settled);
        self.startup_requested = None;
        self.workspace
            .finish_startup(pending, startup::applied(settled, requested), port);
    }

    /// The saved settings name ActivityWatch, which v1 no longer reads.
    pub fn names_activity_watch(&self) -> bool {
        routing::selected(self.current()) == ActivitySourceKind::ActivityWatch
    }

    /// The navigation item of `page` carries a warning: the ActivityWatch callout shows there.
    pub fn warns(&self, page: Page) -> bool {
        page == Page::General && self.names_activity_watch()
    }

    /// « Utiliser la détection intégrée ».
    pub fn use_built_in(&mut self) -> Option<Pending> {
        self.commit(EditGroup::ActivitySource)
    }

    /// Opens or closes « Voir ce qui est envoyé ».
    pub fn toggle_details(&mut self) {
        self.details_open = !self.details_open;
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn current(&self) -> &WorkTimeSettings {
        self.workspace.current()
    }

    pub fn text(&self, field: Field) -> &str {
        &self.drafts.texts[field as usize]
    }

    pub fn set_text(&mut self, field: Field, text: &str) {
        text.clone_into(&mut self.drafts.texts[field as usize]);
    }

    pub fn time(&self, clock: Clock) -> TimeField {
        match clock {
            Clock::LunchStart => self.drafts.lunch_start,
            Clock::LunchEnd => self.drafts.lunch_end,
            Clock::Departure => self.drafts.departure,
            Clock::AppointmentStart => self.drafts.appointment_start,
            Clock::AppointmentEnd => self.drafts.appointment_end,
        }
    }

    pub fn time_mut(&mut self, clock: Clock) -> &mut TimeField {
        match clock {
            Clock::LunchStart => &mut self.drafts.lunch_start,
            Clock::LunchEnd => &mut self.drafts.lunch_end,
            Clock::Departure => &mut self.drafts.departure,
            Clock::AppointmentStart => &mut self.drafts.appointment_start,
            Clock::AppointmentEnd => &mut self.drafts.appointment_end,
        }
    }

    pub fn is_on(&self, switch: Switch) -> bool {
        match switch {
            Switch::Startup => self
                .startup_requested
                .or(self.startup.map(startup::is_on))
                .unwrap_or(false),
            Switch::Credit => self.drafts.credit,
            Switch::Debt => self.drafts.debt,
            Switch::Completion => self.drafts.completion,
            Switch::DepartureNotification => self.drafts.departure_notification,
            Switch::DayComplete => self.drafts.day_complete,
            Switch::Updates => self.drafts.updates && self.usage_ping.checks_updates(),
            Switch::UsagePing => self.drafts.usage_ping,
            Switch::AppointmentProposals => self.drafts.appointment_proposals.enabled,
        }
    }

    /// Debt only exists on top of weekly credit, and a lead time with its rule.
    /// Start with Windows waits for the registration, and for Windows to answer a change.
    pub fn is_enabled(&self, switch: Switch) -> bool {
        match switch {
            Switch::Debt => self.drafts.credit,
            Switch::Startup => {
                self.startup.is_some_and(startup::can_change)
                    && self.workspace.state(EditGroup::Startup).status != EditStatus::Saving
            }
            Switch::Updates => self.usage_ping.checks_updates(),
            _ => true,
        }
    }

    pub fn is_field_enabled(&self, field: Field) -> bool {
        match field {
            Field::DebtLimit => self.drafts.credit && self.drafts.debt,
            Field::CompletionLead => self.drafts.completion,
            Field::DepartureLead => self.drafts.departure_notification,
            _ => true,
        }
    }

    pub fn language(&self) -> LanguageChoice {
        self.drafts.language
    }

    /// Saves the drafts of `group`, or marks it invalid. The pending save runs through
    /// [`Self::run`].
    pub fn commit(&mut self, group: EditGroup) -> Option<Pending> {
        if self.leaving.is_some() {
            return None;
        }
        match self.edit(group) {
            Ok(edit) => self.workspace.submit(edit),
            Err(error) => {
                self.workspace.mark_invalid(group, EditError::Input(error));
                None
            }
        }
    }

    pub fn toggle(&mut self, switch: Switch) -> Option<Pending> {
        if self.leaving.is_some() || !self.is_enabled(switch) {
            return None;
        }
        let on = match switch {
            Switch::Startup => {
                self.startup_requested = Some(!self.is_on(switch));
                return self.commit(switch.group());
            }
            Switch::Credit => &mut self.drafts.credit,
            Switch::Debt => &mut self.drafts.debt,
            Switch::Completion => &mut self.drafts.completion,
            Switch::DepartureNotification => &mut self.drafts.departure_notification,
            Switch::DayComplete => &mut self.drafts.day_complete,
            Switch::Updates => &mut self.drafts.updates,
            Switch::UsagePing => &mut self.drafts.usage_ping,
            Switch::AppointmentProposals => &mut self.drafts.appointment_proposals.enabled,
        };
        *on = !*on;
        self.commit(switch.group())
    }

    pub fn choose_language(&mut self, choice: LanguageChoice) -> Option<Pending> {
        if self.leaving.is_some() || self.drafts.language == choice {
            return None;
        }
        self.drafts.language = choice;
        self.commit(EditGroup::Language)
    }

    /// A spin button of `field`: `steps` of its step from its value, kept within its range. A
    /// draft that is not a number starts from the minimum.
    pub fn spin(&mut self, field: Field, steps: i64) -> Option<Pending> {
        let spin = field.spin()?;
        if self.leaving.is_some() || !self.is_field_enabled(field) {
            return None;
        }
        let value = self.text(field).trim().parse::<i64>().ok();
        let value = value.map_or(spin.minimum as i64, |value| value + steps * spin.step);
        let value = value.clamp(spin.minimum as i64, spin.maximum as i64);
        self.set_text(field, &value.to_string());
        self.commit(field.group())
    }

    pub fn retry(&mut self, group: EditGroup) -> Option<Pending> {
        if self.leaving.is_some() {
            return None;
        }
        self.workspace.retry(group)
    }

    pub fn run(&mut self, pending: Pending, port: &mut impl SettingsPort) {
        self.workspace.run(pending, port);
    }

    pub fn is_saving(&self) -> bool {
        self.workspace.is_saving()
    }

    /// Whether the window may close now; else the unsaved-changes dialog shows.
    pub fn may_close(&mut self) -> bool {
        self.may_leave(Leaving::Window)
    }

    /// Shows `page`, unless the unsaved-changes dialog shows instead. While it shows, the page
    /// it holds back stays the one it lets through.
    pub fn go_to(&mut self, page: Page) -> bool {
        if self.leaving.is_some() {
            return false;
        }
        if page == self.page {
            return true;
        }
        let may = self.may_leave(Leaving::Page(page));
        if may {
            self.page = page;
            self.appointment = None;
        }
        may
    }

    fn may_leave(&mut self, leaving: Leaving) -> bool {
        if self.workspace.has_unsaved_changes() {
            self.leaving = Some(leaving);
            return false;
        }
        true
    }

    pub fn dialog(&self) -> Option<DialogView> {
        self.leaving.map(|_| DialogView {
            title: self.language.text(Key::SettingsUnsavedTitle),
            message: self.language.text(Key::SettingsUnsavedMessage),
            discard: self.language.text(Key::SettingsDiscard),
            keep_editing: self.language.text(Key::SettingsKeepEditing),
        })
    }

    pub fn keep_editing(&mut self) {
        self.leaving = None;
    }

    /// Forgets the invalid drafts and failed saves, shows the saved values again, and returns
    /// what the dialog held back, now allowed.
    pub fn discard(&mut self) -> Option<Leaving> {
        let unsaved = EditGroup::ALL.into_iter().filter(|&group| {
            matches!(
                self.workspace.state(group).status,
                EditStatus::Invalid | EditStatus::SaveFailed
            )
        });
        let unsaved: Vec<_> = unsaved.collect();
        self.workspace.discard_unsaved();
        self.appointment = None;
        for group in unsaved {
            self.drafts.reload(group, self.workspace.current());
        }
        let leaving = self.leaving.take();
        if let Some(Leaving::Page(page)) = leaving {
            self.page = page;
        }
        leaving
    }

    fn edit(&self, group: EditGroup) -> Result<SettingsEdit, InputError> {
        let drafts = &self.drafts;
        let text = |field: Field| self.text(field);
        let minutes = |field: Field| field.parse(text(field));
        Ok(match group {
            EditGroup::Language => SettingsEdit::Language(drafts.language.language_override()),
            EditGroup::Startup => SettingsEdit::StartWithWindows(self.is_on(Switch::Startup)),
            EditGroup::Targets => {
                let target = number::parse_target_duration(text(Field::Target));
                let minimum = number::parse_target_duration(text(Field::Minimum));
                SettingsEdit::Targets {
                    target: target?,
                    minimum: minimum?,
                }
            }
            EditGroup::DayTimes => SettingsEdit::DayTimes {
                lunch_start: drafts.lunch_start.time(),
                lunch_end: drafts.lunch_end.time(),
                departure: drafts.departure.time(),
            },
            EditGroup::Margin => SettingsEdit::Margin(minutes(Field::Margin)?),
            EditGroup::Credit => SettingsEdit::Credit(drafts.credit),
            EditGroup::LongLock => SettingsEdit::LongLock(minutes(Field::LongLock)?),
            EditGroup::CompletionApproach => SettingsEdit::CompletionApproach(
                NotificationRule::new(drafts.completion, minutes(Field::CompletionLead)?),
            ),
            EditGroup::DepartureApproach => SettingsEdit::DepartureApproach(NotificationRule::new(
                drafts.departure_notification,
                minutes(Field::DepartureLead)?,
            )),
            EditGroup::DayComplete => SettingsEdit::DayComplete(drafts.day_complete),
            EditGroup::Debt => SettingsEdit::Debt {
                enabled: drafts.debt,
                limit: number::parse_optional_target_duration(text(Field::DebtLimit))?,
            },
            EditGroup::Updates => SettingsEdit::Updates(drafts.updates),
            EditGroup::IdleThreshold => {
                SettingsEdit::IdleThreshold(Field::IdleThreshold.parse(text(Field::IdleThreshold))?)
            }
            EditGroup::UsagePing => SettingsEdit::UsagePing(drafts.usage_ping),
            EditGroup::ActivitySource => SettingsEdit::ActivitySource(ActivitySourceKind::BuiltIn),
            EditGroup::Appointments => SettingsEdit::Appointments(drafts.appointments.clone()),
            EditGroup::AppointmentProposals => {
                SettingsEdit::AppointmentProposals(drafts.appointment_proposals.clone())
            }
        })
    }

    /// The row whose control the error of `group` is about.
    fn error_row(&self, group: EditGroup, error: EditError) -> Setting {
        match (group, error) {
            (EditGroup::Targets, EditError::Input(_)) => {
                if number::parse_target_duration(self.text(Field::Target)).is_err() {
                    Setting::Target
                } else {
                    Setting::Minimum
                }
            }
            (EditGroup::Targets, EditError::Settings(SettingsError::Target)) => Setting::Target,
            (EditGroup::Targets, _) => Setting::Minimum,
            (EditGroup::DayTimes, EditError::Settings(SettingsError::Lunch)) => Setting::Lunch,
            (EditGroup::DayTimes, _) => Setting::Departure,
            (EditGroup::Debt, _) => Setting::DebtLimit,
            _ => Page::ALL
                .iter()
                .flat_map(|page| page.sections())
                .flat_map(|(_, settings)| settings.iter().copied())
                .find(|setting| setting.group() == group)
                .expect("each group has a row"),
        }
    }

    pub fn view(&self) -> PageView {
        let language = self.language;
        let (status, saving) = match self.workspace.summary() {
            Summary::Saving => (Some(language.text(Key::SettingsSaving)), true),
            Summary::Saved => (Some(language.text(Key::SettingsSavedQuiet)), false),
            Summary::Unsettled => (None, false),
        };
        let sections = self
            .page
            .sections()
            .iter()
            .map(|(heading, settings)| {
                let mut groups: Vec<EditGroup> = Vec::new();
                for setting in settings.iter() {
                    if !groups.contains(&setting.group()) {
                        groups.push(setting.group());
                    }
                }
                SectionView {
                    heading: heading.map(|key| language.text(key)),
                    rows: settings.iter().map(|&setting| self.row(setting)).collect(),
                    callouts: groups
                        .into_iter()
                        .filter_map(|group| self.callout(group))
                        .collect(),
                }
            })
            .collect();
        PageView {
            title: self.page.title(language),
            status,
            saving,
            source: self.source_callout(),
            sections,
            about: (self.page == Page::General).then(|| self.about()),
        }
    }

    fn about(&self) -> AboutView {
        let language = self.language;
        let title = language.text(Key::SettingsVersionCardHeader);
        let version = self.usage_ping.build.version();
        AboutView {
            heading: language.text(Key::SettingsAboutGroup),
            title,
            version,
            label: format!("{title} {version}"),
            released: self
                .usage_ping
                .released
                .map(|date| language.format_dates(Key::SettingsVersionReleasedFormat, &[date])),
        }
    }

    fn source_callout(&self) -> Option<SourceCallout> {
        if !self.warns(self.page) {
            return None;
        }
        let language = self.language;
        Some(SourceCallout {
            title: language.text(Key::SettingsActivityWatchCalloutTitle),
            message: language.text(Key::SettingsActivityWatchCalloutMessage),
            action: language.text(Key::SettingsActivityWatchCalloutAction),
            failure: self.callout(EditGroup::ActivitySource),
        })
    }

    fn details(&self) -> Details {
        let language = self.language;
        Details {
            label: language.text(Key::SettingsUsagePingDetails),
            shown: self.details_open.then(|| {
                if !self.drafts.usage_ping {
                    return PingDetails::Off(language.text(Key::SettingsUsagePingOffText));
                }
                let UsagePingFacts {
                    today, last_sent, ..
                } = self.usage_ping;
                let (body, channel) = ping_body(self.usage_ping);
                let channel = language.text(match channel {
                    Channel::Store => Key::UsagePingChannelStore,
                    Channel::Direct => Key::UsagePingChannelDirect,
                    Channel::Zip => Key::UsagePingChannelZip,
                });
                let last_sent = match last_sent {
                    None => language
                        .text(Key::SettingsUsagePingLastSentNever)
                        .to_owned(),
                    Some(sent) if sent == today => language
                        .text(Key::SettingsUsagePingLastSentToday)
                        .to_owned(),
                    Some(sent) if today.yesterday().ok() == Some(sent) => language
                        .text(Key::SettingsUsagePingLastSentYesterday)
                        .to_owned(),
                    Some(sent) => {
                        language.format_dates(Key::SettingsUsagePingLastSentDateFormat, &[sent])
                    }
                };
                PingDetails::On {
                    body,
                    fields: language.format(Key::SettingsUsagePingFieldsFormat, &[&channel]),
                    server: language.text(Key::SettingsUsagePingServerText),
                    last_sent: language.format(Key::SettingsUsagePingLastSentFormat, &[&last_sent]),
                }
            }),
        }
    }

    fn callout(&self, group: EditGroup) -> Option<CalloutView> {
        let state = self.workspace.state(group);
        let (tone, message) = match state.status {
            EditStatus::SaveFailed if group == EditGroup::ActivitySource => (
                Tone::Error,
                self.language.text(Key::ActivitySourceSwitchFailed),
            ),
            EditStatus::SaveFailed => (Tone::Error, error_text(state.error?, self.language)),
            EditStatus::RefreshPending => {
                (Tone::Warning, self.language.text(Key::SettingsSavedPending))
            }
            _ => return None,
        };
        Some(CalloutView {
            group,
            tone,
            message,
            retry: self.language.text(Key::SettingsRetryContent),
        })
    }

    fn row(&self, setting: Setting) -> RowView {
        let language = self.language;
        let state = self.workspace.state(setting.group());
        let error = state
            .error
            .filter(|_| state.status == EditStatus::Invalid)
            .filter(|&error| self.error_row(setting.group(), error) == setting);
        let invalid = error.is_some();
        let switch = |switch: Switch| Control::Switch {
            switch,
            name: switch.name(language),
            on: self.is_on(switch),
            enabled: self.is_enabled(switch),
        };
        let rule = |field: Field, switch: Switch| Control::Rule {
            field,
            unit: field.unit(language),
            invalid,
            switch,
            name: switch.name(language),
            on: self.is_on(switch),
        };
        let text = |field: Field| Control::Text {
            field,
            invalid,
            enabled: self.is_field_enabled(field),
        };
        let number = |field: Field| Control::Number {
            field,
            unit: field.unit(language),
            invalid,
            enabled: true,
        };
        let control = match setting {
            Setting::Language => Control::Language {
                name: language.text(Key::SettingsLanguageInputAutomationPropertiesName),
                value: self.drafts.language,
            },
            Setting::Startup => switch(Switch::Startup),
            Setting::Target => text(Field::Target),
            Setting::Minimum => text(Field::Minimum),
            Setting::Lunch => Control::Range {
                start: Clock::LunchStart,
                end: Clock::LunchEnd,
                invalid,
            },
            Setting::Departure => Control::Time {
                clock: Clock::Departure,
                invalid,
            },
            Setting::Margin => number(Field::Margin),
            Setting::Credit => switch(Switch::Credit),
            Setting::Debt => switch(Switch::Debt),
            Setting::DebtLimit => text(Field::DebtLimit),
            Setting::LongLock => number(Field::LongLock),
            Setting::Completion => rule(Field::CompletionLead, Switch::Completion),
            Setting::DepartureNotification => {
                rule(Field::DepartureLead, Switch::DepartureNotification)
            }
            Setting::DayComplete => switch(Switch::DayComplete),
            Setting::IdleThreshold => number(Field::IdleThreshold),
            Setting::Updates => switch(Switch::Updates),
            Setting::UsagePing => switch(Switch::UsagePing),
            Setting::AppointmentProposals => switch(Switch::AppointmentProposals),
        };
        let row = RowView {
            tone: match setting {
                Setting::Debt if !self.is_enabled(Switch::Debt) => RowTone::Disabled,
                Setting::DebtLimit if !self.is_field_enabled(Field::DebtLimit) => RowTone::Disabled,
                _ => RowTone::Normal,
            },
            status: self
                .startup
                .filter(|_| setting == Setting::Startup)
                .map(|state| language.text(startup::status(state))),
            error: error.map(|error| error_text(error, language)),
            details: (setting == Setting::UsagePing).then(|| self.details()),
            ..setting.row(language, control)
        };
        if setting == Setting::Updates && !self.usage_ping.checks_updates() {
            return RowView {
                tone: RowTone::Unavailable,
                description: language.text(Key::SettingsUpdatesZipText),
                link: Some(RELEASES_LINK),
                ..row
            };
        }
        if setting == Setting::Updates {
            return RowView {
                description: language.text(self.usage_ping.updates_description()),
                ..row
            };
        }
        row
    }
}

/// The body of the next usage signal, indented as JSON, and the channel it names.
pub(crate) fn ping_body(facts: UsagePingFacts) -> (String, Channel) {
    let UsagePingFacts {
        build,
        today,
        last_sent,
        ..
    } = facts;
    // Once today's signal is sent, the next one goes the day after.
    let day = last_sent
        .filter(|&sent| sent >= today)
        .map_or(today, |sent| {
            sent.tomorrow().expect("a date has a next day")
        });
    let ping = usage_ping_due(true, last_sent, day, build)
        .expect("a signal is due the day after the last one");
    let body = format!(
        "{{\n  \"version\": \"{}\",\n  \"channel\": \"{}\",\n  \"first_this_month\": {}\n}}",
        ping.version,
        ping.channel.as_str(),
        ping.first_this_month
    );
    (body, ping.channel)
}

/// The message of an [`EditError`], from the `.resw` key the core names.
pub fn error_text(error: EditError, language: Language) -> &'static str {
    language.text(match error {
        EditError::SaveFailed => Key::SettingsSaveFailed,
        EditError::StartupChangeFailed => Key::SettingsStartupChangeFailed,
        EditError::Input(error) => match error {
            InputError::TargetDurationRequired => Key::SettingsTargetDurationRequired,
            InputError::TargetDurationOutOfRange => Key::SettingsTargetDurationOutOfRange,
            InputError::WholeNumberRequired => Key::SettingsWholeNumberRequired,
            InputError::WholeSecondsRequired => Key::SettingsWholeSecondsRequired,
            InputError::NumberOutOfRange => Key::SettingsNumberOutOfRange,
        },
        EditError::Settings(error) => match error {
            SettingsError::Schema => Key::SettingsSchema,
            SettingsError::Target => Key::SettingsTarget,
            SettingsError::Minimum => Key::SettingsMinimum,
            SettingsError::Lunch => Key::SettingsLunch,
            SettingsError::Departure => Key::SettingsDeparture,
            SettingsError::Margin => Key::SettingsMargin,
            SettingsError::Lock => Key::SettingsLock,
            SettingsError::Debt => Key::SettingsDebt,
            SettingsError::Language => Key::SettingsLanguage,
            SettingsError::Source => Key::SettingsSource,
            SettingsError::IdleThreshold => Key::SettingsIdleThreshold,
            SettingsError::Notifications => Key::SettingsNotifications,
            SettingsError::Storage => Key::SettingsStorage,
            SettingsError::Appointments => Key::SettingsAppointments,
        },
    })
}

#[cfg(test)]
mod tests;
