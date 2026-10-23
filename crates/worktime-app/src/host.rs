//! The day the shell shows, from a demo or from the user's data, and what keeps it current:
//! the 30 s refresh, the built-in source's sampling, F5 and the tracking actions.

use std::path::Path;
use std::time::Duration;

use gpui::{App, Global};
use jiff::Timestamp;
use jiff::civil::Date;
use worktime_core::diagnostics::{
    DiagnosticInitializationState, DiagnosticRuntimeSnapshot, DiagnosticSnapshot, create_snapshot,
};
use worktime_domain::accounting::{AdjustmentId, TrackingState};
use worktime_domain::adjustments::ManualAdjustment;
use worktime_domain::configuration::{AppointmentProposal, DismissedProposal, WorkTimeSettings};
use worktime_domain::sources::{ActivitySourceKind, IdleStateTracker};
use worktime_domain::time::{LocalWorkday, TimeZoneProvider, WorkdayCalendar};
use worktime_store::data_directory::DataDirectory;
use worktime_store::settings::SaveSettingsError;

use crate::adjust::window::Opener;
use crate::adjust::{Editable, Seed, Target};
use crate::adjust_view;
use crate::demo::DemoHost;
use crate::flyout::{
    AdjustTime, Alerts, CountAutomatically, DeclareWork, ExportDiagnostics, FinishDay, HideCount,
    HideTrustOffer, OpenGeneralSettings, OpenSettings, OpenSettingsJson, OpenWeek, Quit, Refresh,
    ResolveReview, TogglePause, UndoCount, UseBuiltInDetection,
};
use crate::live::{Announcement, LiveHost, Tracking, TrackingCommand};
use crate::settings::Page;
use crate::strings::{Key, Language};
use crate::tray_model::{self, MenuCommand, MenuEntry, MenuState, ReviewCommand};
use crate::week::{Recorded, WeekCommand, WeekTimeline};
use crate::widgets::Notice;
use crate::workday::DayView;
use crate::{settings_view, shell, week_view};

/// The period of `PeriodicRefreshService` in `App.xaml.cs`.
const REFRESH_INTERVAL: Duration = Duration::from_secs(30);

pub enum Source {
    Demo(Box<DemoHost>),
    Live(Box<LiveHost<'static>>),
}

pub struct Today {
    source: Source,
    language: Language,
    view: DayView,
    tracking: TrackingState,
    /// The last refresh recalculated the week, see [`Self::recalculated`].
    recalculated: bool,
    /// What went wrong while WorkTime started, until a flyout told it.
    announcements: Vec<Announcement>,
    refresh_failed: bool,
}

impl Global for Today {}

impl Today {
    /// Refreshes `source` once to have a day to show.
    pub fn open(mut source: Source, language: Language) -> anyhow::Result<Self> {
        let (view, tracking, recalculated, announcements, refresh_failed) = match &mut source {
            Source::Demo(host) => (
                host.refresh(language)?,
                TrackingState::Running,
                true,
                Vec::new(),
                false,
            ),
            Source::Live(host) => {
                host.refresh();
                (
                    host.view(language),
                    host.tracking(),
                    host.recalculated(),
                    host.announcements().to_vec(),
                    host.refresh_failed(),
                )
            }
        };
        Ok(Self {
            source,
            language,
            view,
            tracking,
            recalculated,
            announcements,
            refresh_failed,
        })
    }

    /// What a flyout that opens now tells besides the answer to an action. What went wrong
    /// while WorkTime started is told until a flyout [`announced`](Self::announced) it.
    pub fn opening_alerts(&self) -> Alerts {
        Alerts {
            announcements: self
                .announcements
                .iter()
                .map(|announcement| announcement.notice(self.language))
                .collect(),
            refresh_failure: self.refresh_failure(),
        }
    }

    /// A flyout opened on the [`opening_alerts`](Self::opening_alerts): the next ones tell
    /// nothing of the start.
    pub fn announced(&mut self) {
        self.announcements.clear();
    }

    /// The notice of the last refresh, while it failed.
    pub fn refresh_failure(&self) -> Option<Notice> {
        self.refresh_failed
            .then(|| Notice::refresh_pending(self.language.text(Key::PeriodicRefreshErrorMessage)))
    }

    pub fn view(&self) -> &DayView {
        &self.view
    }

    /// The tracking state, on a working day or not by the calendar now, as `App.xaml.cs`
    /// reads it at every refresh: a published summary can be a day old when the source fails.
    pub fn tracking(&self) -> Tracking {
        Tracking::new(self.tracking, self.workday().is_working_day())
    }

    /// Today's workday by the calendar now.
    fn workday(&self) -> LocalWorkday {
        WorkdayCalendar::new(self.zones()).workday(match &self.source {
            Source::Demo(host) => host.now(),
            Source::Live(host) => host.now(),
        })
    }

    fn zones(&self) -> &dyn TimeZoneProvider {
        match &self.source {
            Source::Demo(host) => host.zones(),
            Source::Live(host) => host.zones(),
        }
    }

    /// The tray menu of the day. A demo has no review nor latest adjustment.
    pub fn menu(&self) -> Vec<MenuEntry> {
        let reviews = match &self.source {
            Source::Live(host) => host.closed_reviews(),
            Source::Demo(_) => Vec::new(),
        };
        tray_model::menu(
            &MenuState {
                tracking: self.tracking(),
                reviews,
                has_latest_adjustment: self.latest_correction().is_some(),
            },
            self.language,
        )
    }

    pub fn live(&mut self) -> Option<&mut LiveHost<'static>> {
        match &mut self.source {
            Source::Live(host) => Some(host.as_mut()),
            Source::Demo(_) => None,
        }
    }

    /// Runs `change` on the user's data, if this is not a demo, then reads the day again.
    pub fn with_live(&mut self, change: impl FnOnce(&mut LiveHost<'static>)) {
        if let Some(host) = self.live() {
            change(host);
        }
        self.read_live();
    }

    pub fn refresh(&mut self) {
        match &mut self.source {
            Source::Demo(host) => {
                let refreshed = host.refresh(self.language);
                self.recalculated = refreshed.is_ok();
                self.refresh_failed = refreshed.is_err();
                match refreshed {
                    Ok(view) => self.view = view,
                    Err(error) => log::error!("cannot refresh the demo day: {error:#}"),
                }
            }
            Source::Live(host) => host.refresh(),
        }
        self.read_live();
    }

    pub fn settings(&self) -> &WorkTimeSettings {
        match &self.source {
            Source::Demo(host) => host.settings(),
            Source::Live(host) => host.settings(),
        }
    }

    /// The `settings.json` in use. A demo keeps its settings in memory.
    pub fn settings_file(&self) -> Option<&Path> {
        match &self.source {
            Source::Demo(_) => None,
            Source::Live(host) => Some(host.settings_file()),
        }
    }

    /// The folder of the user's data. A demo has none to export or import into.
    pub fn data_directory(&self) -> Option<&DataDirectory> {
        match &self.source {
            Source::Demo(_) => None,
            Source::Live(host) => Some(host.data_directory()),
        }
    }

    /// The local date, and that of the last usage signal sent. A demo never sends one.
    pub fn usage_ping_dates(&self) -> (Date, Option<Date>) {
        match &self.source {
            Source::Demo(host) => (host.today(), None),
            Source::Live(host) => (host.today(), host.usage_ping_last_sent()),
        }
    }

    /// The diagnostic snapshot of the day now, as `ExportDiagnosticsAsync` takes it. The shell
    /// installs the day only once it opened, so it is always `Ready`.
    pub fn diagnostic_snapshot(&self, runtime: DiagnosticRuntimeSnapshot) -> DiagnosticSnapshot {
        let (refresh_state, now, today) = match &self.source {
            Source::Demo(host) => (host.refresh_state(), host.now(), host.today()),
            Source::Live(host) => (host.refresh_state(), host.now(), host.today()),
        };
        create_snapshot(
            DiagnosticInitializationState::Ready,
            Some(self.settings()),
            refresh_state,
            Some(today),
            now,
            runtime,
        )
    }

    /// The diagnostic log the export reads. A demo writes none.
    pub fn log_directory(&self) -> Option<&Path> {
        match &self.source {
            Source::Demo(_) => None,
            Source::Live(host) => Some(host.log_directory()),
        }
    }

    /// Saves the settings of « Paramètres ». The caller refreshes, so that the day shows them.
    pub fn save_settings(&mut self, settings: &WorkTimeSettings) -> Result<(), SaveSettingsError> {
        match &mut self.source {
            Source::Demo(host) => host.save_settings(settings),
            Source::Live(host) => host.save_settings(settings),
        }
    }

    /// Saves `at` as the last Store check, as `StoreUpdateService` records it.
    pub fn record_update_check(&mut self, at: Timestamp) -> Result<(), SaveSettingsError> {
        let mut settings = self.settings().clone();
        settings.updates.last_checked_at = Some(at);
        self.save_settings(&settings)
    }

    /// « Utiliser la détection intégrée » while « Paramètres » is closed: saves the built-in
    /// source, then refreshes. A failed save answers the notice the flyout shows, in the words
    /// of the settings page's ActivityWatch callout; the day still offers the switch to retry.
    pub fn use_built_in_detection(&mut self) -> Option<Notice> {
        let settings = WorkTimeSettings {
            activity_source: Some(ActivitySourceKind::BuiltIn),
            ..self.settings().clone()
        };
        let saved = self.save_settings(&settings);
        if let Err(error) = &saved {
            log::error!("cannot switch to the built-in detection: {error}");
        }
        self.refresh();
        saved.err().map(|_| {
            Notice::source_switch_failed(self.language.text(Key::ActivitySourceSwitchFailed))
        })
    }

    /// « Compter automatiquement à l’avenir » while « Paramètres » is closed. False when the
    /// appointment is gone or the save failed, which only logs: the offer then stays.
    pub fn count_automatically(&mut self, appointment_id: &str) -> bool {
        let mut settings = self.settings().clone();
        let Some(appointment) = settings
            .recurring_appointments
            .iter_mut()
            .find(|appointment| appointment.id == appointment_id)
        else {
            return false;
        };
        appointment.automatic = true;
        let saved = self.save_settings(&settings);
        if let Err(error) = &saved {
            log::error!("cannot count an appointment automatically: {error}");
        }
        self.refresh();
        saved.is_ok()
    }

    /// Records a refusal of a proposal; a failed save only logs, as the proposal comes back.
    pub fn dismiss_proposal(&mut self, dismissal: DismissedProposal) {
        let mut settings = self.settings().clone();
        let (today, _) = self.usage_ping_dates();
        settings.appointment_proposals.dismiss(dismissal, today);
        if let Err(error) = self.save_settings(&settings) {
            log::error!("cannot record the refusal of a proposal: {error}");
        }
    }

    /// Whether the last refresh recalculated the week, so that the stored week shows every
    /// correction recorded before it.
    pub fn recalculated(&self) -> bool {
        self.recalculated
    }

    /// The current week, as the last refresh stored it.
    pub fn week(&self) -> anyhow::Result<WeekTimeline> {
        match &self.source {
            Source::Demo(host) => host.week(),
            Source::Live(host) => host.week(),
        }
    }

    /// Records a correction of the week, then refreshes so that the week and the day show it.
    pub fn run_week(&mut self, command: &WeekCommand) -> Result<Recorded, Key> {
        let result = match &mut self.source {
            Source::Demo(host) => host.run_week_command(command),
            Source::Live(host) => host.run_week_command(command),
        };
        self.refresh();
        let proposal = command
            .counted_workday()
            .and_then(|workday| self.proposal(workday));
        result.map(|saved| Recorded {
            saved,
            recalculated: self.recalculated,
            proposal,
        })
    }

    /// « Compter chaque jeudi… » after a correction of `workday` counted time; never in a demo.
    fn proposal(&self, workday: &LocalWorkday) -> Option<AppointmentProposal> {
        match &self.source {
            Source::Live(host) => host.proposal(workday),
            Source::Demo(_) => None,
        }
    }

    /// The day « Corriger le temps… » corrects: `workday` when the week names one, else today
    /// by the calendar, as `AddTimeAsync` of the .NET app opens it. `None` on a day off.
    pub fn adjust_target(&self, workday: Option<LocalWorkday>) -> Option<Target> {
        let workday = workday.unwrap_or_else(|| self.workday());
        let zone = self.zones().get(&workday.time_zone_id).ok()?;
        workday.is_working_day().then_some(Target { workday, zone })
    }

    /// The time a correction opened now is recorded at.
    pub fn adjustment_time(&self) -> Timestamp {
        match &self.source {
            Source::Demo(host) => host.correction_time(),
            Source::Live(host) => host.now(),
        }
    }

    /// The correction `id` of `workday` « Modifier… » of the week opens, refused once another
    /// replaced or withdrew it, and for a long-lock confirmation, which the form cannot replace.
    pub fn editable(&mut self, workday: &LocalWorkday, id: &AdjustmentId) -> Result<Editable, Key> {
        let active = match &mut self.source {
            Source::Demo(host) => host.active_adjustment(workday, id),
            Source::Live(host) => host.active_adjustment(workday, id),
        }?
        .ok_or(Key::HistoryAdjustmentStale)?;
        Editable::new(active).ok_or(Key::HistoryCorrectionFailed)
    }

    /// The latest correction of the published day, unless the form cannot replace it. None in a
    /// demo, whose tray has no latest adjustment, see [`Self::menu`].
    fn latest_correction(&self) -> Option<Editable> {
        let Source::Live(host) = &self.source else {
            return None;
        };
        host.latest_adjustment().cloned().and_then(Editable::new)
    }

    /// « Modifier le dernier ajustement… » of the tray: the latest correction, only on a working
    /// day by the calendar, as `EditTimeAsync` of the .NET app checks `CanAddTime`.
    fn latest_editable(&self) -> Option<Editable> {
        self.tracking()
            .can_correct_time()
            .then(|| self.latest_correction())
            .flatten()
    }

    /// Records a correction of « Corriger le temps… », then refreshes so that the week and the
    /// day show it.
    pub fn save_adjustment(&mut self, adjustment: &ManualAdjustment) -> Result<Recorded, Key> {
        let result = match &mut self.source {
            Source::Demo(host) => host.save_adjustment(adjustment),
            Source::Live(host) => host.save_adjustment(adjustment),
        };
        self.refresh();
        result.map(|saved| Recorded {
            saved: vec![saved],
            recalculated: self.recalculated,
            proposal: None,
        })
    }

    /// Brings the tracking state up to now before anything reads it, see
    /// [`LiveHost::catch_up`]. True when what the day shows changed.
    pub fn catch_up(&mut self) -> bool {
        let caught_up = self.live().is_some_and(|host| host.catch_up());
        if caught_up {
            self.read_live();
        }
        caught_up
    }

    /// Toggles from the state the user saw: the tray menu, the flyout and the palette can show a
    /// day that has since rolled over, and a Resume read as a Pause would pause the new day.
    fn toggle_pause(&mut self) {
        let seen = TrackingCommand::toggling(self.tracking);
        self.catch_up();
        if self.tracking().can_toggle() && TrackingCommand::toggling(self.tracking) == seen {
            self.command(seen);
        }
    }

    fn finish(&mut self) {
        self.catch_up();
        if self.tracking().can_finish() {
            self.command(TrackingCommand::Finish);
        }
    }

    fn resolve_review(&mut self, command: &ReviewCommand) {
        self.with_live(|host| host.resolve_review(command));
    }

    fn undo_count(&mut self, id: &AdjustmentId) {
        self.with_live(|host| host.undo_count(id));
    }

    fn hide_count(&mut self, id: &AdjustmentId) {
        self.with_live(|host| host.hide_count(id));
    }

    fn hide_offer(&mut self, appointment_id: &str) {
        self.with_live(|host| host.hide_offer(appointment_id));
    }

    /// A demo has no ledger to record the command in: it only shows its outcome.
    fn command(&mut self, command: TrackingCommand) {
        match self.live() {
            Some(host) => host.command(command),
            None => self.tracking = command.outcome(),
        }
        self.read_live();
    }

    fn read_live(&mut self) {
        let language = self.language;
        if let Source::Live(host) = &self.source {
            self.view = host.view(language);
            self.tracking = host.tracking();
            self.recalculated = host.recalculated();
            self.refresh_failed = host.refresh_failed();
        }
    }
}

/// Makes `today` the shell's day: the flyout's actions and the timers act on it from now on.
pub fn install(today: Today, cx: &mut App) {
    cx.set_global(today);
    cx.on_action(|_: &Refresh, cx| change(cx, Today::refresh));
    cx.on_action(|_: &TogglePause, cx| change(cx, Today::toggle_pause));
    cx.on_action(|_: &FinishDay, cx| change(cx, Today::finish));
    cx.on_action(|_: &OpenWeek, cx| week_view::open(cx));
    cx.on_action(|_: &AdjustTime, cx| {
        let _ = adjust_view::window::open(None, Seed::Blank, Opener::Flyout, cx);
    });
    cx.on_action(|DeclareWork(unrecorded): &DeclareWork, cx| {
        let seed = Seed::DeclareWork(unrecorded.clone());
        let _ = adjust_view::window::open(None, seed, Opener::Flyout, cx);
    });
    cx.on_action(|ResolveReview(command): &ResolveReview, cx| {
        change(cx, |today| today.resolve_review(command))
    });
    cx.on_action(|UndoCount(id): &UndoCount, cx| change(cx, |today| today.undo_count(id)));
    cx.on_action(|HideCount(id): &HideCount, cx| change(cx, |today| today.hide_count(id)));
    cx.on_action(|CountAutomatically(id): &CountAutomatically, cx| {
        if settings_view::count_automatically(id, cx) {
            change(cx, |today| today.hide_offer(id));
        }
    });
    cx.on_action(|HideTrustOffer(id): &HideTrustOffer, cx| {
        change(cx, |today| today.hide_offer(id))
    });
    cx.on_action(|_: &OpenSettings, cx| settings_view::open(cx));
    cx.on_action(|_: &OpenSettingsJson, cx| settings_view::open_on(Page::Json, cx));
    // The .NET app opens its activity source page; here the detection settings are on « Général ».
    cx.on_action(|_: &OpenGeneralSettings, cx| settings_view::open_on(Page::General, cx));
    cx.on_action(|_: &ExportDiagnostics, cx| crate::diagnostics::export(cx));
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &UseBuiltInDetection, cx| settings_view::use_built_in_detection(cx));
    cx.spawn(async |cx| {
        loop {
            cx.background_executor().timer(REFRESH_INTERVAL).await;
            cx.update(|cx| change(cx, Today::refresh));
        }
    })
    .detach();
    if cx.global_mut::<Today>().live().is_none() {
        return;
    }
    watch_rollover(cx);
    let sampling = IdleStateTracker::DEFAULT_POLL_INTERVAL.unsigned_abs();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(sampling).await;
            cx.update(|cx| {
                if let Some(host) = cx.global_mut::<Today>().live() {
                    host.poll();
                }
            });
        }
    })
    .detach();
    cx.on_app_quit(|cx| {
        if let Some(host) = cx.global_mut::<Today>().live() {
            host.shut_down();
        }
        async {}
    })
    .detach();
}

/// Applies `change` to the day, then shows the result in the flyout and the tray.
pub fn change(cx: &mut App, change: impl FnOnce(&mut Today)) {
    change(cx.global_mut::<Today>());
    publish(cx);
}

/// Catches the day up, see [`Today::catch_up`], and shows it where it changed.
pub fn catch_up(cx: &mut App) {
    if cx.global_mut::<Today>().catch_up() {
        publish(cx);
    }
}

/// Catches the day up at each midnight, so the tray menu offers the new day's commands from the
/// rollover.
fn watch_rollover(cx: &mut App) {
    // `tray-icon` opens the menu built ahead with no hook, so the 30 s refresh could show the old day.
    cx.spawn(async |cx| {
        loop {
            let rollover = cx.update(|cx| {
                cx.global_mut::<Today>()
                    .live()
                    .and_then(|host| host.until_rollover())
            });
            cx.background_executor()
                .timer(rollover.unwrap_or(REFRESH_INTERVAL))
                .await;
            cx.update(catch_up);
        }
    })
    .detach();
}

/// Shows the day in the tray menu, the flyout and the week.
fn publish(cx: &mut App) {
    #[cfg(windows)]
    crate::tray::show(cx);
    let today = cx.global::<Today>();
    let (view, tracking, recalculated) = (today.view.clone(), today.tracking(), today.recalculated);
    let refresh_failure = today.refresh_failure();
    shell::show_day(view, tracking, recalculated, refresh_failure, cx);
    week_view::reload(cx);
}

/// Runs a command of the tray menu. The greyed ones never reach here.
pub fn run(command: &MenuCommand, cx: &mut App) {
    match command {
        MenuCommand::Open => shell::open_flyout(cx),
        MenuCommand::TogglePause => change(cx, Today::toggle_pause),
        MenuCommand::Finish => change(cx, Today::finish),
        MenuCommand::Review(review) => change(cx, |today| today.resolve_review(review)),
        MenuCommand::Quit => cx.quit(),
        MenuCommand::CorrectTime => {
            let _ = adjust_view::window::open(None, Seed::Blank, Opener::Tray, cx);
        }
        MenuCommand::ExportDiagnostics => crate::diagnostics::export(cx),
        MenuCommand::EditLatestAdjustment => {
            if let Some(edited) = cx.global::<Today>().latest_editable() {
                let workday = edited.workday().clone();
                let _ =
                    adjust_view::window::open(Some(workday), Seed::Edit(edited), Opener::Tray, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests;
