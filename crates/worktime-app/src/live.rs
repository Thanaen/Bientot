//! The day on the user's data, as `App.xaml.cs` (`InitializeAsync`) composes it: the settings,
//! the diagnostic log under `<data>/logs`, the migrated ledger, the built-in source behind the
//! source router, the weekly recalculation and the tracking commands. Everything here runs on
//! the thread that owns the ledger connection; the Windows notifications reach it as calls.

use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::path::Path;

use anyhow::Context as _;
use jiff::Timestamp;
use jiff::civil::Date;
use rusqlite::Connection;
use worktime_core::accounting::WorkdayAccounting;
use worktime_core::corrections::ManualAdjustmentCommands;
use worktime_core::diagnostics::{
    DiagnosticLogLevel, DiagnosticLogOptions, DiagnosticValue, LocalDiagnosticLogger,
};
use worktime_core::notifications::{NotificationCoordinator, NotificationSink};
use worktime_core::refresh::RefreshCoordinator;
use worktime_core::reviews::{
    FinishLongLockReviewCommand, LongLockReviewAdjustmentCommand, LongLockReviews,
    ManualResolution, ReviewError, adjustment_id_for, resolution_interval, resolution_of,
};
use worktime_core::routing::{self, ActivitySourceRouter, ActivitySourceSwitch, SwitchError};
use worktime_core::sources::{ActivityReadResult, ActivitySource};
use worktime_core::tracking::TrackingControl;
use worktime_core::weekly::{WeeklyRecalculation, WorkdayRefreshState};
use worktime_domain::accounting::{AdjustmentId, OperationKey, SessionState, TrackingState};
use worktime_domain::adjustments::ManualAdjustment;
use worktime_domain::configuration::{AppointmentProposal, WorkTimeSettings};
use worktime_domain::history::DailySummarySnapshot;
use worktime_domain::intervals::UtcInterval;
use worktime_domain::sources::ActivitySourceKind;
use worktime_domain::summaries::DailySummary;
use worktime_domain::time::{Clock, LocalWorkday, TimeZoneProvider, WorkdayCalendar};
use worktime_store::activity::{
    ActivityError, ActivitySourceSelectionRepository, ActivitySpanRepository,
};
use worktime_store::adjustments::ManualAdjustmentRepository;
use worktime_store::app_state::{AppStateStore, ClosedCallouts};
use worktime_store::credit::WeeklyCreditRepository;
use worktime_store::data_directory::DataDirectory;
use worktime_store::history::DailySummaryRepository;
use worktime_store::notifications::NotificationReceiptRepository;
use worktime_store::reviews::LongLockResolutionRepository;
use worktime_store::sessions::SessionEventRepository;
use worktime_store::settings::{LoadedSettings, SaveSettingsError, SettingsSource, SettingsStore};
use worktime_windows::PowerState;
use worktime_windows::activity::{BuiltInActivitySource, LastInputProbe};
use worktime_windows::lifecycle::{LifecycleEventRecorder, LifecycleHooks, LifecycleTransition};
use worktime_windows::power_history::{PowerHistoryRead, PowerHistoryRecovery};

use crate::automatic::{self, CountedAbsence, TrustOffer};
use crate::ledger;
use crate::strings::{Key, Language};
use crate::tray_model::{ClosedReview, ReviewAction, ReviewCommand};
use crate::week::{WeekCommand, WeekTimeline};
use crate::widgets::Notice;
use crate::workday::{DayView, TrackingStatus};

/// What went wrong while WorkTime started, which the first flyout tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Announcement {
    /// Invalid settings were set aside for the defaults.
    SettingsRecovered,
    /// Windows refused to register WorkTime's toasts.
    NotificationsRefused,
    /// The import chosen before the restart replaced the history and settings.
    Imported,
    /// The import chosen before the restart could not be applied: nothing changed.
    ImportFailed,
}

impl Announcement {
    pub fn notice(self, language: Language) -> Notice {
        match self {
            Self::SettingsRecovered => {
                Notice::settings_recovered(language.text(Key::SettingsRecovered))
            }
            Self::NotificationsRefused => {
                Notice::warning(language.text(Key::NotificationRegistrationFailed))
            }
            Self::Imported => Notice::success(language.text(Key::TransferImported)),
            Self::ImportFailed => Notice::error(language.text(Key::TransferImportFailed)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackingCommand {
    Pause,
    Resume,
    Finish,
}

impl TrackingCommand {
    /// The pause toggle on a day in `state`: a paused or finished day resumes, as `CanResume`
    /// of `TrayCommandViewModel` says.
    pub fn toggling(state: TrackingState) -> Self {
        match state {
            TrackingState::Running => Self::Pause,
            TrackingState::Paused | TrackingState::Finished => Self::Resume,
        }
    }

    /// The state the command leaves the day in.
    pub fn outcome(self) -> TrackingState {
        match self {
            Self::Pause => TrackingState::Paused,
            Self::Resume => TrackingState::Running,
            Self::Finish => TrackingState::Finished,
        }
    }

    /// The command's word in the operation key, as `TrayOperationKeyFactory` writes it.
    fn name(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Finish => "finish",
        }
    }
}

/// Today's tracking as the shell offers it: its state, and whether today is a working day by
/// the calendar, as `App.xaml.cs` reads it at every refresh whatever the source publishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tracking {
    state: TrackingState,
    working_day: bool,
}

impl Tracking {
    pub fn new(state: TrackingState, working_day: bool) -> Self {
        Self { state, working_day }
    }

    pub fn state(self) -> TrackingState {
        self.state
    }

    pub fn is_working_day(self) -> bool {
        self.working_day
    }

    /// Whether the pause toggle can run.
    pub fn can_toggle(self) -> bool {
        self.can(TrackingCommand::toggling(self.state))
    }

    /// Whether « Terminer » can run.
    pub fn can_finish(self) -> bool {
        self.can(TrackingCommand::Finish)
    }

    /// Whether « Corriger le temps… » can correct today, as `CanAddTime` of
    /// `TrayCommandViewModel` says: only on a working day.
    pub fn can_correct_time(self) -> bool {
        self.working_day
    }

    /// Only on a working day, as `MainWindow.UpdateCommands` of the .NET app enables Pause,
    /// Resume and Finish, and only when it changes the state, as `CanExecute` of
    /// `TrayCommandViewModel` says: a finished day cannot finish again.
    fn can(self, command: TrackingCommand) -> bool {
        self.working_day && command.outcome() != self.state
    }

    /// What a status bar says about it, as `TrackingText` of the .NET app.
    pub fn status(self) -> TrackingStatus {
        if self.working_day {
            TrackingStatus::Tracking(self.state)
        } else {
            TrackingStatus::DayOff
        }
    }
}

pub fn load_settings(directory: &DataDirectory) -> anyhow::Result<LoadedSettings> {
    let store = SettingsStore::new(directory.settings_file())?;
    store
        .load()
        .with_context(|| format!("cannot read {}", store.path().display()))
}

/// Opens the ledger of `directory`, the connection [`LiveHost::start`] takes.
pub fn open_ledger(directory: &DataDirectory) -> anyhow::Result<Connection> {
    let path = directory.database_file();
    worktime_store::database::open(&path)
        .with_context(|| format!("cannot open the ledger {}", path.display()))
}

pub struct LiveHost<'a> {
    directory: DataDirectory,
    connection: &'a Connection,
    clock: &'a dyn Clock,
    zones: &'a dyn TimeZoneProvider,
    settings: WorkTimeSettings,
    store: SettingsStore,
    app_state: AppStateStore,
    built_in: BuiltInActivitySource<'a>,
    logger: LocalDiagnosticLogger<'a>,
    coordinator: RefreshCoordinator<WorkdayRefreshState>,
    tracking: TrackingState,
    /// The latest adjustment of the published working day, which the tray offers to edit.
    latest_adjustment: Option<ManualAdjustment>,
    /// The last refresh recalculated the week: the stored week shows all recorded before it.
    recalculated: bool,
    /// `None` until Windows accepted to show WorkTime's toasts.
    notifications: Option<NotificationCoordinator<'a>>,
    notification_sink: Option<&'a dyn NotificationSink>,
    /// What went wrong while WorkTime started, for the first flyout to tell, in that order.
    announcements: Vec<Announcement>,
    /// The last refresh failed, see [`Self::refresh`].
    refresh_failed: bool,
    /// The local day the week was last recalculated on. Its publication is that day's summary,
    /// or the summary kept beside a source failure.
    recalculated_on: Option<Date>,
    /// The automatic counts and the appointments whose callout the user closed, for the rest
    /// of the session and, through `app-state.json`, of the day.
    hidden_counts: HashSet<AdjustmentId>,
    hidden_offers: HashSet<String>,
    /// The automatic counts whose save failed, not retried before WorkTime starts again.
    failed_counts: HashSet<AdjustmentId>,
    /// The earlier days of the week read for automatic counts, by the stored summary they had.
    scanned_days: HashMap<Date, String>,
    gains: ledger::GainsCache,
}

/// What a catch-up compares to tell whether the day shown changed.
#[derive(Clone, PartialEq, Eq)]
struct Snapshot {
    tracking: TrackingState,
    recalculated_on: Option<Date>,
    refresh_failed: bool,
    /// What the last publication shows, so that publishing the same summary again is no change.
    published: Option<WorkdayRefreshState>,
    recalculated: bool,
    latest_adjustment: Option<ManualAdjustment>,
}

/// The built-in source as the router reads it.
struct BuiltInSource<'s, 'a>(&'s BuiltInActivitySource<'a>);

impl ActivitySource for BuiltInSource<'_, '_> {
    fn read(&self, interval: UtcInterval) -> Result<ActivityReadResult, ActivityError> {
        let snapshot = self.0.read(interval)?;
        Ok(ActivityReadResult::Snapshot {
            coverage: snapshot.coverage,
            health: snapshot.health,
        })
    }
}

/// What a recorded lifecycle change asks of the host.
struct Hooks<'h, 'a> {
    built_in: &'h mut BuiltInActivitySource<'a>,
    settings: &'h WorkTimeSettings,
    logger: &'h mut LocalDiagnosticLogger<'a>,
    refresh: bool,
}

impl LifecycleHooks for Hooks<'_, '_> {
    fn power_changing(&mut self, state: PowerState) {
        if let Err(error) = self.built_in.observe_power(state, self.settings) {
            log_failure(self.logger, "activity_sampling.failed", &error);
        }
    }

    fn refresh(&mut self) {
        self.refresh = true;
    }
}

impl<'a> LiveHost<'a> {
    /// Opens the day on `connection`, the ledger of `directory`. Settings never saved stand for
    /// the onboarding of R6 slice 8, which selects the built-in source: it is selected without
    /// saving them, so that the onboarding still shows then.
    pub fn start(
        directory: &DataDirectory,
        loaded: LoadedSettings,
        connection: &'a Connection,
        clock: &'a dyn Clock,
        zones: &'a dyn TimeZoneProvider,
        input: &'a dyn LastInputProbe,
    ) -> anyhow::Result<Self> {
        let logger = LocalDiagnosticLogger::new(
            directory.log_directory(),
            DiagnosticLogOptions::from_settings(loaded.settings.logging).unwrap_or_default(),
            clock,
        );
        worktime_store::database::migrate(connection)?;
        let announcements = match loaded.source {
            SettingsSource::Recovered { .. } => {
                vec![Announcement::SettingsRecovered]
            }
            SettingsSource::File | SettingsSource::Missing => Vec::new(),
        };
        let mut settings = loaded.settings;
        if loaded.source == SettingsSource::Missing {
            settings
                .activity_source
                .get_or_insert(ActivitySourceKind::BuiltIn);
        }
        let store = SettingsStore::new(directory.settings_file())?;
        let settings = routing::record_startup_selection(
            settings,
            &ActivitySourceSelectionRepository::new(connection, clock),
            &store,
            clock,
        )?;
        let mut host = Self {
            directory: directory.clone(),
            connection,
            clock,
            zones,
            settings,
            store,
            app_state: AppStateStore::new(directory.app_state_file())?,
            built_in: BuiltInActivitySource::new(
                input,
                ActivitySpanRepository::new(connection, clock),
                clock,
            ),
            logger,
            coordinator: RefreshCoordinator::new(),
            tracking: TrackingState::Running,
            latest_adjustment: None,
            recalculated: false,
            notifications: None,
            notification_sink: None,
            announcements,
            refresh_failed: false,
            recalculated_on: None,
            hidden_counts: HashSet::new(),
            hidden_offers: HashSet::new(),
            failed_counts: HashSet::new(),
            scanned_days: HashMap::new(),
            gains: ledger::GainsCache::default(),
        };
        host.tracking = host.with_tracking(|tracking| tracking.reconcile_current_state())?;
        host.restore_closed_callouts();
        Ok(host)
    }

    /// The callouts the user closed earlier today, before WorkTime started again.
    fn restore_closed_callouts(&mut self) {
        let state = match self.app_state.load() {
            Ok(state) => state,
            Err(error) => {
                log::warn!("cannot read the app state: {error}");
                return;
            }
        };
        let (counts, offers) = state.closed_callouts.on(self.today());
        self.hidden_counts
            .extend(counts.iter().filter_map(|id| AdjustmentId::new(id).ok()));
        self.hidden_offers.extend(offers.iter().cloned());
    }

    /// Keeps the closed callouts in `app-state.json` for the rest of the day.
    fn save_closed_callouts(&mut self) {
        let mut counts: Vec<_> = self
            .hidden_counts
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        let mut offers: Vec<_> = self.hidden_offers.iter().cloned().collect();
        counts.sort();
        offers.sort();
        let closed = ClosedCallouts {
            date: Some(self.today()),
            counts,
            offers,
        };
        if let Err(error) = self
            .app_state
            .update(|state| state.closed_callouts = closed)
        {
            log_failure(&mut self.logger, "app_state.save_failed", &error);
        }
    }

    pub fn settings(&self) -> &WorkTimeSettings {
        &self.settings
    }

    pub fn settings_file(&self) -> &Path {
        self.store.path()
    }

    /// The folder of the ledger and settings, which « Données » exports and imports into.
    pub fn data_directory(&self) -> &DataDirectory {
        &self.directory
    }

    /// Has the first flyout tell `announcement` after those of the start.
    pub fn announce(&mut self, announcement: Announcement) {
        self.announcements.push(announcement);
    }

    /// The local date of the last usage signal sent, `None` before the first one or while
    /// `app-state.json` cannot be read.
    pub fn usage_ping_last_sent(&self) -> Option<Date> {
        match self.app_state.load() {
            Ok(state) => state.usage_ping_last_sent,
            Err(error) => {
                log::warn!("cannot read the app state: {error}");
                None
            }
        }
    }

    /// Where the date of the last usage signal sent is kept.
    pub fn app_state(&self) -> &AppStateStore {
        &self.app_state
    }

    pub fn today(&self) -> Date {
        self.clock.now().to_zoned(self.zones.local().1).date()
    }

    /// How long until the next midnight, where the next workday starts and a catch-up resets a
    /// finished day. `None` when the local zone does not resolve, or resolves that midnight no
    /// later than now.
    pub fn until_rollover(&self) -> Option<std::time::Duration> {
        let calendar = WorkdayCalendar::new(self.zones);
        let now = self.clock.now();
        let window = calendar.window(&calendar.workday(now)).ok()?;
        std::time::Duration::try_from(now.duration_until(window.end))
            .ok()
            .filter(|left| !left.is_zero())
    }

    /// Writes `settings` to `settings.json` and works on them from now on. Another activity
    /// source is recorded in the selection ledger from now, as the settings save it. The caller
    /// refreshes, so that the day shows them.
    pub fn save_settings(&mut self, settings: &WorkTimeSettings) -> Result<(), SaveSettingsError> {
        match settings.activity_source {
            Some(source) if settings.activity_source != self.settings.activity_source => {
                ActivitySourceSwitch::new(
                    ActivitySourceSelectionRepository::new(self.connection, self.clock),
                    self.clock,
                )
                .select(
                    &self.settings,
                    source,
                    |_| settings.clone(),
                    |next| self.store.save(&next),
                )
                .map_err(|error| match error {
                    SwitchError::Save(error) => error,
                    SwitchError::InvalidSettings(errors) => SaveSettingsError::Invalid(errors),
                    SwitchError::Selections(error) => {
                        SaveSettingsError::Io(std::io::Error::other(error))
                    }
                })?;
            }
            _ => self.store.save(settings)?,
        }
        self.settings = settings.clone();
        // Turning an appointment automatic counts the absences already in its week.
        self.scanned_days.clear();
        // The reminders read the settings they were enabled with.
        if let Some(sink) = self.notification_sink {
            self.enable_notifications(sink);
        }
        Ok(())
    }

    pub fn tracking(&self) -> TrackingState {
        self.tracking
    }

    pub fn log(
        &mut self,
        level: DiagnosticLogLevel,
        event: &str,
        properties: &[(&str, DiagnosticValue)],
    ) {
        if let Err(error) = self.logger.write(level, event, properties) {
            log::warn!("cannot write the diagnostic log: {error}");
        }
    }

    /// Logs `event` with the error's type and message, as `FailureProperties` does.
    pub fn log_failure<E: Display>(&mut self, event: &str, error: &E) {
        log_failure(&mut self.logger, event, error);
    }

    /// The host refreshes after every command itself, so the control asks for nothing.
    fn with_tracking<T, E>(
        &self,
        command: impl FnOnce(&TrackingControl) -> Result<T, E>,
    ) -> Result<T, E> {
        command(&TrackingControl::new(
            SessionEventRepository::new(self.connection, self.clock),
            self.clock,
            self.zones,
            &|| {},
        ))
    }

    /// Records the power history of the current workday read by `read`, which runs on a
    /// thread of its own and is given up after [`PowerHistoryRecovery::READ_TIMEOUT`].
    pub fn recover_power_history(
        &mut self,
        read: impl FnOnce(UtcInterval) -> PowerHistoryRead + Send + 'static,
    ) -> anyhow::Result<()> {
        let calendar = WorkdayCalendar::new(self.zones);
        let window = calendar.window(&calendar.workday(self.clock.now()))?;
        let history = PowerHistoryRecovery::new(
            SessionEventRepository::new(self.connection, self.clock),
            PowerHistoryRecovery::READ_TIMEOUT,
        )
        .recover(&window, read)?;
        if let Some(reason) = history.reason() {
            let status = match history {
                PowerHistoryRead::Truncated => "Truncated",
                _ => "Unavailable",
            };
            self.log(
                DiagnosticLogLevel::Warning,
                "power_history.recovery_unavailable",
                &[("status", status.into()), ("reason", reason.into())],
            );
        }
        Ok(())
    }

    /// Records the session state at startup and the power as active, then the session and power
    /// changes that arrived while WorkTime started, in the order they happened. The state
    /// `probe` reads, `None` when it gave up, is stamped with the instant before it runs, as
    /// `PrepareAsync` does. `arrived` is read only once the probe returned, so that it also
    /// yields the changes that came in during it.
    pub fn prepare_lifecycle(
        &mut self,
        probe: impl FnOnce() -> Option<SessionState>,
        arrived: impl Iterator<Item = LifecycleTransition>,
    ) {
        let probed_at = self.clock.now();
        let (zone_id, _) = self.zones.local();
        let session = probe();
        let mut hooks = Hooks {
            built_in: &mut self.built_in,
            settings: &self.settings,
            logger: &mut self.logger,
            refresh: false,
        };
        let prepared = session.is_some_and(|session| {
            LifecycleEventRecorder::new(SessionEventRepository::new(self.connection, self.clock))
                .prepare(session, probed_at, &zone_id, &mut hooks)
                .is_ok()
        });
        if !prepared {
            self.log(
                DiagnosticLogLevel::Warning,
                "lifecycle.current_probe_unavailable",
                &[],
            );
        }
        let mut arrived: Vec<_> = arrived.collect();
        arrived.sort_by_key(|transition| transition.occurred_at);
        for transition in &arrived {
            self.record(transition);
        }
    }

    /// Records a lock, unlock, suspend or resume, then refreshes when the ledger changed.
    pub fn record_lifecycle(&mut self, transition: &LifecycleTransition) {
        if self.record(transition) {
            self.refresh();
        }
    }

    /// Records `transition`; true when it is new to the ledger.
    fn record(&mut self, transition: &LifecycleTransition) -> bool {
        let mut hooks = Hooks {
            built_in: &mut self.built_in,
            settings: &self.settings,
            logger: &mut self.logger,
            refresh: false,
        };
        let recorded =
            LifecycleEventRecorder::new(SessionEventRepository::new(self.connection, self.clock))
                .record(transition, &mut hooks);
        if let Err(error) = recorded {
            log::error!("cannot record a session or power change: {error}");
        }
        hooks.refresh
    }

    /// The tick of the built-in source, every `IdleStateTracker::DEFAULT_POLL_INTERVAL`.
    pub fn poll(&mut self) {
        if let Err(error) = self.built_in.poll(&self.settings) {
            log_failure(&mut self.logger, "activity_sampling.failed", &error);
        }
    }

    /// Stores the settled boundary of the open activity span before the app quits.
    pub fn shut_down(&mut self) {
        if let Err(error) = self.built_in.end_observation() {
            log_failure(&mut self.logger, "activity_sampling.failed", &error);
        }
    }

    /// The periodic refresh: resets a finished day the clock has outlived, then recalculates
    /// the week and publishes today. A failure keeps the last publication, and the flyout tells
    /// it until a refresh succeeds.
    pub fn refresh(&mut self) {
        self.refresh_as("periodic_refresh.failed");
    }

    /// Reads the tracking state in effect now, resetting a finished day the clock has outlived.
    /// Then recalculates the week and publishes today only when:
    /// - the tracking state changed,
    /// - the week was last recalculated before today, or
    /// - the last refresh failed, so that every catch-up recalculates while it fails and notices
    ///   when the failure ended.
    ///
    /// A source that keeps failing the same day is read again by the periodic refresh only.
    /// A tracking state that cannot be read keeps the last publication and shows a failed
    /// refresh. True when what the day shows changed, as `Snapshot` lists it.
    pub fn catch_up(&mut self) -> bool {
        const FAILURE_EVENT: &str = "tracking_refresh.failed";
        let before = self.snapshot();
        if !self.reconcile(FAILURE_EVENT) {
            self.refresh_failed = true;
        } else {
            let needs_recalculation = self.tracking != before.tracking
                || before.recalculated_on != Some(self.today())
                || before.refresh_failed;
            if needs_recalculation {
                self.recalculate(FAILURE_EVENT, false);
            }
        }
        self.snapshot() != before
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            tracking: self.tracking,
            recalculated_on: self.recalculated_on,
            refresh_failed: self.refresh_failed,
            published: self
                .coordinator
                .current()
                .map(|publication| publication.summary.clone()),
            recalculated: self.recalculated,
            latest_adjustment: self.latest_adjustment.clone(),
        }
    }

    fn refresh_as(&mut self, failure_event: &str) {
        let reconciled = self.reconcile(failure_event);
        self.recalculate(failure_event, !reconciled);
    }

    /// Stores the tracking state in effect now, resetting a finished day the clock has outlived.
    /// False when it could not be read.
    fn reconcile(&mut self, failure_event: &str) -> bool {
        match self.with_tracking(|tracking| tracking.reconcile_current_state()) {
            Ok(state) => {
                self.tracking = state;
                true
            }
            Err(error) => {
                log_failure(&mut self.logger, failure_event, &error);
                false
            }
        }
    }

    /// Recalculates the week and publishes today, then again once the absences of automatic
    /// appointments are counted. `failed` carries a failure of the step before.
    fn recalculate(&mut self, failure_event: &str, failed: bool) {
        self.recalculate_once(failure_event, failed);
        if !self.refresh_failed && self.count_automatically() {
            self.recalculate_once(failure_event, false);
        }
    }

    /// « Compter automatiquement »: records the counts of the published day and of the earlier
    /// days of its week, where an absence that ended after its day or while WorkTime was not
    /// running waits, see [`automatic::counts`]. True when one was recorded.
    fn count_automatically(&mut self) -> bool {
        let mut counts = match self.published_summary() {
            Some(summary) if summary.workday().is_working_day() => {
                match ManualAdjustmentRepository::new(self.connection)
                    .read_for_workday(summary.workday())
                {
                    Ok(recorded) => automatic::counts(
                        summary.workday(),
                        summary.accounting(),
                        summary.computed_at(),
                        &self.settings.recurring_appointments,
                        &recorded,
                    ),
                    Err(error) => {
                        log_failure(&mut self.logger, "automatic_count.failed", &error);
                        Vec::new()
                    }
                }
            }
            _ => Vec::new(),
        };
        counts.extend(self.earlier_counts());
        let mut recorded = false;
        for count in counts {
            if self.failed_counts.contains(count.id()) {
                continue;
            }
            if self.save_adjustment(&count).is_ok() {
                recorded = true;
            } else {
                self.failed_counts.insert(count.id().clone());
            }
        }
        recorded
    }

    /// The counts of the days of the week before the published one. A day is read again only
    /// once its stored summary changed, or the appointments did.
    fn earlier_counts(&mut self) -> Vec<ManualAdjustment> {
        let appointments = &self.settings.recurring_appointments;
        let Some(today) = self
            .published_summary()
            .map(|summary| summary.workday().date)
        else {
            return Vec::new();
        };
        if !appointments.iter().any(|appointment| appointment.automatic) {
            return Vec::new();
        }
        let revisions = match DailySummaryRepository::new(self.connection).read_latest_week(today) {
            Ok(revisions) => revisions,
            Err(error) => {
                log_failure(&mut self.logger, "automatic_count.failed", &error);
                return Vec::new();
            }
        };
        let read = |snapshot: &DailySummarySnapshot| {
            self.with_accounting(|accounting| {
                let context = accounting.capture_context();
                let calculation = accounting.calculate(snapshot.workday().date, &context)?;
                let current = calculation.summary_input.accounting();
                if current.timeline().window().workday != *snapshot.workday() {
                    return Ok(Vec::new());
                }
                let recorded = ManualAdjustmentRepository::new(self.connection)
                    .read_for_workday(snapshot.workday())?;
                anyhow::Ok(automatic::counts(
                    snapshot.workday(),
                    current,
                    context.observed_at,
                    appointments,
                    &recorded,
                ))
            })
        };
        let mut counts = Vec::new();
        let mut scanned = HashMap::new();
        let mut failures = Vec::new();
        for revision in &revisions {
            let snapshot = revision.snapshot();
            let date = snapshot.workday().date;
            if date >= today || !snapshot.workday().is_working_day() {
                continue;
            }
            let fingerprint = snapshot.input_fingerprint();
            if !snapshot.has_pending_review()
                || self.scanned_days.get(&date).map(String::as_str) == Some(fingerprint)
            {
                scanned.insert(date, fingerprint.to_owned());
                continue;
            }
            match read(snapshot) {
                Ok(found) => {
                    counts.extend(found);
                    scanned.insert(date, fingerprint.to_owned());
                }
                Err(error) => failures.push(error),
            }
        }
        for error in failures {
            log_failure(&mut self.logger, "automatic_count.failed", &error);
        }
        self.scanned_days = scanned;
        counts
    }

    fn recalculate_once(&mut self, failure_event: &str, mut failed: bool) {
        self.recalculated_on = Some(self.today());
        let settings = self.settings.clone();
        let read_settings = move || settings.clone();
        let built_in = BuiltInSource(&self.built_in);
        let router = ActivitySourceRouter::new(
            ActivitySourceSelectionRepository::new(self.connection, self.clock),
            &built_in,
        );
        let accounting = WorkdayAccounting::new(
            &router,
            SessionEventRepository::new(self.connection, self.clock),
            ManualAdjustmentRepository::new(self.connection),
            self.clock,
            self.zones,
            &read_settings,
        );
        let store = WeeklyCreditRepository::new(self.connection);
        let weekly = WeeklyRecalculation::new(&accounting, &store);
        // A source failure publishes the previous summary without storing the week again.
        self.recalculated = match self
            .coordinator
            .refresh(|previous| weekly.refresh(previous))
        {
            Ok(publication) => publication.summary.source_failure.is_none(),
            Err(error) => {
                failed = true;
                log_failure(&mut self.logger, failure_event, &error);
                false
            }
        };

        let workday = self
            .published_summary()
            .map(|summary| summary.workday().clone())
            .filter(|workday| workday.is_working_day());
        let latest = workday.map_or(Ok(None), |workday| {
            ManualAdjustmentCommands::new(
                &accounting,
                ManualAdjustmentRepository::new(self.connection),
            )
            .latest_active(&workday)
        });
        self.latest_adjustment = latest.unwrap_or_else(|error| {
            failed = true;
            log_failure(&mut self.logger, failure_event, &error);
            None
        });
        self.refresh_failed = failed;

        if let (Some(notifications), Some(publication)) =
            (&mut self.notifications, self.coordinator.current())
            && let Err(error) = notifications.observe(publication)
        {
            log_failure(&mut self.logger, "notification.failed", &error);
        }
    }

    /// Shows a toast through `sink` when a reminder of `settings.notifications` falls due.
    pub fn enable_notifications(&mut self, sink: &'a dyn NotificationSink) {
        self.notification_sink = Some(sink);
        let settings = self.settings.clone();
        self.notifications = Some(NotificationCoordinator::new(
            NotificationReceiptRepository::new(self.connection),
            sink,
            self.zones,
            Box::new(move || settings.clone()),
        ));
    }

    /// Windows refused to register WorkTime's toasts, as `error` says: logs it, and the first
    /// flyout tells it.
    pub fn notifications_refused<E: Display>(&mut self, error: &E) {
        log_failure(&mut self.logger, "notification.registration_failed", error);
        self.announcements.push(Announcement::NotificationsRefused);
    }

    /// What went wrong while WorkTime started: invalid settings restored to their defaults,
    /// then Windows notifications unavailable.
    pub fn announcements(&self) -> &[Announcement] {
        &self.announcements
    }

    pub fn refresh_failed(&self) -> bool {
        self.refresh_failed
    }

    /// The summary last published, kept beside a source failure or not.
    fn published_summary(&self) -> Option<&DailySummary> {
        self.coordinator
            .current()
            .and_then(|publication| publication.summary.summary.as_ref())
    }

    /// The long locks of the published day that ended without the user saying what they were.
    pub fn closed_reviews(&self) -> Vec<ClosedReview> {
        let Some(summary) = self.published_summary() else {
            return Vec::new();
        };
        let workday = summary.workday();
        let zone = self
            .zones
            .get(&workday.time_zone_id)
            .unwrap_or_else(|_| self.zones.local().1);
        let local = |instant: jiff::Timestamp| instant.to_zoned(zone.clone()).time();
        let current = summary.accounting();
        current
            .pending_reviews()
            .iter()
            .filter(|review| !review.is_open)
            .map(|review| {
                let shown = resolution_interval(current, review.locked_episode)
                    .unwrap_or(review.locked_episode);
                ClosedReview {
                    workday: workday.clone(),
                    id: review.id.clone(),
                    locked_episode: review.locked_episode,
                    local_start: local(shown.start()),
                    local_end: local(shown.end()),
                    finish_at: local(review.locked_episode.start()),
                    appointment: review.appointment_name(&self.settings.recurring_appointments),
                    duration: shown.duration(),
                }
            })
            .collect()
    }

    pub fn latest_adjustment(&self) -> Option<&ManualAdjustment> {
        self.latest_adjustment.as_ref()
    }

    /// The adjustment `id` of `workday` that « Modifier… » of the week opens, unless a later
    /// one replaced it.
    pub fn active_adjustment(
        &mut self,
        workday: &LocalWorkday,
        id: &AdjustmentId,
    ) -> Result<Option<ManualAdjustment>, Key> {
        let result = self.with_accounting(|accounting| {
            ledger::active_adjustment(workday, id, self.connection, accounting)
        });
        result.map_err(|refusal| {
            refusal.logged(|error| log_failure(&mut self.logger, "history_command.failed", error))
        })
    }

    pub fn recalculated(&self) -> bool {
        self.recalculated
    }

    /// Resolves a long lock from the tray menu, then refreshes.
    pub fn resolve_review(&mut self, command: &ReviewCommand) {
        if let Err(error) = self.try_resolve_review(command) {
            log_failure(&mut self.logger, "tray_command.failed", &error);
        }
        self.refresh_as("tracking_refresh.failed");
    }

    /// `DailySummaryTrayCommandBuilder`: the review must still be one of the published day.
    fn try_resolve_review(&self, command: &ReviewCommand) -> Result<(), ReviewError> {
        let review = &command.review;
        let summary = self
            .published_summary()
            .filter(|summary| *summary.workday() == review.workday)
            .ok_or_else(|| ReviewError::LockedEpisodeChanged(review.id.clone()))?;
        let reviews = LongLockReviews::new(
            ManualAdjustmentRepository::new(self.connection),
            SessionEventRepository::new(self.connection, self.clock),
            LongLockResolutionRepository::new(self.connection),
        );
        let adjustment = |resolution| LongLockReviewAdjustmentCommand {
            adjustment_id: adjustment_id_for(&review.workday, &review.id, resolution),
            review_id: review.id.clone(),
            locked_episode: review.locked_episode,
            created_at: summary.computed_at(),
        };
        match command.action {
            ReviewAction::ConfirmWork => reviews
                .confirm_as_work(
                    summary.accounting(),
                    &adjustment(ManualResolution::ConfirmWork),
                )
                .map(drop),
            ReviewAction::Exclude => reviews
                .reclassify_as_not_work(
                    summary.accounting(),
                    &adjustment(ManualResolution::Exclude),
                )
                .map(drop),
            ReviewAction::FinishAtLockStart => reviews
                .finish_at_lock_start(
                    summary.accounting(),
                    &FinishLongLockReviewCommand {
                        review_id: review.id.clone(),
                        locked_episode: review.locked_episode,
                    },
                )
                .map(drop),
        }
    }

    /// The day's accounting on the ledger and the routed source, as a refresh reads it.
    fn with_accounting<R>(&self, run: impl FnOnce(&WorkdayAccounting) -> R) -> R {
        let settings = self.settings.clone();
        let read_settings = move || settings.clone();
        let built_in = BuiltInSource(&self.built_in);
        let router = ActivitySourceRouter::new(
            ActivitySourceSelectionRepository::new(self.connection, self.clock),
            &built_in,
        );
        run(&WorkdayAccounting::new(
            &router,
            SessionEventRepository::new(self.connection, self.clock),
            ManualAdjustmentRepository::new(self.connection),
            self.clock,
            self.zones,
            &read_settings,
        ))
    }

    /// The current week as the last refresh stored it.
    pub fn week(&self) -> anyhow::Result<WeekTimeline> {
        self.with_accounting(|accounting| {
            ledger::read_week(
                self.connection,
                accounting,
                self.clock,
                self.zones,
                &self.gains,
            )
        })
    }

    /// Records a correction of the week. The caller refreshes, so that the week shows it.
    pub fn run_week_command(&mut self, command: &WeekCommand) -> Result<Vec<AdjustmentId>, Key> {
        let result = self.with_accounting(|accounting| {
            ledger::run(command, self.connection, accounting, self.clock)
        });
        result.map_err(|refusal| {
            refusal.logged(|error| log_failure(&mut self.logger, "history_command.failed", error))
        })
    }

    /// The appointment to propose after a correction of `workday`, see [`ledger::proposal`].
    pub fn proposal(&self, workday: &LocalWorkday) -> Option<AppointmentProposal> {
        ledger::proposal(workday, &self.settings, self.connection, self.zones)
    }

    /// Records a correction of « Corriger le temps… ». The caller refreshes, so that the week
    /// and the day show it.
    pub fn save_adjustment(&mut self, adjustment: &ManualAdjustment) -> Result<AdjustmentId, Key> {
        let result = self
            .with_accounting(|accounting| ledger::save(adjustment, self.connection, accounting));
        result.map_err(|refusal| {
            refusal.logged(|error| log_failure(&mut self.logger, "adjustment.failed", error))
        })
    }

    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// The last refresh's publication, as the diagnostic snapshot reads it.
    pub fn refresh_state(&self) -> Option<&WorkdayRefreshState> {
        self.coordinator
            .current()
            .map(|publication| &publication.summary)
    }

    pub fn log_directory(&self) -> &Path {
        self.logger.directory()
    }

    pub fn zones(&self) -> &dyn TimeZoneProvider {
        self.zones
    }

    /// Pauses, resumes or finishes the day now, then refreshes.
    pub fn command(&mut self, command: TrackingCommand) {
        let key = OperationKey::new(format!(
            "tray:{}:{:032x}",
            command.name(),
            crate::random_id()
        ))
        .expect("the key starts with its lane");
        let recorded = self.with_tracking(|tracking| match command {
            TrackingCommand::Pause => tracking.pause(key),
            TrackingCommand::Resume => tracking.resume(key),
            TrackingCommand::Finish => tracking.finish(key),
        });
        if let Err(error) = recorded {
            log_failure(&mut self.logger, "tray_command.failed", &error);
        }
        self.refresh_as("tracking_refresh.failed");
    }

    /// The last publication in `language`; a day without a summary until a refresh succeeded.
    pub fn view(&self, language: Language) -> DayView {
        let unpublished = WorkdayRefreshState {
            summary: None,
            source_failure: None,
        };
        let state = self
            .coordinator
            .current()
            .map_or(&unpublished, |publication| &publication.summary);
        let mut view = DayView::present(state, &self.settings, &self.zones.local().1, language);
        view.appointment_review = self
            .closed_reviews()
            .into_iter()
            .filter(|review| review.appointment.is_some())
            .min_by_key(|review| review.locked_episode.start());
        view.counted_absence = self.counted_absence();
        view.trust_offer = self.trust_offer();
        view
    }

    /// The corrections of the published working day.
    fn recorded_today(&self) -> Option<(&DailySummary, Vec<ManualAdjustment>)> {
        let summary = self
            .published_summary()
            .filter(|summary| summary.workday().is_working_day())?;
        let recorded = ManualAdjustmentRepository::new(self.connection)
            .read_for_workday(summary.workday())
            .ok()?;
        Some((summary, recorded))
    }

    /// The earliest absence of the day counted automatically, until the user closed its callout.
    fn counted_absence(&self) -> Option<CountedAbsence> {
        let (summary, recorded) = self.recorded_today()?;
        let workday = summary.workday();
        let zone = self
            .zones
            .get(&workday.time_zone_id)
            .unwrap_or_else(|_| self.zones.local().1);
        let local = |instant: Timestamp| instant.to_zoned(zone.clone()).time();
        automatic::in_effect(&recorded)
            .into_iter()
            .filter(|count| !self.hidden_counts.contains(count.id()))
            .filter_map(|count| {
                let interval = count.interval()?;
                let review = automatic::counted_review(count.id())?;
                let appointment = self
                    .settings
                    .recurring_appointments
                    .iter()
                    .find(|appointment| Some(appointment.id.as_str()) == review.appointment_id())
                    .map(|appointment| appointment.name.clone())
                    .unwrap_or_default();
                Some(CountedAbsence {
                    workday: workday.clone(),
                    adjustment_id: count.id().clone(),
                    interval,
                    local_start: local(interval.start()),
                    local_end: local(interval.end()),
                    duration: interval.duration(),
                    appointment,
                })
            })
            .min_by_key(|absence| absence.interval.start())
    }

    /// « Compter automatiquement à l’avenir » once the user confirmed an absence of an
    /// appointment today, at least the [`automatic::TRUST_STREAK`]th time in a row.
    fn trust_offer(&self) -> Option<TrustOffer> {
        let (summary, recorded) = self.recorded_today()?;
        recorded.iter().find_map(|adjustment| {
            let (review, resolution) = resolution_of(adjustment.id())?;
            let appointment = self
                .settings
                .recurring_appointments
                .iter()
                .find(|appointment| Some(appointment.id.as_str()) == review.appointment_id())?;
            if resolution != ManualResolution::ConfirmWork
                || !appointment.enabled
                || appointment.automatic
                || self.hidden_offers.contains(&appointment.id)
            {
                return None;
            }
            let streak = automatic::streak(&appointment.id, summary.workday(), self.connection);
            (streak >= automatic::TRUST_STREAK).then(|| TrustOffer {
                appointment_id: appointment.id.clone(),
                appointment: appointment.name.clone(),
                weekday: appointment.weekday,
                streak,
            })
        })
    }

    /// « Ce n’était pas du travail » on an absence counted automatically: excludes it in place
    /// of its count, then refreshes.
    pub fn undo_count(&mut self, id: &AdjustmentId) {
        let undo = self.recorded_today().and_then(|(_, recorded)| {
            automatic::in_effect(&recorded)
                .into_iter()
                .find(|count| count.id() == id)
                .and_then(|count| automatic::undo(count, self.clock.now()))
        });
        if let Some(undo) = undo {
            let _ = self.save_adjustment(&undo);
        }
        self.refresh_as("tracking_refresh.failed");
    }

    /// The cross of the callout of an absence counted automatically.
    pub fn hide_count(&mut self, id: &AdjustmentId) {
        if self.hidden_counts.insert(id.clone()) {
            self.save_closed_callouts();
        }
    }

    /// The cross of « Compter automatiquement à l’avenir », or the offer taken.
    pub fn hide_offer(&mut self, appointment_id: &str) {
        if self.hidden_offers.insert(appointment_id.to_owned()) {
            self.save_closed_callouts();
        }
    }
}

/// `FailureProperties` of `App.xaml.cs`: the error's type and its message, which the log
/// redacts.
fn log_failure<E: Display>(logger: &mut LocalDiagnosticLogger, event: &str, error: &E) {
    let message = error.to_string();
    let written = logger.write(
        DiagnosticLogLevel::Error,
        event,
        &[
            ("exceptionType", std::any::type_name::<E>().into()),
            ("message", message.as_str().into()),
        ],
    );
    if let Err(written) = written {
        log::warn!("cannot write the diagnostic log: {written}");
    }
    log::error!("{event}: {message}");
}

#[cfg(test)]
mod tests;
