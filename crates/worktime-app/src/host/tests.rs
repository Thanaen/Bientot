use std::path::Path;

use jiff::SignedDuration;
use jiff::tz::TimeZone;
use worktime_domain::accounting::{RecordedTransition, SessionState};
use worktime_domain::time::{Clock as _, FixedTimeZoneProvider, FrozenClock, WorkdayCalendar};
use worktime_store::data_directory::DataDirectory;
use worktime_store::sessions::SessionEventRepository;

use super::*;
use crate::demo::Typing;
use crate::flyout;
use crate::live;
use crate::tray_model::ReviewAction;
use crate::week::{Answer, SegmentKind, Severity, WeekState};
use crate::widgets::NoticeKind;
use crate::workday::{FailureRemedy, SourceHint};

/// Thursday 8 October 2026, 08:00 UTC, where [`clock`] starts.
const START: &str = "2026-10-08T08:00:00Z";

/// The midnight after [`START`], where Friday 9 October starts in UTC.
const MIDNIGHT: &str = "2026-10-09T00:00:00Z";

fn clock() -> &'static FrozenClock {
    Box::leak(Box::new(FrozenClock::new(START.parse().unwrap())))
}

/// Starts WorkTime on `folder`, as a launch on the user's data does.
fn start(folder: &Path, clock: &'static FrozenClock) -> Today {
    open(live_host(folder, clock))
}

fn live_host(folder: &Path, clock: &'static FrozenClock) -> LiveHost<'static> {
    let directory = DataDirectory::from(folder.to_owned());
    let connection = worktime_store::database::open(&directory.database_file()).unwrap();
    LiveHost::start(
        &directory,
        live::load_settings(&directory).unwrap(),
        Box::leak(Box::new(connection)),
        clock,
        Box::leak(Box::new(FixedTimeZoneProvider::new("UTC", TimeZone::UTC))),
        &Typing,
    )
    .unwrap()
}

fn open(host: LiveHost<'static>) -> Today {
    Today::open(Source::Live(Box::new(host)), Language::English).unwrap()
}

#[test]
fn the_pause_toggle_resumes_a_finished_day() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);
    assert_eq!(today.tracking().state(), TrackingState::Running);

    clock.advance(SignedDuration::from_mins(1));
    today.finish();
    assert_eq!(today.tracking().state(), TrackingState::Finished);

    clock.advance(SignedDuration::from_mins(1));
    today.toggle_pause();
    assert_eq!(today.tracking().state(), TrackingState::Running);
}

/// `CanFinish` of the .NET app: a finished day cannot finish again, from the flyout's keyboard
/// shortcut either.
#[test]
fn finishing_a_finished_day_records_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);
    for _ in 0..2 {
        clock.advance(SignedDuration::from_mins(1));
        today.finish();
        assert_eq!(today.tracking().state(), TrackingState::Finished);
    }
    assert_eq!(
        recorded_tracking(folder.path(), clock),
        [TrackingState::Finished]
    );
}

/// The next working day resets a finished day, so that Finish finishes it again at once, before
/// any periodic refresh read the new day.
#[test]
fn the_next_working_day_can_finish_before_the_periodic_refresh() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);
    clock.advance(SignedDuration::from_mins(1));
    today.finish();
    assert_eq!(today.tracking().state(), TrackingState::Finished);

    clock.advance(SignedDuration::from_hours(24));
    today.finish();

    assert_eq!(today.tracking().state(), TrackingState::Finished);
    assert_eq!(
        recorded_tracking(folder.path(), clock),
        [TrackingState::Running, TrackingState::Finished]
    );
}

/// A Resume still shown for the finished day, as the tray menu built ahead of its opening does,
/// leaves the next working day running rather than pausing it.
#[test]
fn a_stale_resume_on_the_next_working_day_records_no_pause() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);
    clock.advance(SignedDuration::from_mins(1));
    today.finish();
    assert_eq!(today.tracking().state(), TrackingState::Finished);

    clock.advance(SignedDuration::from_hours(24));
    today.toggle_pause();

    assert_eq!(today.tracking().state(), TrackingState::Running);
    assert_eq!(
        recorded_tracking(folder.path(), clock),
        [TrackingState::Running]
    );
}

/// Opening the flyout on the next working day offers to finish it, and the tray menu with it,
/// before any periodic refresh read the new day.
#[test]
fn catching_up_on_the_next_working_day_offers_to_finish_it() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);
    clock.advance(SignedDuration::from_mins(1));
    today.finish();
    clock.advance(SignedDuration::from_hours(24));

    assert!(today.catch_up());

    assert_eq!(today.tracking().state(), TrackingState::Running);
    assert_eq!(
        command_items(&today),
        [
            (MenuCommand::TogglePause, true),
            (MenuCommand::Finish, true),
            (MenuCommand::CorrectTime, true)
        ]
    );
    assert!(!today.catch_up());
}

/// The catch-up timed by `until_rollover` is the first to offer to finish the next working day,
/// so the tray menu it rebuilds offers Finish from midnight. The 30 s refresh bounds what timer
/// slack or a sleep leaves stale.
#[test]
fn the_rollover_is_the_first_instant_a_catch_up_offers_to_finish_the_next_day() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);
    clock.advance(SignedDuration::from_mins(1));
    today.finish();
    let rollover = today.live().unwrap().until_rollover().unwrap();
    let midnight: jiff::Timestamp = MIDNIGHT.parse().unwrap();
    assert_eq!(
        rollover,
        clock.now().duration_until(midnight).unsigned_abs()
    );

    let rollover = SignedDuration::try_from(rollover).unwrap();
    clock.advance(rollover - SignedDuration::from_millis(1));
    assert!(!today.catch_up(), "a millisecond before midnight");
    assert_eq!(command_items(&today)[1], (MenuCommand::Finish, false));

    clock.advance(SignedDuration::from_millis(1));
    assert!(today.catch_up(), "at midnight");
    assert_eq!(command_items(&today)[1], (MenuCommand::Finish, true));
    assert_eq!(
        today.live().unwrap().until_rollover(),
        Some(SignedDuration::from_hours(24).unsigned_abs())
    );
}

/// Resolves the local zone's id to another zone: the local day is still Wednesday 7 October
/// at [`START`], while its next midnight, in the zone the id resolves to, has passed.
struct DriftedZones;

impl worktime_domain::time::TimeZoneProvider for DriftedZones {
    fn local(&self) -> (String, TimeZone) {
        ("Drifted".to_owned(), TimeZone::fixed(jiff::tz::offset(-12)))
    }

    fn get(&self, _: &str) -> Result<TimeZone, worktime_domain::time::UnknownTimeZone> {
        Ok(TimeZone::fixed(jiff::tz::offset(14)))
    }
}

/// A midnight resolved no later than now leaves the rollover untimed, so the watcher waits the
/// refresh interval instead of firing at once.
#[test]
fn a_midnight_already_past_times_no_rollover() {
    let folder = tempfile::tempdir().unwrap();
    let directory = DataDirectory::from(folder.path().to_owned());
    let connection = worktime_store::database::open(&directory.database_file()).unwrap();
    let host = LiveHost::start(
        &directory,
        live::load_settings(&directory).unwrap(),
        Box::leak(Box::new(connection)),
        clock(),
        &DriftedZones,
        &Typing,
    )
    .unwrap();
    assert_eq!(host.until_rollover(), None);
}

/// The first catch-up after midnight shows the next day while it keeps running, before any
/// periodic refresh read it.
#[test]
fn catching_up_after_midnight_shows_the_next_day() {
    let folder = tempfile::tempdir().unwrap();
    // Friday 9 October 2026.
    let clock = Box::leak(Box::new(FrozenClock::new(
        "2026-10-09T23:58:00Z".parse().unwrap(),
    )));
    let mut today = start(folder.path(), clock);
    assert_ne!(today.view().status, "Non-working day");

    clock.advance(SignedDuration::from_mins(4));
    assert!(today.catch_up());

    assert_eq!(today.tracking().state(), TrackingState::Running);
    assert_eq!(today.view().status, "Non-working day");
    assert!(!today.catch_up());
}

/// A catch-up that cannot read the tracking state shows the failed refresh, and tells it once:
/// the next one that fails changes nothing shown. The next one that succeeds ends it.
#[test]
fn a_failed_catch_up_is_told_once_until_one_succeeds() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);

    rename(folder.path(), "SessionEvent", "Hidden");
    clock.advance(SignedDuration::from_mins(1));
    assert!(today.catch_up());
    let failure =
        Notice::refresh_pending("Bientôt could not refresh. It will try again automatically.");
    assert_eq!(today.refresh_failure(), Some(failure.clone()));
    clock.advance(SignedDuration::from_mins(1));
    assert!(!today.catch_up());
    assert_eq!(today.refresh_failure(), Some(failure));

    rename(folder.path(), "Hidden", "SessionEvent");
    clock.advance(SignedDuration::from_mins(1));
    assert!(today.catch_up());
    assert_eq!(today.refresh_failure(), None);
    assert!(!today.catch_up());
}

/// A failed periodic refresh ends at the next catch-up that succeeds, as the next refresh would.
#[test]
fn a_catch_up_ends_a_failed_refresh() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);
    rename(folder.path(), "SessionEvent", "Hidden");
    clock.advance(SignedDuration::from_mins(1));
    today.refresh();
    assert!(today.refresh_failure().is_some());

    rename(folder.path(), "Hidden", "SessionEvent");
    clock.advance(SignedDuration::from_mins(1));
    assert!(today.catch_up());
    assert_eq!(today.refresh_failure(), None);
    assert!(!today.catch_up());
}

/// While the week cannot be stored, a catch-up still reads the latest correction beside the
/// failure, and reports it for the tray to show at once, as the periodic refresh would. One that
/// reads nothing new reports nothing.
#[test]
fn a_catch_up_during_a_failed_refresh_shows_what_it_read_beside_it() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);
    rename(folder.path(), "CreditLedgerEntry", "Hidden");
    clock.advance(SignedDuration::from_mins(1));
    today.refresh();
    assert!(today.refresh_failure().is_some());
    clock.advance(SignedDuration::from_mins(1));
    assert!(!today.catch_up());

    let directory = DataDirectory::from(folder.path().to_owned());
    let connection = worktime_store::database::open(&directory.database_file()).unwrap();
    let calendar = WorkdayCalendar::new(FixedTimeZoneProvider::new("UTC", TimeZone::UTC));
    let correction = ManualAdjustment::duration_work(
        AdjustmentId::new("recorded-beside").unwrap(),
        calendar.workday(clock.now()),
        SignedDuration::from_mins(10),
        clock.now(),
    )
    .unwrap();
    worktime_store::adjustments::ManualAdjustmentRepository::new(&connection)
        .append(&correction)
        .unwrap();
    clock.advance(SignedDuration::from_mins(1));

    assert!(today.catch_up());
    assert!(today.refresh_failure().is_some());
    assert!(today.menu().into_iter().any(|entry| matches!(
        entry,
        MenuEntry::Item {
            command: MenuCommand::EditLatestAdjustment,
            enabled: true,
            ..
        }
    )));
    assert!(!today.catch_up());
}

/// A source that keeps failing is left to the periodic refresh: a catch-up the same day does
/// not read it again, and the first one after midnight shows the new day.
#[test]
fn a_failing_source_is_read_again_by_a_catch_up_only_on_a_new_day() {
    let folder = tempfile::tempdir().unwrap();
    // Wednesday 7 October 2026.
    let clock = Box::leak(Box::new(FrozenClock::new(
        "2026-10-07T08:00:00Z".parse().unwrap(),
    )));
    let mut today = on_activity_watch(folder.path(), clock);
    clock.set("2026-10-08T08:00:00Z".parse().unwrap());
    today.refresh();
    clock.set("2026-10-09T08:00:00Z".parse().unwrap());
    today.refresh();
    let published = |today: &mut Today| today.live().unwrap().refresh_state().unwrap().clone();
    let failed = published(&mut today);
    assert_eq!(
        failed
            .source_failure
            .as_ref()
            .map(|failure| failure.workday_date),
        Some(jiff::civil::date(2026, 10, 8)),
        "Thursday is stored without evidence"
    );

    clock.advance(SignedDuration::from_mins(1));
    assert!(!today.catch_up());
    assert_eq!(
        published(&mut today),
        failed,
        "the source is not read again"
    );

    clock.set("2026-10-10T00:02:00Z".parse().unwrap());
    assert!(today.catch_up());
    assert_eq!(today.view().status, "Check activity tracking");
    clock.advance(SignedDuration::from_mins(1));
    assert!(!today.catch_up());
}

/// The tracking changes recorded on the workday of `clock` now.
fn recorded_tracking(folder: &Path, clock: &FrozenClock) -> Vec<TrackingState> {
    let directory = DataDirectory::from(folder.to_owned());
    let connection = worktime_store::database::open(&directory.database_file()).unwrap();
    let calendar = WorkdayCalendar::new(FixedTimeZoneProvider::new("UTC", TimeZone::UTC));
    let window = calendar.window(&calendar.workday(clock.now())).unwrap();
    SessionEventRepository::new(&connection, clock)
        .read_for_accounting(&window)
        .unwrap()
        .in_window()
        .iter()
        .filter_map(|transition| match transition {
            RecordedTransition::Tracking(recorded) => Some(recorded.state),
            _ => None,
        })
        .collect()
}

/// Whether the tray menu enables the pause toggle, « Terminer » and « Corriger le temps… ».
fn command_items(today: &Today) -> Vec<(MenuCommand, bool)> {
    today
        .menu()
        .into_iter()
        .filter_map(|entry| match entry {
            MenuEntry::Item {
                command:
                    command
                    @ (MenuCommand::TogglePause | MenuCommand::Finish | MenuCommand::CorrectTime),
                enabled,
                ..
            } => Some((command, enabled)),
            _ => None,
        })
        .collect()
}

/// The day « Corriger le temps… » opens on from the flyout or the tray, if any.
fn corrected_day(today: &Today) -> Option<jiff::civil::Date> {
    today.adjust_target(None).map(|target| target.workday.date)
}

/// `MainWindow.UpdateCommands` of the .NET app: no pause, resume nor finish on a day off,
/// from the flyout's keyboard shortcuts nor the tray menu either. Nor a correction, as
/// `CanAddTime` says.
#[test]
fn a_day_off_records_no_pause_nor_finish() {
    let folder = tempfile::tempdir().unwrap();
    // Saturday 10 October 2026.
    let clock = Box::leak(Box::new(FrozenClock::new(
        "2026-10-10T08:00:00Z".parse().unwrap(),
    )));
    let mut today = start(folder.path(), clock);

    for _ in 0..2 {
        clock.advance(SignedDuration::from_mins(1));
        today.toggle_pause();
        clock.advance(SignedDuration::from_mins(1));
        today.finish();
    }
    assert_eq!(today.tracking().state(), TrackingState::Running);
    assert_eq!(recorded_tracking(folder.path(), clock), []);
    assert_eq!(
        command_items(&today),
        [
            (MenuCommand::TogglePause, false),
            (MenuCommand::Finish, false),
            (MenuCommand::CorrectTime, false)
        ]
    );
    assert_eq!(corrected_day(&today), None);
}

/// `CanAddTime` of the .NET app follows the working day of the calendar, and `AddTimeAsync`
/// corrects today by the calendar: a built-in source that fails from Saturday on keeps
/// Friday's summary published, yet Saturday offers no correction, of Friday or any day.
#[test]
fn saturday_offers_no_correction_while_friday_stays_published() {
    let folder = tempfile::tempdir().unwrap();
    // Friday 9 October 2026.
    let clock = Box::leak(Box::new(FrozenClock::new(
        "2026-10-09T16:00:00Z".parse().unwrap(),
    )));
    let mut today = start(folder.path(), clock);
    assert_eq!(corrected_day(&today), Some(jiff::civil::date(2026, 10, 9)));
    rename(folder.path(), "ActivitySpan", "Hidden");

    clock.set("2026-10-10T10:00:00Z".parse().unwrap());
    today.refresh();
    assert_eq!(today.view().status, "Check activity tracking");
    assert_eq!(
        command_items(&today),
        [
            (MenuCommand::TogglePause, false),
            (MenuCommand::Finish, false),
            (MenuCommand::CorrectTime, false)
        ]
    );
    assert_eq!(corrected_day(&today), None);
}

/// `App.xaml.cs` reads the working day from the calendar at every refresh: a built-in source
/// that fails from Sunday on keeps Saturday's summary published, yet Monday resumes the pause
/// taken on Friday.
#[test]
fn monday_resumes_a_friday_pause_while_the_source_fails() {
    let folder = tempfile::tempdir().unwrap();
    // Friday 9 October 2026.
    let clock = Box::leak(Box::new(FrozenClock::new(
        "2026-10-09T16:00:00Z".parse().unwrap(),
    )));
    let mut today = start(folder.path(), clock);
    clock.advance(SignedDuration::from_mins(1));
    today.toggle_pause();

    clock.set("2026-10-10T10:00:00Z".parse().unwrap());
    today.refresh();
    assert_eq!(today.view().status, "Non-working day");
    rename(folder.path(), "ActivitySpan", "Hidden");

    clock.set("2026-10-12T08:00:00Z".parse().unwrap());
    today.refresh();
    assert_eq!(today.view().status, "Non-working day");
    assert_eq!(today.tracking().state(), TrackingState::Paused);
    assert_eq!(
        command_items(&today),
        [
            (MenuCommand::TogglePause, true),
            (MenuCommand::Finish, true),
            (MenuCommand::CorrectTime, true)
        ]
    );
    assert_eq!(corrected_day(&today), Some(jiff::civil::date(2026, 10, 12)));

    clock.advance(SignedDuration::from_mins(1));
    today.toggle_pause();
    assert_eq!(today.tracking().state(), TrackingState::Running);
    assert_eq!(
        recorded_tracking(folder.path(), clock),
        [TrackingState::Running]
    );
}

/// A launch on a Saturday whose built-in source fails publishes no summary, and still offers
/// no pause, « Terminer » nor correction.
#[test]
fn a_weekend_start_without_a_summary_offers_no_pause_nor_finish() {
    let folder = tempfile::tempdir().unwrap();
    // Saturday 10 October 2026.
    let clock = Box::leak(Box::new(FrozenClock::new(
        "2026-10-10T08:00:00Z".parse().unwrap(),
    )));
    start(folder.path(), clock);
    rename(folder.path(), "ActivitySpan", "Hidden");

    clock.advance(SignedDuration::from_mins(1));
    let mut today = start(folder.path(), clock);
    assert!(today.live().unwrap().refresh_state().is_none());
    assert_eq!(today.view().status, "Check activity tracking");
    assert_eq!(
        command_items(&today),
        [
            (MenuCommand::TogglePause, false),
            (MenuCommand::Finish, false),
            (MenuCommand::CorrectTime, false)
        ]
    );
    assert_eq!(corrected_day(&today), None);
    clock.advance(SignedDuration::from_mins(1));
    today.toggle_pause();
    today.finish();
    assert_eq!(recorded_tracking(folder.path(), clock), []);
}

#[test]
fn a_restart_on_a_finished_day_shows_it_finished_then_resumes_it() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    start(folder.path(), clock).finish();

    clock.advance(SignedDuration::from_mins(1));
    let mut today = start(folder.path(), clock);
    assert_eq!(today.tracking().state(), TrackingState::Finished);

    clock.advance(SignedDuration::from_mins(1));
    today.toggle_pause();
    assert_eq!(today.tracking().state(), TrackingState::Running);
}

#[test]
fn the_pause_toggle_pauses_a_running_day_and_resumes_a_paused_one() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);
    for state in [TrackingState::Paused, TrackingState::Running] {
        clock.advance(SignedDuration::from_mins(1));
        today.toggle_pause();
        assert_eq!(today.tracking().state(), state);
    }
}

/// Renames a table of the ledger through a second connection: hiding one makes what needs it
/// fail, and giving the name back repairs it.
fn rename(folder: &Path, from: &str, to: &str) {
    let directory = DataDirectory::from(folder.to_owned());
    worktime_store::database::open(&directory.database_file())
        .unwrap()
        .execute(&format!("ALTER TABLE {from} RENAME TO {to}"), [])
        .unwrap();
}

/// Today after half an hour at the keyboard, with the night before as a gap to settle.
fn a_day_with_a_gap(folder: &Path, clock: &'static FrozenClock) -> (Today, WeekState) {
    let mut today = start(folder, clock);
    for _ in 0..360 {
        clock.advance(SignedDuration::from_secs(5));
        today.live().unwrap().poll();
    }
    today.refresh();
    let state = WeekState::new(today.week().unwrap(), Language::English);
    (today, state)
}

/// What the week window does after every refresh of the day.
fn reload(today: &Today, state: &mut WeekState) {
    match today.week() {
        Ok(week) => state.reload(week, today.recalculated()),
        Err(_) => state.load_failed(),
    }
}

/// « Régler les trous », then « J'ai travaillé » on the first slot, when the week offers it.
fn settle_first_gap(state: &mut WeekState) -> Option<WeekCommand> {
    state.settle_next();
    state.choose(Answer::Count)
}

const REFRESH_PENDING: &str = "The correction was saved, but the week could not be refreshed. \
                               Refresh the week; the correction will not be submitted again.";

#[test]
fn a_correction_saved_before_a_failed_recalculation_asks_to_refresh() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let (mut today, mut state) = a_day_with_a_gap(folder.path(), clock);
    let pending = state.timeline.pending_count();
    let command = settle_first_gap(&mut state).unwrap();
    rename(folder.path(), "CreditLedgerEntry", "Hidden");
    clock.advance(SignedDuration::from_mins(1));
    let result = today.run_week(&command);
    reload(&today, &mut state);
    state.finished(result);

    let bar = state.info_bar().unwrap();
    assert_eq!(
        (bar.message.as_str(), bar.severity, bar.can_undo),
        (REFRESH_PENDING, Severity::Warning, false)
    );
    // The week shown still has the slot, but no longer offers to settle it.
    assert_eq!(state.timeline.pending_count(), pending);
    assert_eq!(settle_first_gap(&mut state), None);

    rename(folder.path(), "Hidden", "CreditLedgerEntry");
    clock.advance(SignedDuration::from_mins(1));
    today.refresh();
    reload(&today, &mut state);
    let bar = state.info_bar().unwrap();
    assert_eq!((bar.severity, bar.can_undo), (Severity::Success, true));
    assert_eq!(state.timeline.pending_count(), pending - 1);
}

#[test]
fn a_correction_saved_before_a_failed_week_read_asks_to_refresh() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let (mut today, mut state) = a_day_with_a_gap(folder.path(), clock);
    let command = settle_first_gap(&mut state).unwrap();
    clock.advance(SignedDuration::from_mins(1));
    let result = today.run_week(&command);
    rename(folder.path(), "DailySummaryRevision", "Hidden");
    reload(&today, &mut state);
    state.finished(result);

    let bar = state.info_bar().unwrap();
    assert_eq!(
        (bar.message.as_str(), bar.severity, bar.can_undo),
        (REFRESH_PENDING, Severity::Warning, false)
    );
    assert_eq!(settle_first_gap(&mut state), None);

    rename(folder.path(), "Hidden", "DailySummaryRevision");
    clock.advance(SignedDuration::from_mins(1));
    today.refresh();
    reload(&today, &mut state);
    let bar = state.info_bar().unwrap();
    assert_eq!((bar.severity, bar.can_undo), (Severity::Success, true));
}

/// Today on ActivityWatch, which the flyout offers to leave for the built-in detection.
fn on_activity_watch(folder: &Path, clock: &'static FrozenClock) -> Today {
    let mut today = start(folder, clock);
    let settings = WorkTimeSettings {
        activity_source: Some(ActivitySourceKind::ActivityWatch),
        ..today.settings().clone()
    };
    today.save_settings(&settings).unwrap();
    clock.advance(SignedDuration::from_mins(1));
    today.refresh();
    today
}

fn offers_built_in(today: &Today) -> bool {
    today.view().source_hint.as_ref().is_some_and(|hint| {
        matches!(
            hint,
            SourceHint::Failure {
                remedy: FailureRemedy::UseBuiltIn,
                ..
            }
        )
    })
}

#[test]
fn a_failed_switch_from_the_flyout_says_so_and_still_offers_it() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = on_activity_watch(folder.path(), clock);
    assert!(offers_built_in(&today));
    // A folder where `settings.json` should be: the save cannot replace it.
    let settings_file = DataDirectory::from(folder.path().to_owned()).settings_file();
    std::fs::remove_file(&settings_file).unwrap();
    std::fs::create_dir(&settings_file).unwrap();

    clock.advance(SignedDuration::from_mins(1));
    let notice = today.use_built_in_detection().unwrap();
    assert_eq!(
        notice,
        Notice::source_switch_failed(
            "Couldn’t switch to built-in detection. Check disk access and try again."
        )
    );
    assert_eq!(
        today.settings().activity_source,
        Some(ActivitySourceKind::ActivityWatch)
    );
    assert!(offers_built_in(&today));
    // The periodic refreshes keep the notice, next to the button that retries.
    assert!(notice.outlives(true));
    let shown = flyout::after_switch(None, Some(notice));

    std::fs::remove_dir(&settings_file).unwrap();
    clock.advance(SignedDuration::from_mins(1));
    let failure = today.use_built_in_detection();
    assert_eq!(failure, None);
    assert_eq!(
        today.settings().activity_source,
        Some(ActivitySourceKind::BuiltIn)
    );
    assert_eq!(flyout::after_switch(shown, failure), None);
}

#[test]
fn the_diagnostic_export_of_a_live_day_holds_its_snapshot_and_its_log() {
    use worktime_core::diagnostics::{
        DiagnosticArchitecture, DiagnosticExport, DiagnosticLogLevel, DiagnosticNumericVersion,
        DiagnosticSummaryAvailability, NoFinalPathCheck,
    };

    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);
    today.with_live(|host| host.log(DiagnosticLogLevel::Information, "startup.tray_ready", &[]));
    let version = DiagnosticNumericVersion {
        major: 0,
        minor: 1,
        build: 0,
        revision: 0,
    };
    let snapshot = today.diagnostic_snapshot(DiagnosticRuntimeSnapshot {
        application_version: version,
        operating_system_version: version,
        dot_net_version: version,
        process_architecture: DiagnosticArchitecture::X64,
        operating_system_architecture: DiagnosticArchitecture::X64,
    });
    assert_eq!(
        snapshot.initialization,
        DiagnosticInitializationState::Ready
    );
    assert!(snapshot.settings.is_some());
    // No activity source covers this day yet, so its summary is not current.
    assert_eq!(
        snapshot
            .summary
            .as_ref()
            .map(|summary| summary.availability),
        Some(DiagnosticSummaryAvailability::Stale)
    );
    assert_eq!(
        today.log_directory(),
        Some(folder.path().join("logs").as_path())
    );

    let destination = folder
        .path()
        .join("export")
        .join("worktime-diagnostics.zip");
    let exported = DiagnosticExport::new(clock, &NoFinalPathCheck)
        .export(&destination, today.log_directory(), &snapshot)
        .unwrap();
    assert_eq!(exported.log_record_count, 1);
    assert_eq!(
        std::fs::metadata(&destination).unwrap().len(),
        exported.archive_bytes
    );
}

#[test]
fn a_store_check_is_saved_to_the_settings_and_keeps_the_switch() {
    let folder = tempfile::tempdir().unwrap();
    let mut today = start(folder.path(), clock());
    let at: Timestamp = "2026-10-08T08:00:00Z".parse().unwrap();

    today.record_update_check(at).unwrap();

    let saved = live::load_settings(&DataDirectory::from(folder.path().to_owned())).unwrap();
    assert_eq!(saved.settings.updates.last_checked_at, Some(at));
    assert!(saved.settings.updates.check_for_updates);
    assert_eq!(today.settings().updates.last_checked_at, Some(at));
}

const SETTINGS_RESTORED: &str =
    "Invalid settings were backed up and the defaults restored. Review your settings.";

const NOTIFICATIONS_UNAVAILABLE: &str =
    "Windows notifications are unavailable. Workday tracking continues.";

/// `InitializeAsync` of the .NET app shows `SettingsRecovered` once the store moved an invalid
/// `settings.json` aside. The next launch reads no invalid file, so it tells nothing.
#[test]
fn restored_settings_are_told_in_the_first_flyout_only() {
    let folder = tempfile::tempdir().unwrap();
    let settings_file = DataDirectory::from(folder.path().to_owned()).settings_file();
    std::fs::write(&settings_file, "{ \"schemaVersion\": ").unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);

    assert_eq!(
        today.opening_alerts(),
        Alerts {
            announcements: vec![Notice::settings_recovered(SETTINGS_RESTORED)],
            refresh_failure: None,
        }
    );
    today.announced();
    assert_eq!(today.opening_alerts(), Alerts::default());

    let restarted = start(folder.path(), clock);
    assert_eq!(restarted.opening_alerts(), Alerts::default());
}

/// `InitializeAsync` of the .NET app shows `NotificationRegistrationFailed` when Windows
/// refuses the toasts, after `SettingsRecovered`.
#[test]
fn unavailable_notifications_are_told_in_the_first_flyout_only() {
    let folder = tempfile::tempdir().unwrap();
    let settings_file = DataDirectory::from(folder.path().to_owned()).settings_file();
    std::fs::write(&settings_file, "[]").unwrap();
    let mut host = live_host(folder.path(), clock());
    host.notifications_refused(&"the sender is not registered");
    let mut today = open(host);

    assert_eq!(
        today.opening_alerts().announcements,
        [
            Notice::settings_recovered(SETTINGS_RESTORED),
            Notice::warning(NOTIFICATIONS_UNAVAILABLE)
        ]
    );
    today.announced();
    assert_eq!(today.opening_alerts().announcements, []);
}

/// A flyout that could not open told nothing: the next one tells what went wrong at the start.
#[test]
fn the_start_is_told_until_a_flyout_announced_it() {
    let folder = tempfile::tempdir().unwrap();
    let settings_file = DataDirectory::from(folder.path().to_owned()).settings_file();
    std::fs::write(&settings_file, "[]").unwrap();
    let today = start(folder.path(), clock());

    for _ in 0..2 {
        assert_eq!(
            today.opening_alerts().announcements,
            [Notice::settings_recovered(SETTINGS_RESTORED)]
        );
    }
}

/// `PeriodicRefreshService` of the .NET app shows `PeriodicRefreshErrorMessage` at each failed
/// refresh; the next refresh that succeeds ends it here.
#[test]
fn a_failed_refresh_is_told_until_one_succeeds() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let mut today = start(folder.path(), clock);
    assert_eq!(today.refresh_failure(), None);

    rename(folder.path(), "SessionEvent", "Hidden");
    clock.advance(SignedDuration::from_mins(1));
    today.refresh();
    let failure =
        Notice::refresh_pending("Bientôt could not refresh. It will try again automatically.");
    assert_eq!(today.refresh_failure(), Some(failure.clone()));
    for _ in 0..2 {
        assert_eq!(
            today.opening_alerts().refresh_failure,
            Some(failure.clone())
        );
    }

    rename(folder.path(), "Hidden", "SessionEvent");
    clock.advance(SignedDuration::from_mins(1));
    today.refresh();
    assert_eq!(today.refresh_failure(), None);
    assert_eq!(today.opening_alerts(), Alerts::default());
}

#[path = "correction_tests.rs"]
mod correction_tests;

/// The day of the problem periods, in French, its demo ledger in `folder`.
fn problem_periods(folder: &Path) -> Today {
    let host = DemoHost::open(worktime_core::demo::DemoScenario::ProblemPeriods, folder).unwrap();
    Today::open(Source::Demo(Box::new(host)), Language::French).unwrap()
}

fn worked(today: &Today) -> String {
    today.view().worked.as_ref().unwrap().worked.to_string()
}

/// « Oui, compter 26 min » of the hint as the flyout shows it.
fn count_shown(today: &Today) -> flyout::CountUnrecorded {
    let Some(SourceHint::Unrecorded {
        workday, intervals, ..
    }) = today.view().source_hint.clone()
    else {
        panic!("the problem periods leave time unrecorded");
    };
    flyout::CountUnrecorded { workday, intervals }
}

/// The 26 minutes the hint reports are counted as work at once, the notice says so with
/// « Annuler », and « Annuler » brings them back unrecorded.
#[test]
fn counting_the_unrecorded_time_adds_it_to_the_day_until_undone() {
    let folder = tempfile::tempdir().unwrap();
    let mut today = problem_periods(folder.path());
    let nb = |text: &str| text.replace('~', "\u{a0}");
    assert_eq!(worked(&today), nb("5~h~04~min"));

    let notice = today.count_unrecorded(&count_shown(&today));
    assert_eq!(notice.message, nb("Compté comme travail~:~26~min."));
    assert_eq!(notice.kind, NoticeKind::Success);
    assert_eq!(worked(&today), nb("5~h~30~min"));
    assert_eq!(today.view().source_hint, None);

    let undo = notice.undo.expect("the count can be undone");
    let undone = today.undo(&undo);
    assert_eq!(undone.message, "Action annulée.");
    assert_eq!(undone.undo, None);
    assert_eq!(worked(&today), nb("5~h~04~min"));
    assert_eq!(
        today
            .view()
            .source_hint
            .as_ref()
            .map(|hint| hint.text(true)),
        Some(
            &nb(
                "Temps non renseigné · 26~min en 2~créneaux, entre 09:40 et 14:20. \
                 Travailliez-vous\u{202f}?"
            )
            .into()
        )
    );
}

/// The notice reads the same whatever the duration: « 1 min » takes no plural agreement.
#[test]
fn the_counted_notice_agrees_with_a_single_minute() {
    let folder = tempfile::tempdir().unwrap();
    let mut today = problem_periods(folder.path());
    let mut count = count_shown(&today);
    let first = count.intervals[0];
    count.intervals = vec![
        UtcInterval::new(first.start(), first.start() + SignedDuration::from_mins(1)).unwrap(),
    ];
    let notice = today.count_unrecorded(&count);
    assert_eq!(
        notice.message,
        "Compté comme travail\u{a0}:\u{a0}1\u{a0}min."
    );
}

/// Counted elsewhere since the flyout showed it, the time is not counted twice: the notice
/// says the day changed.
#[test]
fn time_counted_since_the_hint_showed_it_is_refused() {
    let folder = tempfile::tempdir().unwrap();
    let mut today = problem_periods(folder.path());
    let count = count_shown(&today);
    today.count_unrecorded(&count);
    let again = today.count_unrecorded(&count);
    assert_eq!(
        again.message,
        "Cette période a changé depuis son affichage. Actualisez la journée pour voir son état \
         actuel."
    );
    assert_eq!(again.kind, NoticeKind::Warning);
    assert_eq!(again.undo, None);
    assert_eq!(worked(&today), "5\u{a0}h\u{a0}30\u{a0}min");
}
