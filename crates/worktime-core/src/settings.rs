//! The editing of the settings window, ported from `Services/SettingsWorkspace.cs` and
//! `SettingsService.cs`. Each group of settings is validated and saved on its own, so that an
//! invalid draft in one group never blocks another, and a failed save keeps what to retry.
//!
//! .NET queues the saves on tasks. Here a save is accepted by [`SettingsWorkspace::submit`],
//! which marks its group as saving, and runs later through [`SettingsWorkspace::run`]: the
//! window draws the saving state in between. A group's generation still tells a late result
//! from the newest draft.

pub mod language;
pub mod number;

use std::collections::BTreeMap;

use jiff::SignedDuration;
use jiff::civil::Time;
use worktime_domain::configuration::{
    AppointmentProposalSettings, NotificationRule, RecurringAppointment, SettingsError,
    WorkTimeSettings,
};
use worktime_domain::sources::ActivitySourceKind;
use worktime_store::settings::SaveSettingsError;

use self::number::InputError;

/// The settings saved together, as one control or a pair of them edits them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EditGroup {
    Language,
    Startup,
    Targets,
    DayTimes,
    Margin,
    Credit,
    LongLock,
    CompletionApproach,
    DepartureApproach,
    DayComplete,
    Debt,
    Updates,
    IdleThreshold,
    UsagePing,
    ActivitySource,
    Appointments,
    AppointmentProposals,
}

impl EditGroup {
    pub const ALL: [EditGroup; 17] = [
        EditGroup::Language,
        EditGroup::Startup,
        EditGroup::Targets,
        EditGroup::DayTimes,
        EditGroup::Margin,
        EditGroup::Credit,
        EditGroup::LongLock,
        EditGroup::CompletionApproach,
        EditGroup::DepartureApproach,
        EditGroup::DayComplete,
        EditGroup::Debt,
        EditGroup::Updates,
        EditGroup::IdleThreshold,
        EditGroup::UsagePing,
        EditGroup::ActivitySource,
        EditGroup::Appointments,
        EditGroup::AppointmentProposals,
    ];
}

/// The new values of one group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsEdit {
    /// `None` follows Windows; else `fr-FR` or `en-US`.
    Language(Option<String>),
    /// Saved once Windows took the change, through [`SettingsWorkspace::finish_startup`].
    StartWithWindows(bool),
    Targets {
        target: SignedDuration,
        minimum: SignedDuration,
    },
    DayTimes {
        lunch_start: Time,
        lunch_end: Time,
        departure: Time,
    },
    Margin(SignedDuration),
    Credit(bool),
    LongLock(SignedDuration),
    CompletionApproach(NotificationRule),
    DepartureApproach(NotificationRule),
    /// The rule's lead time is kept.
    DayComplete(bool),
    Debt {
        enabled: bool,
        limit: Option<SignedDuration>,
    },
    /// The time of the last check is kept.
    Updates(bool),
    IdleThreshold(SignedDuration),
    UsagePing(bool),
    /// The port records the change in the selection ledger with the save, through
    /// `routing::ActivitySourceSwitch`.
    ActivitySource(ActivitySourceKind),
    /// The whole list, as « Rendez-vous réguliers » edits one appointment at a time.
    Appointments(Vec<RecurringAppointment>),
    /// « Proposer des rendez-vous d’après vos corrections » with the proposals turned down.
    AppointmentProposals(AppointmentProposalSettings),
}

impl SettingsEdit {
    pub fn group(&self) -> EditGroup {
        match self {
            Self::Language(_) => EditGroup::Language,
            Self::StartWithWindows(_) => EditGroup::Startup,
            Self::Targets { .. } => EditGroup::Targets,
            Self::DayTimes { .. } => EditGroup::DayTimes,
            Self::Margin(_) => EditGroup::Margin,
            Self::Credit(_) => EditGroup::Credit,
            Self::LongLock(_) => EditGroup::LongLock,
            Self::CompletionApproach(_) => EditGroup::CompletionApproach,
            Self::DepartureApproach(_) => EditGroup::DepartureApproach,
            Self::DayComplete(_) => EditGroup::DayComplete,
            Self::Debt { .. } => EditGroup::Debt,
            Self::Updates(_) => EditGroup::Updates,
            Self::IdleThreshold(_) => EditGroup::IdleThreshold,
            Self::UsagePing(_) => EditGroup::UsagePing,
            Self::ActivitySource(_) => EditGroup::ActivitySource,
            Self::Appointments(_) => EditGroup::Appointments,
            Self::AppointmentProposals(_) => EditGroup::AppointmentProposals,
        }
    }

    pub fn apply(&self, settings: &WorkTimeSettings) -> WorkTimeSettings {
        let mut settings = settings.clone();
        match self {
            Self::Language(language) => settings.language_override = language.clone(),
            Self::StartWithWindows(enabled) => settings.start_with_windows = *enabled,
            Self::Targets { target, minimum } => {
                settings.nominal_daily_target = *target;
                settings.minimum_effective_target = *minimum;
            }
            Self::DayTimes {
                lunch_start,
                lunch_end,
                departure,
            } => {
                settings.lunch_start = *lunch_start;
                settings.lunch_end = *lunch_end;
                settings.preferred_latest_departure = *departure;
            }
            Self::Margin(margin) => settings.orange_warning_margin = *margin,
            Self::Credit(enabled) => settings.weekly_credit_enabled = *enabled,
            Self::LongLock(threshold) => settings.long_lock_review_threshold = *threshold,
            Self::CompletionApproach(rule) => settings.notifications.completion_approach = *rule,
            Self::DepartureApproach(rule) => {
                settings.notifications.latest_departure_approach = *rule;
            }
            Self::DayComplete(enabled) => settings.notifications.day_complete.enabled = *enabled,
            Self::Debt { enabled, limit } => {
                settings.weekly_debt_enabled = *enabled;
                settings.maximum_weekly_debt = *limit;
            }
            Self::Updates(enabled) => settings.updates.check_for_updates = *enabled,
            Self::IdleThreshold(threshold) => {
                settings.built_in_activity.idle_threshold = *threshold;
            }
            Self::UsagePing(enabled) => settings.usage_ping = *enabled,
            Self::ActivitySource(source) => settings.activity_source = Some(*source),
            Self::Appointments(appointments) => {
                settings.recurring_appointments.clone_from(appointments);
            }
            Self::AppointmentProposals(proposals) => {
                settings.appointment_proposals.clone_from(proposals);
            }
        }
        settings
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditStatus {
    Saved,
    Saving,
    /// The draft was refused before any save.
    Invalid,
    /// The save failed; [`SettingsWorkspace::retry`] runs it again.
    SaveFailed,
    /// Saved, but the day was not recalculated; [`SettingsWorkspace::retry`] recalculates it.
    RefreshPending,
}

/// What a group's message says, named after its `.resw` key by [`Self::resource_key`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    Settings(SettingsError),
    Input(InputError),
    SaveFailed,
    /// Windows did not take a start-with-Windows change.
    StartupChangeFailed,
}

impl EditError {
    pub fn resource_key(self) -> &'static str {
        match self {
            Self::Settings(error) => error.resource_key(),
            Self::Input(error) => error.resource_key(),
            Self::SaveFailed => "SettingsSaveFailed",
            Self::StartupChangeFailed => "SettingsStartupChangeFailed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupState {
    pub generation: u64,
    pub status: EditStatus,
    /// Set while [`EditStatus::Invalid`] or [`EditStatus::SaveFailed`].
    pub error: Option<EditError>,
}

impl GroupState {
    const UNTOUCHED: Self = Self {
        generation: 0,
        status: EditStatus::Saved,
        error: None,
    };
}

/// Where the settings go once accepted: the file, then the app's recalculated day.
pub trait SettingsPort {
    /// Writes `settings` and makes them the app's.
    fn save(&mut self, settings: &WorkTimeSettings) -> Result<(), SaveSettingsError>;

    /// Recalculates the day on the settings saved last; whether it could.
    fn refresh(&mut self) -> bool;
}

/// A save or a recalculation the workspace accepted, for [`SettingsWorkspace::run`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    group: EditGroup,
    generation: u64,
    operation: Operation,
}

impl Pending {
    pub fn group(&self) -> EditGroup {
        self.group
    }

    /// What a start-with-Windows change asks Windows for, before it saves.
    pub fn startup(&self) -> Option<bool> {
        match self.operation {
            Operation::Save(SettingsEdit::StartWithWindows(enabled)) => Some(enabled),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Operation {
    Save(SettingsEdit),
    Refresh,
}

/// The status line under the window's title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Summary {
    Saving,
    Saved,
    /// A group is invalid, failed or waits for a recalculation: its own message says so.
    Unsettled,
}

pub struct SettingsWorkspace {
    current: WorkTimeSettings,
    /// The day was recalculated on `current`.
    refreshed: bool,
    states: BTreeMap<EditGroup, GroupState>,
    failed: BTreeMap<EditGroup, Pending>,
}

impl SettingsWorkspace {
    pub fn new(current: WorkTimeSettings) -> Self {
        Self {
            current,
            refreshed: true,
            states: BTreeMap::new(),
            failed: BTreeMap::new(),
        }
    }

    /// The settings saved last.
    pub fn current(&self) -> &WorkTimeSettings {
        &self.current
    }

    pub fn state(&self, group: EditGroup) -> GroupState {
        self.states
            .get(&group)
            .copied()
            .unwrap_or(GroupState::UNTOUCHED)
    }

    /// A draft is invalid or failed to save: closing would lose it.
    pub fn has_unsaved_changes(&self) -> bool {
        self.states
            .values()
            .any(|state| matches!(state.status, EditStatus::Invalid | EditStatus::SaveFailed))
    }

    pub fn is_saving(&self) -> bool {
        self.states
            .values()
            .any(|state| state.status == EditStatus::Saving)
    }

    pub fn summary(&self) -> Summary {
        if self.is_saving() {
            Summary::Saving
        } else if self
            .states
            .values()
            .all(|state| state.status == EditStatus::Saved)
        {
            Summary::Saved
        } else {
            Summary::Unsettled
        }
    }

    /// Accepts `edit` when the settings it leaves are valid; else marks its group invalid with
    /// the first error.
    pub fn submit(&mut self, edit: SettingsEdit) -> Option<Pending> {
        let group = edit.group();
        let generation = self.next_generation(group);
        if let Some(&error) = edit.apply(&self.current).validate().first() {
            self.invalid(group, generation, EditError::Settings(error));
            return None;
        }
        Some(self.start(Pending {
            group,
            generation,
            operation: Operation::Save(edit),
        }))
    }

    /// A draft that is not even a value, such as « 7h ».
    pub fn mark_invalid(&mut self, group: EditGroup, error: EditError) {
        let generation = self.next_generation(group);
        self.invalid(group, generation, error);
    }

    /// Runs again the failed save of `group`, or its recalculation.
    pub fn retry(&mut self, group: EditGroup) -> Option<Pending> {
        let state = self.state(group);
        let pending = if state.status == EditStatus::RefreshPending {
            Pending {
                group,
                generation: state.generation,
                operation: Operation::Refresh,
            }
        } else {
            self.failed.get(&group)?.clone()
        };
        Some(self.start(pending))
    }

    /// Saves and recalculates. A result older than its group's newest draft leaves the group's
    /// state alone.
    pub fn run(&mut self, pending: Pending, port: &mut impl SettingsPort) {
        let refreshed = match &pending.operation {
            Operation::Save(edit) => self.update(edit, port),
            Operation::Refresh => {
                self.refreshed = port.refresh();
                Some(self.refreshed)
            }
        };
        match refreshed {
            Some(true) => self.complete(&pending, EditStatus::Saved),
            Some(false) => self.complete(&pending, EditStatus::RefreshPending),
            None => self.fail(pending, EditError::SaveFailed),
        }
    }

    /// Saves a start-with-Windows change once Windows has `applied` it, as .NET's
    /// `SettingsWindow.OnStartupChanged` does; else fails it, to retry.
    pub fn finish_startup(
        &mut self,
        pending: Pending,
        applied: bool,
        port: &mut impl SettingsPort,
    ) {
        if applied {
            self.run(pending, port);
        } else {
            self.fail(pending, EditError::StartupChangeFailed);
        }
    }

    /// Forgets the invalid drafts and the failed saves.
    pub fn discard_unsaved(&mut self) {
        self.failed.clear();
        for state in self.states.values_mut() {
            if matches!(state.status, EditStatus::Invalid | EditStatus::SaveFailed) {
                state.status = EditStatus::Saved;
                state.error = None;
            }
        }
    }

    /// Whether the day was recalculated, or `None` when the save failed.
    fn update(&mut self, edit: &SettingsEdit, port: &mut impl SettingsPort) -> Option<bool> {
        let settings = edit.apply(&self.current);
        if !settings.validate().is_empty() {
            return None;
        }
        if settings == self.current {
            return Some(self.refreshed);
        }
        port.save(&settings).ok()?;
        self.current = settings;
        self.refreshed = port.refresh();
        Some(self.refreshed)
    }

    fn next_generation(&self, group: EditGroup) -> u64 {
        self.state(group).generation + 1
    }

    fn invalid(&mut self, group: EditGroup, generation: u64, error: EditError) {
        self.failed.remove(&group);
        self.states.insert(
            group,
            GroupState {
                generation,
                status: EditStatus::Invalid,
                error: Some(error),
            },
        );
    }

    fn start(&mut self, pending: Pending) -> Pending {
        self.failed.remove(&pending.group);
        self.states.insert(
            pending.group,
            GroupState {
                generation: pending.generation,
                status: EditStatus::Saving,
                error: None,
            },
        );
        pending
    }

    fn complete(&mut self, pending: &Pending, status: EditStatus) {
        // A recalculation covers every group saved before it.
        if status == EditStatus::Saved {
            for state in self.states.values_mut() {
                if state.status == EditStatus::RefreshPending {
                    state.status = EditStatus::Saved;
                }
            }
        }
        if let Some(state) = self.states.get_mut(&pending.group)
            && state.generation == pending.generation
        {
            self.failed.remove(&pending.group);
            state.status = status;
            state.error = None;
        }
    }

    fn fail(&mut self, pending: Pending, error: EditError) {
        let Some(state) = self.states.get_mut(&pending.group) else {
            return;
        };
        if state.generation != pending.generation {
            return;
        }
        state.status = EditStatus::SaveFailed;
        state.error = Some(error);
        self.failed.insert(pending.group, pending);
    }
}

#[cfg(test)]
mod tests;
