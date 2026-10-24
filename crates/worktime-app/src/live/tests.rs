use std::num::NonZeroU64;

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use worktime_domain::accounting::{RecordedTransition, SessionState};
use worktime_domain::adjustments::{AdjustmentKind, ManualClassification};
use worktime_domain::time::{FixedTimeZoneProvider, FrozenClock};
use worktime_windows::lifecycle::LifecycleChange;
use worktime_windows::power_history::{
    PowerHistory, PowerHistoryBoundary, PowerHistoryBoundaryKind, PowerHistoryInterval,
    PowerHistoryIntervalKind,
};

use super::*;
use crate::demo::Typing;
use crate::strings::keep_together;
use crate::tray_model::{self, MenuEntry, MenuState};
use crate::week::{Answer, Gains, Lock, Recorded, SegmentId, SegmentKind, WeekState};
use crate::workday::SourceHint;

struct Machine {
    folder: tempfile::TempDir,
    connection: Connection,
    clock: FrozenClock,
    zones: FixedTimeZoneProvider,
}

impl Machine {
    /// Thursday 8 October 2026, 08:00 UTC, on a folder WorkTime never used.
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let connection =
            worktime_store::database::open(&folder.path().join("worktime.db")).unwrap();
        Self {
            folder,
            connection,
            clock: FrozenClock::new("2026-10-08T08:00:00Z".parse().unwrap()),
            zones: FixedTimeZoneProvider::new("UTC", TimeZone::UTC),
        }
    }

    fn directory(&self) -> DataDirectory {
        DataDirectory::from(self.folder.path().to_owned())
    }

    fn start(&self) -> LiveHost<'_> {
        let directory = self.directory();
        let loaded = load_settings(&directory).unwrap();
        LiveHost::start(
            &directory,
            loaded,
            &self.connection,
            &self.clock,
            &self.zones,
            &Typing,
        )
        .unwrap()
    }

    /// Samples every 5 s for `duration`, as the host's timer does.
    fn work(&self, host: &mut LiveHost, duration: SignedDuration) {
        let end = self.clock.now() + duration;
        while self.clock.now() < end {
            self.clock.advance(SignedDuration::from_secs(5));
            host.poll();
        }
    }

    fn log(&self) -> String {
        let Ok(files) = std::fs::read_dir(self.folder.path().join("logs")) else {
            return String::new();
        };
        files
            .map(|file| std::fs::read_to_string(file.unwrap().path()).unwrap())
            .collect()
    }
}

#[test]
fn a_first_start_selects_the_built_in_source_without_saving_the_settings() {
    let machine = Machine::new();
    let host = machine.start();
    assert_eq!(
        host.settings().activity_source,
        Some(ActivitySourceKind::BuiltIn)
    );
    let timeline = ActivitySourceSelectionRepository::new(&machine.connection, &machine.clock)
        .read_timeline(Timestamp::UNIX_EPOCH, None)
        .unwrap();
    assert_eq!(
        timeline
            .iter()
            .map(|selection| (selection.effective_from, selection.source))
            .collect::<Vec<_>>(),
        [(machine.clock.now(), ActivitySourceKind::BuiltIn)]
    );
    assert!(!machine.directory().settings_file().exists());
}

#[test]
fn the_day_is_recalculated_from_the_built_in_source() {
    let machine = Machine::new();
    let mut host = machine.start();
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(90));
    host.refresh();
    let day = host.view(Language::English);
    assert_eq!(day.source_hint, None);
    let worked = day.worked.unwrap();
    assert_eq!(
        (
            worked.worked.as_ref(),
            worked.target.map(|target| target.duration).as_deref()
        ),
        ("1h\u{a0}30m", Some("7h\u{a0}00m"))
    );
    assert_eq!(machine.log(), "");
}

/// The night before the first activity of the day is not unrecorded work (#301), though no
/// power history says the computer was off then.
#[test]
fn the_night_before_the_first_activity_is_not_unrecorded() {
    let machine = Machine::new();
    machine.clock.set("2026-10-08T09:30:00Z".parse().unwrap());
    let mut host = machine.start();
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(90));
    host.refresh();
    assert_eq!(host.view(Language::English).source_hint, None);
}

/// Paused from the start, the day holds no work: it has no arrival to leave the night out by, so
/// all of its unknown time is reported.
#[test]
fn a_day_without_work_reports_all_of_its_unrecorded_time() {
    let machine = Machine::new();
    machine.clock.set("2026-10-08T09:30:00Z".parse().unwrap());
    let mut host = machine.start();
    host.command(TrackingCommand::Pause);
    machine.work(&mut host, SignedDuration::from_mins(30));
    host.refresh();
    assert_eq!(
        host.view(Language::English)
            .source_hint
            .map(|hint| match hint {
                SourceHint::Unrecorded {
                    message, intervals, ..
                } => (message, intervals),
                other => panic!("not unrecorded time: {other:?}"),
            }),
        Some((
            "Unrecorded time · 9h\u{a0}30m, from 12:00\u{a0}AM to 9:30\u{a0}AM. Were you working? \
             Enter the times."
                .into(),
            vec![
                UtcInterval::new(
                    "2026-10-08T00:00:00Z".parse().unwrap(),
                    "2026-10-08T09:30:00Z".parse().unwrap()
                )
                .unwrap()
            ],
        ))
    );
}

/// WorkTime stopped from 10:00 to 11:00 after the day began: that hour is unrecorded, the night
/// before 09:30 is not.
#[test]
fn a_gap_after_the_first_activity_is_unrecorded() {
    let machine = Machine::new();
    machine.clock.set("2026-10-08T09:30:00Z".parse().unwrap());
    let mut host = machine.start();
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(30));
    machine.clock.set("2026-10-08T11:00:00Z".parse().unwrap());
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(30));
    host.refresh();
    assert_eq!(
        host.view(Language::English)
            .source_hint
            .map(|hint| match hint {
                SourceHint::Unrecorded {
                    message, intervals, ..
                } => (message, intervals),
                other => panic!("not unrecorded time: {other:?}"),
            }),
        Some((
            "Unrecorded time · 1h\u{a0}00m, from 10:00\u{a0}AM to 11:00\u{a0}AM. Were you working? \
             Enter the times.".into(),
            vec![
                UtcInterval::new(
                    "2026-10-08T10:00:00Z".parse().unwrap(),
                    "2026-10-08T11:00:00Z".parse().unwrap()
                )
                .unwrap()
            ],
        ))
    );
}

/// Unrecorded time under five minutes is not reported, as `SourceDiagnosticsPresenter` of the
/// .NET app leaves it out; from five minutes it is, to the minute as Début and Fin show it.
#[test]
fn unrecorded_time_is_reported_from_five_minutes() {
    for (resumed, hint) in [
        ("2026-10-08T08:34:59Z", None),
        (
            "2026-10-08T08:35:00Z",
            Some(
                "Unrecorded time · 5m, from 8:30\u{a0}AM to 8:35\u{a0}AM. Were you working? Enter the times.",
            ),
        ),
        (
            "2026-10-08T08:35:01Z",
            Some(
                "Unrecorded time · 5m, from 8:30\u{a0}AM to 8:35\u{a0}AM. Were you working? Enter the times.",
            ),
        ),
    ] {
        let machine = Machine::new();
        let mut host = machine.start();
        host.poll();
        // Work ends at 08:30, so resuming at 08:35:00 leaves exactly five minutes.
        machine.work(&mut host, SignedDuration::from_mins(30));
        machine.clock.set(resumed.parse().unwrap());
        host.poll();
        machine.work(&mut host, SignedDuration::from_mins(30));
        host.refresh();
        assert_eq!(
            host.view(Language::English)
                .source_hint
                .map(|hint| match hint {
                    SourceHint::Unrecorded { message, .. } => message.to_string(),
                    failure => panic!("not a failure: {failure:?}"),
                }),
            hint.map(str::to_owned),
            "resumed at {resumed}"
        );
    }
}

/// The night the power history recovered was off, not unknown: a start two minutes after the
/// boot leaves too little unrecorded time to report.
#[test]
fn a_start_shortly_after_a_recovered_boot_reports_no_unrecorded_time() {
    let machine = Machine::new();
    let mut host = machine.start();
    let boundary = |kind, record_id, at: &str| PowerHistoryBoundary {
        kind,
        record_id: NonZeroU64::new(record_id).unwrap(),
        occurred_at: at.parse().unwrap(),
    };
    let boot = boundary(PowerHistoryBoundaryKind::Boot, 2, "2026-10-08T07:58:00Z");
    let night = PowerHistoryInterval::new(
        PowerHistoryIntervalKind::PoweredOff,
        boundary(
            PowerHistoryBoundaryKind::Shutdown,
            1,
            "2026-10-07T20:00:00Z",
        ),
        boot,
    )
    .unwrap();
    host.recover_power_history(move |_| {
        PowerHistoryRead::Complete(PowerHistory {
            intervals: vec![night],
            boots: vec![boot],
        })
    })
    .unwrap();
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(90));
    host.refresh();
    assert_eq!(host.view(Language::English).source_hint, None);
    assert_eq!(machine.log(), "");
}

/// `MainWindow.UpdateCommands` of the .NET app only asks to resume on a working day. The
/// header says the day off: the status bar says nothing more, as for a pause.
#[test]
fn a_day_off_says_no_work_is_scheduled_whatever_the_tracking() {
    let machine = Machine::new();
    machine.clock.set("2026-10-10T08:00:00Z".parse().unwrap());
    let mut host = machine.start();
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(30));
    host.refresh();
    let day = host.view(Language::English);
    assert_eq!(day.status, "Non-working day");
    for tracking in [
        TrackingState::Running,
        TrackingState::Paused,
        TrackingState::Finished,
    ] {
        let shown = day.shown_while(tracking);
        assert_eq!(
            (
                shown.status.as_ref(),
                shown.headline.label.as_deref(),
                shown.headline.value.as_ref(),
            ),
            ("Non-working day", None, "No work scheduled"),
            "{tracking:?}"
        );
    }
}

/// Nothing is due on a day off: the time worked shows alone, with no target to reach. Its
/// activity is never credited, so half an hour at the keyboard still shows no time worked.
#[test]
fn a_day_off_shows_the_time_worked_without_a_target() {
    let machine = Machine::new();
    machine.clock.set("2026-10-10T08:00:00Z".parse().unwrap());
    let mut host = machine.start();
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(30));
    host.refresh();
    let shown = |language| {
        let day = host.view(language);
        let worked = day.worked.unwrap();
        (
            worked.label.to_string(),
            worked.worked.replace('\u{a0}', "_"),
            worked.target,
            day.tray_tooltip.replace('\u{a0}', "_"),
        )
    };
    assert_eq!(
        shown(Language::English),
        (
            "Time worked".to_owned(),
            "0m".to_owned(),
            None,
            "Non-working day · 0m · No work scheduled".to_owned()
        )
    );
    assert_eq!(
        shown(Language::French),
        (
            "Temps travaillé".to_owned(),
            "0_min".to_owned(),
            None,
            "Jour non ouvré · 0_min · Aucun travail prévu".to_owned()
        )
    );
}

#[test]
fn pause_resume_and_finish_are_recorded_then_shown() {
    let machine = Machine::new();
    let mut host = machine.start();
    assert_eq!(host.tracking(), TrackingState::Running);
    for (command, state) in [
        (TrackingCommand::Pause, TrackingState::Paused),
        (TrackingCommand::Resume, TrackingState::Running),
        (TrackingCommand::Finish, TrackingState::Finished),
    ] {
        machine.clock.advance(SignedDuration::from_mins(1));
        host.command(command);
        assert_eq!(host.tracking(), state);
    }
    assert_eq!(machine.log(), "");
}

#[test]
fn a_power_history_that_could_not_be_read_is_logged() {
    let machine = Machine::new();
    let mut host = machine.start();
    host.recover_power_history(|_| PowerHistoryRead::Truncated)
        .unwrap();
    let log = machine.log();
    assert!(log.contains(r#""Level":"Warning","Event":"power_history.recovery_unavailable","Properties":{"status":"Truncated","reason":"windows-system-log-record-limit"}"#), "{log}");
}

#[test]
fn a_session_probe_that_gave_up_is_logged() {
    let machine = Machine::new();
    let mut host = machine.start();
    host.prepare_lifecycle(|| Some(SessionState::Unlocked), std::iter::empty());
    assert_eq!(machine.log(), "");
    host.prepare_lifecycle(|| None, std::iter::empty());
    let log = machine.log();
    assert!(
        log.contains(r#""Event":"lifecycle.current_probe_unavailable","Properties":null"#),
        "{log}"
    );
}

#[test]
fn an_unlock_that_arrives_while_the_probe_runs_follows_the_locked_state_it_read() {
    let machine = Machine::new();
    let mut host = machine.start();
    let probed_at = machine.clock.now();
    let unlocked_at = probed_at + SignedDuration::from_secs(2);
    let (window, arrived) = std::sync::mpsc::channel();
    host.prepare_lifecycle(
        || {
            machine.clock.advance(SignedDuration::from_secs(2));
            window
                .send(LifecycleTransition {
                    change: LifecycleChange::Session(SessionState::Unlocked),
                    occurred_at: machine.clock.now(),
                    time_zone_id: "UTC".into(),
                })
                .unwrap();
            machine.clock.advance(SignedDuration::from_secs(1));
            Some(SessionState::Locked)
        },
        arrived.try_iter(),
    );

    let calendar = WorkdayCalendar::new(&machine.zones);
    let window = calendar
        .window(&calendar.workday(machine.clock.now()))
        .unwrap();
    let history = SessionEventRepository::new(&machine.connection, &machine.clock)
        .read_for_accounting(&window)
        .unwrap();
    let sessions: Vec<_> = history
        .in_window()
        .iter()
        .filter_map(|transition| match transition {
            RecordedTransition::Session(session) => Some((session.occurred_at, session.state)),
            _ => None,
        })
        .collect();
    assert_eq!(
        sessions,
        [
            (probed_at, SessionState::Locked),
            (unlocked_at, SessionState::Unlocked)
        ]
    );
    assert_eq!(machine.log(), "");
}

/// Starts WorkTime this morning as a user who first started it last night and whose computer
/// slept through the night, so that the day holds no unknown time.
fn start_after_a_night_asleep(machine: &Machine) -> LiveHost<'_> {
    let morning = machine.clock.now();
    machine.clock.set("2026-10-07T22:00:00Z".parse().unwrap());
    machine.start().record_lifecycle(&change(
        machine,
        LifecycleChange::Power(PowerState::Suspended),
    ));
    machine.clock.set(morning);
    let mut host = machine.start();
    host.record_lifecycle(&change(machine, LifecycleChange::Power(PowerState::Active)));
    host
}

fn change(machine: &Machine, change: LifecycleChange) -> LifecycleTransition {
    LifecycleTransition {
        change,
        occurred_at: machine.clock.now(),
        time_zone_id: "UTC".into(),
    }
}

fn session(machine: &Machine, state: SessionState) -> LifecycleTransition {
    change(machine, LifecycleChange::Session(state))
}

#[test]
fn a_long_lock_that_ended_is_offered_for_review_then_excluded_from_work() {
    let machine = Machine::new();
    let mut host = start_after_a_night_asleep(&machine);
    host.prepare_lifecycle(|| Some(SessionState::Unlocked), std::iter::empty());
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(60));
    host.record_lifecycle(&session(&machine, SessionState::Locked));
    machine.clock.advance(SignedDuration::from_hours(2));
    host.record_lifecycle(&session(&machine, SessionState::Unlocked));
    machine.work(&mut host, SignedDuration::from_mins(5));
    host.refresh();

    let reviews = host.closed_reviews();
    let worked = |host: &LiveHost| host.view(Language::English).worked.unwrap().worked;
    assert_eq!(
        reviews
            .iter()
            .map(|review| (review.locked_episode.start(), review.locked_episode.end()))
            .collect::<Vec<_>>(),
        [(
            "2026-10-08T09:00:00Z".parse().unwrap(),
            "2026-10-08T11:00:00Z".parse().unwrap()
        )]
    );
    // Until it is reviewed, the lock counts as work.
    assert_eq!(worked(&host), "3h\u{a0}04m");

    host.resolve_review(&ReviewCommand {
        review: reviews[0].clone(),
        action: ReviewAction::Exclude,
    });

    assert_eq!(host.closed_reviews(), []);
    assert_eq!(worked(&host), "1h\u{a0}04m");
    assert_eq!(
        host.latest_adjustment().map(ManualAdjustment::kind),
        Some(&AdjustmentKind::Reclassification {
            interval: UtcInterval::new(
                "2026-10-08T09:00:00Z".parse().unwrap(),
                "2026-10-08T11:00:00Z".parse().unwrap()
            )
            .unwrap(),
            classification: ManualClassification::Excluded,
            confirms_review_id: None,
        })
    );
    assert_eq!(machine.log(), "");
}

/// The computer slept through « Réunion » from 10:00 to 11:00: the flyout asks about it, with
/// its name and how long it lasted, until it is answered.
#[test]
fn the_flyout_asks_about_an_absence_in_a_recurring_appointment() {
    let machine = Machine::new();
    let mut host = start_after_a_night_asleep(&machine);
    let mut settings = host.settings().clone();
    settings.recurring_appointments = vec![worktime_domain::configuration::RecurringAppointment {
        id: "thursday-1000".into(),
        name: "Réunion".into(),
        weekday: jiff::civil::Weekday::Thursday,
        start: jiff::civil::time(10, 0, 0, 0),
        end: jiff::civil::time(11, 0, 0, 0),
        enabled: true,
        since: Some(jiff::civil::date(2026, 10, 1)),
        alternate_weeks_from: None,
        automatic: false,
    }];
    host.save_settings(&settings).unwrap();
    host.prepare_lifecycle(|| Some(SessionState::Unlocked), std::iter::empty());
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(120));
    host.record_lifecycle(&change(
        &machine,
        LifecycleChange::Power(PowerState::Suspended),
    ));
    machine.clock.advance(SignedDuration::from_hours(1));
    host.record_lifecycle(&change(
        &machine,
        LifecycleChange::Power(PowerState::Active),
    ));
    machine.work(&mut host, SignedDuration::from_mins(5));
    host.refresh();

    let review = host.view(Language::French).appointment_review.unwrap();
    assert_eq!(review.appointment.as_deref(), Some("Réunion"));
    assert_eq!(review.duration, SignedDuration::from_hours(1));
    host.resolve_review(&ReviewCommand {
        review,
        action: ReviewAction::ConfirmWork,
    });
    assert_eq!(host.view(Language::French).appointment_review, None);
    assert_eq!(machine.log(), "");
}

/// Records the reminders the host shows.
#[derive(Default)]
struct Reminders(std::cell::RefCell<Vec<worktime_core::notifications::Notification>>);

impl NotificationSink for Reminders {
    fn show(
        &self,
        notification: &worktime_core::notifications::Notification,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.0.borrow_mut().push(notification.clone());
        Ok(())
    }
}

#[test]
fn the_completion_reminder_is_shown_once_the_minute_it_fell_due_has_passed() {
    let machine = Machine::new();
    let reminders = Reminders::default();
    let mut host = start_after_a_night_asleep(&machine);
    host.enable_notifications(&reminders);
    host.poll();
    // 08:00 to 14:50: 10 of the 7 hours are left.
    machine.work(&mut host, SignedDuration::from_mins(6 * 60 + 50));
    host.refresh();

    machine.work(&mut host, SignedDuration::from_mins(1));
    host.refresh();
    host.refresh();

    let shown: Vec<_> = reminders
        .0
        .borrow()
        .iter()
        .map(|reminder| (reminder.triggers.clone(), reminder.remaining_minutes()))
        .collect();
    assert_eq!(
        shown,
        [(
            [worktime_domain::notifications::NotificationTrigger::CompletionApproach].into(),
            9
        )]
    );
    assert_eq!(machine.log(), "");
}

/// What the week window does with a command: record it, refresh, reload, then report.
fn run_from_week(host: &mut LiveHost, state: &mut WeekState, command: WeekCommand) {
    let result = host.run_week_command(&command);
    host.refresh();
    state.reload(host.week().unwrap(), host.recalculated());
    state.finished(result.map(|saved| {
        Recorded {
            saved,
            recalculated: host.recalculated(),
            proposal: command
                .counted_workday()
                .and_then(|workday| host.proposal(workday)),
        }
    }));
}

/// Today's slots left to settle. The days before WorkTime's first start are unknown time too.
fn pending_today(state: &WeekState) -> usize {
    state
        .timeline
        .day(state.timeline.today)
        .unwrap()
        .pending_count()
}

fn segment_of(state: &WeekState, kind: SegmentKind) -> SegmentId {
    let day = state.timeline.day(state.timeline.today).unwrap();
    day.segments
        .iter()
        .find(|segment| segment.kind == kind)
        .unwrap()
        .id(day.date)
}

#[test]
fn a_lock_that_ended_today_finishes_the_day_at_its_start_from_the_week() {
    let machine = Machine::new();
    let mut host = start_after_a_night_asleep(&machine);
    host.prepare_lifecycle(|| Some(SessionState::Unlocked), std::iter::empty());
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(60));
    host.record_lifecycle(&session(&machine, SessionState::Locked));
    machine.clock.advance(SignedDuration::from_hours(2));
    host.record_lifecycle(&session(&machine, SessionState::Unlocked));
    machine.work(&mut host, SignedDuration::from_mins(5));
    host.refresh();

    let mut state = WeekState::new(host.week().unwrap(), Language::English);
    state.open(segment_of(&state, SegmentKind::Review));
    let labels: Vec<_> = state
        .bubble()
        .unwrap()
        .actions
        .into_iter()
        .map(|action| action.label)
        .collect();
    assert_eq!(
        labels,
        [
            "Count as work",
            "Exclude from work",
            "Finish the day at\u{a0}09:00"
        ]
    );
    let command = state.choose(Answer::Finish).unwrap();
    machine.clock.advance(SignedDuration::from_mins(1));
    run_from_week(&mut host, &mut state, command);

    assert_eq!(
        state.info_bar().unwrap().message,
        "Day finished at\u{a0}09:00."
    );
    assert_eq!(host.closed_reviews(), []);
    assert_eq!(pending_today(&state), 0);
    assert_eq!(host.tracking(), TrackingState::Finished);
    assert_eq!(machine.log(), "");
}

/// Without a power history, the night before the first activity is unknown time: the week
/// starts the day at the arrival and asks only about the gap after it (#305).
#[test]
fn the_week_leaves_the_night_before_the_first_activity_out() {
    let machine = Machine::new();
    machine.clock.set("2026-10-08T09:30:00Z".parse().unwrap());
    let mut host = machine.start();
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(60));
    machine.clock.advance(SignedDuration::from_mins(30));
    machine.work(&mut host, SignedDuration::from_mins(30));
    host.refresh();

    let state = WeekState::new(host.week().unwrap(), Language::English);
    let day = state.timeline.day(state.timeline.today).unwrap();
    assert_eq!(
        day.segments
            .iter()
            .map(|segment| (segment.kind, segment.interval.start()))
            .collect::<Vec<_>>(),
        [
            (SegmentKind::Work, "2026-10-08T09:30:00Z".parse().unwrap()),
            (SegmentKind::Gap, "2026-10-08T10:30:00Z".parse().unwrap()),
            (SegmentKind::Work, "2026-10-08T11:00:05Z".parse().unwrap()),
        ]
    );
    assert_eq!(pending_today(&state), 1);
}

#[test]
fn a_gap_settled_from_the_week_is_withdrawn_by_undo() {
    let machine = Machine::new();
    let mut host = start_after_a_night_asleep(&machine);
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(60));
    // Nothing samples the activity for half an hour: unknown time.
    machine.clock.advance(SignedDuration::from_mins(30));
    machine.work(&mut host, SignedDuration::from_mins(30));
    host.refresh();

    let mut state = WeekState::new(host.week().unwrap(), Language::English);
    assert_eq!(pending_today(&state), 1);
    state.open(segment_of(&state, SegmentKind::Gap));
    let command = state.choose(Answer::Count).unwrap();
    machine.clock.advance(SignedDuration::from_mins(1));
    run_from_week(&mut host, &mut state, command);
    let bar = state.info_bar().unwrap();
    assert_eq!(
        bar.message,
        format!("09:00–09:30 counted as work ({}).", keep_together("30m"))
    );
    assert!(bar.can_undo);
    assert_eq!(pending_today(&state), 0);

    machine.clock.advance(SignedDuration::from_mins(1));
    let undo = state.undo().unwrap();
    run_from_week(&mut host, &mut state, undo);
    let bar = state.info_bar().unwrap();
    assert_eq!((bar.message.as_str(), bar.can_undo), ("Undone.", false));
    assert_eq!(pending_today(&state), 1);
    assert_eq!(machine.log(), "");
}

/// Works from 08:00 until `lock`, leaves the session locked until `unlock` (still locked when
/// `None`), then works five minutes more and refreshes.
fn lock_between<'m>(machine: &'m Machine, lock: &str, unlock: Option<&str>) -> LiveHost<'m> {
    let mut host = start_after_a_night_asleep(machine);
    host.prepare_lifecycle(|| Some(SessionState::Unlocked), std::iter::empty());
    host.poll();
    let lock: Timestamp = lock.parse().unwrap();
    machine.work(&mut host, lock.duration_since(machine.clock.now()));
    host.record_lifecycle(&session(machine, SessionState::Locked));
    match unlock {
        Some(unlock) => {
            machine.clock.set(unlock.parse().unwrap());
            host.record_lifecycle(&session(machine, SessionState::Unlocked));
            machine.work(&mut host, SignedDuration::from_mins(5));
        }
        None => machine.clock.advance(SignedDuration::from_mins(150)),
    }
    host.refresh();
    host
}

fn bubble_of_review(state: &mut WeekState) -> (String, Vec<(String, bool)>) {
    state.open(segment_of(state, SegmentKind::Review));
    let bubble = state.bubble().unwrap();
    (
        format!("{} · {}", bubble.title, bubble.duration),
        bubble
            .actions
            .into_iter()
            .map(|action| (action.label, action.enabled))
            .collect(),
    )
}

fn worked_today(host: &LiveHost) -> String {
    host.view(Language::English)
        .worked
        .unwrap()
        .worked
        .to_string()
}

#[test]
fn confirming_a_lock_across_lunch_shows_and_credits_the_whole_lock() {
    let machine = Machine::new();
    let mut host = lock_between(
        &machine,
        "2026-10-08T11:30:00Z",
        Some("2026-10-08T14:30:00Z"),
    );
    let mut state = WeekState::new(host.week().unwrap(), Language::English);
    let (subtitle, actions) = bubble_of_review(&mut state);
    assert_eq!(
        subtitle,
        format!("11:30–14:30 · {}", keep_together("3h 00m"))
    );
    assert_eq!(
        actions,
        [
            ("Count as work".to_owned(), true),
            ("Exclude from work".to_owned(), true),
            (
                format!("Finish the day at{}11:30", keep_together(" ")),
                true
            ),
        ]
    );

    let command = state.choose(Answer::Count).unwrap();
    machine.clock.advance(SignedDuration::from_mins(1));
    run_from_week(&mut host, &mut state, command);
    assert_eq!(
        state.info_bar().unwrap().message,
        "11:30–14:30 confirmed as work."
    );
    // 08:00 to 14:35, lunch included: the lock was confirmed whole. The 5 s before the first
    // sample after the unlock stay unknown.
    assert_eq!(worked_today(&host), keep_together("6h 34m"));
    assert_eq!(machine.log(), "");
}

/// The tray menu's submenus, each as its label and the labels of its items.
fn tray_submenus(host: &LiveHost, language: Language) -> Vec<(String, Vec<String>)> {
    let state = MenuState {
        tracking: Tracking::new(host.tracking(), true),
        reviews: host.closed_reviews(),
        has_latest_adjustment: host.latest_adjustment().is_some(),
    };
    tray_model::menu(&state, language)
        .into_iter()
        .filter_map(|entry| match entry {
            MenuEntry::Submenu { label, items } => Some((
                label,
                items
                    .into_iter()
                    .map(|item| match item {
                        MenuEntry::Item { label, .. } => label,
                        other => format!("{other:?}"),
                    })
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn the_tray_names_a_lock_across_lunch_by_the_whole_lock_as_the_week_view() {
    let machine = Machine::new();
    let host = lock_between(
        &machine,
        "2026-10-08T11:30:00Z",
        Some("2026-10-08T14:30:00Z"),
    );

    assert_eq!(
        tray_submenus(&host, Language::French),
        [(
            "Verrouillage à vérifier · 11:30–14:30".to_owned(),
            vec![
                "Oui, je travaillais".to_owned(),
                "Ce n’était pas du travail".to_owned(),
                "Terminer la journée à\u{a0}11:30".to_owned(),
            ]
        )]
    );
    assert_eq!(machine.log(), "");
}

#[test]
fn the_tray_offers_a_submenu_only_once_the_lock_is_over() {
    let going_on = Machine::new();
    let host = lock_between(&going_on, "2026-10-08T10:00:00Z", None);
    assert_eq!(tray_submenus(&host, Language::English), []);

    let over = Machine::new();
    let host = lock_between(&over, "2026-10-08T10:00:00Z", Some("2026-10-08T12:30:00Z"));
    assert_eq!(
        tray_submenus(&host, Language::English)
            .into_iter()
            .map(|(label, _)| label)
            .collect::<Vec<_>>(),
        ["Lock to review · 10:00\u{a0}AM–12:30\u{a0}PM"]
    );
    assert_eq!(going_on.log(), "");
    assert_eq!(over.log(), "");
}

#[test]
fn a_lock_that_started_during_lunch_finishes_the_day_at_its_start() {
    let machine = Machine::new();
    let mut host = lock_between(
        &machine,
        "2026-10-08T13:00:00Z",
        // Outside lunch for longer than the review threshold: not a late lunch (#136).
        Some("2026-10-08T15:40:00Z"),
    );
    let mut state = WeekState::new(host.week().unwrap(), Language::English);
    let (subtitle, actions) = bubble_of_review(&mut state);
    assert_eq!(
        subtitle,
        format!("13:00–15:40 · {}", keep_together("2h 40m"))
    );
    assert_eq!(
        actions[2],
        (
            format!("Finish the day at{}13:00", keep_together(" ")),
            true
        )
    );

    let command = state.choose(Answer::Finish).unwrap();
    machine.clock.advance(SignedDuration::from_mins(1));
    run_from_week(&mut host, &mut state, command);
    assert_eq!(
        state.info_bar().unwrap().message,
        format!("Day finished at{}13:00.", keep_together(" "))
    );
    assert_eq!(host.tracking(), TrackingState::Finished);
    // 08:00 to 13:00 at the keyboard, lunch included, then nothing after the lock.
    assert_eq!(worked_today(&host), keep_together("5h 00m"));
    assert_eq!(machine.log(), "");
}

#[test]
fn a_lock_still_going_on_into_lunch_cannot_be_answered_yet() {
    let machine = Machine::new();
    let mut host = lock_between(&machine, "2026-10-08T10:00:00Z", None);
    let mut state = WeekState::new(host.week().unwrap(), Language::English);
    let (subtitle, actions) = bubble_of_review(&mut state);
    assert_eq!(
        subtitle,
        // The lock goes on into lunch, which the day does not draw after it.
        format!("10:00–12:00 · {}", keep_together("2h 00m"))
    );
    assert_eq!(
        actions,
        [
            ("Count as work".to_owned(), false),
            ("Exclude from work".to_owned(), false),
        ]
    );
    assert_eq!(state.choose(Answer::Count), None);
    assert_eq!(state.choose(Answer::Finish), None);

    // The ledger refuses both: the lock has not ended.
    let workday = state
        .timeline
        .day(state.timeline.today)
        .unwrap()
        .figures
        .as_ref()
        .unwrap()
        .workday
        .clone();
    let review = worktime_domain::accounting::LongLockReviewId::new("session-lock:3").unwrap();
    for command in [
        WeekCommand::Range {
            workday: workday.clone(),
            range: state.selection().unwrap().range,
            classification: ManualClassification::Credited,
        },
        WeekCommand::FinishAtLockStart { workday, review },
    ] {
        assert_eq!(
            host.run_week_command(&command),
            Err(Key::HistoryPeriodResolved)
        );
    }
}

/// The same lock confirmed as work three Thursdays in a row: the third result proposes an
/// appointment on its hours, and none once the user turned it down.
#[test]
fn the_third_thursday_corrected_alike_proposes_an_appointment() {
    let machine = Machine::new();
    let mut host = start_after_a_night_asleep(&machine);
    host.prepare_lifecycle(|| Some(SessionState::Unlocked), std::iter::empty());
    let mut proposals = Vec::new();
    for day in ["2026-10-08", "2026-10-15", "2026-10-22"] {
        let at = |time: &str| -> Timestamp { format!("{day}T{time}:00Z").parse().unwrap() };
        if day != "2026-10-08" {
            host.record_lifecycle(&change(
                &machine,
                LifecycleChange::Power(PowerState::Suspended),
            ));
            machine.clock.set(at("08:00"));
            host.record_lifecycle(&change(
                &machine,
                LifecycleChange::Power(PowerState::Active),
            ));
        }
        host.poll();
        machine.work(&mut host, at("14:02").duration_since(machine.clock.now()));
        host.record_lifecycle(&session(&machine, SessionState::Locked));
        machine.clock.set(at("15:56"));
        host.record_lifecycle(&session(&machine, SessionState::Unlocked));
        machine.work(&mut host, SignedDuration::from_mins(5));
        host.refresh();
        let mut state = WeekState::new(host.week().unwrap(), Language::French);
        state.open(segment_of(&state, SegmentKind::Review));
        let command = state.choose(Answer::Count).unwrap();
        run_from_week(&mut host, &mut state, command);
        proposals.push(state.proposal().cloned());
        machine.clock.set(at("18:00"));
    }

    assert_eq!(proposals[..2], [None, None]);
    let proposal = proposals[2].clone().expect("the third Thursday proposes");
    assert_eq!(
        (proposal.weekday, proposal.hours()),
        (
            jiff::civil::Weekday::Thursday,
            (
                jiff::civil::time(14, 0, 0, 0),
                jiff::civil::time(16, 0, 0, 0)
            )
        )
    );
    assert_eq!(proposal.alternate_weeks_from, None);
    assert_eq!(proposal.occurrences.len(), 3);

    let mut settings = host.settings().clone();
    settings
        .appointment_proposals
        .dismiss(proposal.dismissal(None), jiff::civil::date(2026, 10, 22));
    host.save_settings(&settings).unwrap();
    let workday = host
        .week()
        .unwrap()
        .day(jiff::civil::date(2026, 10, 22))
        .unwrap()
        .figures
        .as_ref()
        .unwrap()
        .workday
        .clone();
    assert_eq!(host.proposal(&workday), None);
    assert_eq!(machine.log(), "");
}

/// The Thursday 10:00–11:00 appointment, counted without asking when `automatic`.
fn thursday_meeting(automatic: bool) -> worktime_domain::configuration::RecurringAppointment {
    worktime_domain::configuration::RecurringAppointment {
        id: "thursday-1000".into(),
        name: "Réunion".into(),
        weekday: jiff::civil::Weekday::Thursday,
        start: jiff::civil::time(10, 0, 0, 0),
        end: jiff::civil::time(11, 0, 0, 0),
        enabled: true,
        since: Some(jiff::civil::date(2026, 9, 1)),
        alternate_weeks_from: None,
        automatic,
    }
}

/// Works from 08:00 to 10:00, sleeps through the meeting, then works five minutes more.
fn sleep_through_the_meeting(machine: &Machine, host: &mut LiveHost) {
    host.prepare_lifecycle(|| Some(SessionState::Unlocked), std::iter::empty());
    host.poll();
    machine.work(host, SignedDuration::from_mins(120));
    host.record_lifecycle(&change(
        machine,
        LifecycleChange::Power(PowerState::Suspended),
    ));
    machine.clock.advance(SignedDuration::from_hours(1));
    host.record_lifecycle(&change(machine, LifecycleChange::Power(PowerState::Active)));
    machine.work(host, SignedDuration::from_mins(5));
}

/// « Compter automatiquement »: the absence is counted as work once the user is back, the
/// flyout says so, and « Ce n’était pas du travail » takes the hour back, for good.
#[test]
fn an_automatic_appointment_counts_the_absence_once_the_user_is_back() {
    let machine = Machine::new();
    let mut host = start_after_a_night_asleep(&machine);
    let mut settings = host.settings().clone();
    settings.recurring_appointments = vec![thursday_meeting(true)];
    host.save_settings(&settings).unwrap();
    sleep_through_the_meeting(&machine, &mut host);
    host.refresh();

    let view = host.view(Language::French);
    assert_eq!(view.appointment_review, None);
    let counted = view.counted_absence.expect("the absence is counted");
    assert_eq!(counted.appointment, "Réunion");
    assert_eq!(counted.duration, SignedDuration::from_hours(1));
    assert_eq!(worked_today(&host), keep_together("3h 05m"));
    assert_eq!(
        pending_today(&WeekState::new(host.week().unwrap(), Language::French)),
        0
    );

    host.refresh();
    let recorded = ManualAdjustmentRepository::new(&machine.connection)
        .read_for_workday(&counted.workday)
        .unwrap();
    assert_eq!(recorded.len(), 1);

    host.undo_count(&counted.adjustment_id);
    assert_eq!(host.view(Language::French).counted_absence, None);
    assert_eq!(host.view(Language::French).appointment_review, None);
    assert_eq!(worked_today(&host), keep_together("2h 05m"));
    host.refresh();
    assert_eq!(worked_today(&host), keep_together("2h 05m"));
    assert_eq!(machine.log(), "");
}

/// Until the user is back, nothing is recorded: the absence stays counted for now.
#[test]
fn an_automatic_appointment_records_nothing_while_the_user_is_away() {
    let machine = Machine::new();
    let mut host = start_after_a_night_asleep(&machine);
    let mut settings = host.settings().clone();
    settings.recurring_appointments = vec![thursday_meeting(true)];
    host.save_settings(&settings).unwrap();
    host.prepare_lifecycle(|| Some(SessionState::Unlocked), std::iter::empty());
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(120));
    host.record_lifecycle(&change(
        &machine,
        LifecycleChange::Power(PowerState::Suspended),
    ));
    machine.clock.advance(SignedDuration::from_mins(30));
    host.refresh();

    assert_eq!(host.view(Language::French).counted_absence, None);
    let workday = host
        .week()
        .unwrap()
        .day(jiff::civil::date(2026, 10, 8))
        .unwrap()
        .figures
        .as_ref()
        .unwrap()
        .workday
        .clone();
    assert!(
        ManualAdjustmentRepository::new(&machine.connection)
            .read_for_workday(&workday)
            .unwrap()
            .is_empty()
    );
}

/// Thursday's adjustments, as the store holds them.
fn thursday_adjustments(machine: &Machine, host: &LiveHost) -> Vec<ManualAdjustment> {
    let workday = host
        .week()
        .unwrap()
        .day(jiff::civil::date(2026, 10, 8))
        .unwrap()
        .figures
        .as_ref()
        .unwrap()
        .workday
        .clone();
    ManualAdjustmentRepository::new(&machine.connection)
        .read_for_workday(&workday)
        .unwrap()
}

fn pending_on_thursday(host: &LiveHost) -> usize {
    host.week()
        .unwrap()
        .day(jiff::civil::date(2026, 10, 8))
        .unwrap()
        .pending_count()
}

/// Sleeps from the end of Thursday to 08:00 on Friday, then works five minutes.
fn sleep_until_friday(machine: &Machine, host: &mut LiveHost) {
    host.record_lifecycle(&change(
        machine,
        LifecycleChange::Power(PowerState::Suspended),
    ));
    machine.clock.set("2026-10-09T08:00:00Z".parse().unwrap());
    host.record_lifecycle(&change(machine, LifecycleChange::Power(PowerState::Active)));
    machine.work(host, SignedDuration::from_mins(5));
}

/// An absence left unanswered on Thursday counts once its appointment turns automatic on
/// Friday, and only once.
#[test]
fn turning_an_appointment_automatic_counts_the_absences_earlier_in_the_week() {
    let machine = Machine::new();
    let mut host = start_after_a_night_asleep(&machine);
    let mut settings = host.settings().clone();
    settings.recurring_appointments = vec![thursday_meeting(false)];
    host.save_settings(&settings).unwrap();
    sleep_through_the_meeting(&machine, &mut host);
    host.refresh();
    assert_eq!(pending_on_thursday(&host), 1);
    sleep_until_friday(&machine, &mut host);
    host.refresh();
    assert_eq!(pending_on_thursday(&host), 1);

    settings.recurring_appointments = vec![thursday_meeting(true)];
    host.save_settings(&settings).unwrap();
    host.refresh();
    host.refresh();

    let recorded = thursday_adjustments(&machine, &host);
    assert_eq!(recorded.len(), 1);
    assert!(
        recorded[0]
            .id()
            .as_str()
            .starts_with("appointment-auto:2026-10-08:")
    );
    assert_eq!(
        recorded[0].interval().map(|interval| interval.duration()),
        Some(SignedDuration::from_hours(1))
    );
    assert_eq!(pending_on_thursday(&host), 0);
    assert_eq!(host.view(Language::French).counted_absence, None);
    assert_eq!(machine.log(), "");
}

/// An absence in an automatic appointment that ends on the next day counts on its own day.
#[test]
fn an_absence_that_ends_the_next_day_counts_on_its_own_day() {
    let machine = Machine::new();
    let mut host = start_after_a_night_asleep(&machine);
    let mut settings = host.settings().clone();
    settings.recurring_appointments = vec![thursday_meeting(true)];
    host.save_settings(&settings).unwrap();
    host.prepare_lifecycle(|| Some(SessionState::Unlocked), std::iter::empty());
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(120));
    sleep_until_friday(&machine, &mut host);
    host.refresh();

    let recorded = thursday_adjustments(&machine, &host);
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(pending_on_thursday(&host), 0);
    assert_eq!(machine.log(), "");
}

/// The closed callout of an absence counted automatically stays closed.
#[test]
fn closing_the_callout_of_a_counted_absence_keeps_the_count() {
    let machine = Machine::new();
    let mut host = start_after_a_night_asleep(&machine);
    let mut settings = host.settings().clone();
    settings.recurring_appointments = vec![thursday_meeting(true)];
    host.save_settings(&settings).unwrap();
    sleep_through_the_meeting(&machine, &mut host);
    host.refresh();
    let counted = host.view(Language::French).counted_absence.unwrap();
    host.hide_count(&counted.adjustment_id);
    host.refresh();
    assert_eq!(host.view(Language::French).counted_absence, None);
    assert_eq!(worked_today(&host), keep_together("3h 05m"));
}

/// The fourth « Oui, je travaillais » in a row on the meeting offers to count it automatically,
/// once: closed, the offer stays closed.
#[test]
fn four_confirmations_in_a_row_offer_to_count_automatically() {
    let machine = Machine::new();
    let mut host = start_after_a_night_asleep(&machine);
    let mut settings = host.settings().clone();
    settings.recurring_appointments = vec![thursday_meeting(false)];
    host.save_settings(&settings).unwrap();
    host.prepare_lifecycle(|| Some(SessionState::Unlocked), std::iter::empty());
    let mut offers = Vec::new();
    for day in ["2026-10-08", "2026-10-15", "2026-10-22", "2026-10-29"] {
        let at = |time: &str| -> Timestamp { format!("{day}T{time}:00Z").parse().unwrap() };
        if day != "2026-10-08" {
            host.record_lifecycle(&change(
                &machine,
                LifecycleChange::Power(PowerState::Suspended),
            ));
            machine.clock.set(at("08:00"));
            host.record_lifecycle(&change(
                &machine,
                LifecycleChange::Power(PowerState::Active),
            ));
        }
        host.poll();
        machine.work(&mut host, at("10:00").duration_since(machine.clock.now()));
        host.record_lifecycle(&change(
            &machine,
            LifecycleChange::Power(PowerState::Suspended),
        ));
        machine.clock.set(at("11:00"));
        host.record_lifecycle(&change(
            &machine,
            LifecycleChange::Power(PowerState::Active),
        ));
        machine.work(&mut host, SignedDuration::from_mins(5));
        host.refresh();
        let review = host.view(Language::French).appointment_review.unwrap();
        host.resolve_review(&ReviewCommand {
            review,
            action: ReviewAction::ConfirmWork,
        });
        offers.push(host.view(Language::French).trust_offer);
        machine.clock.set(at("18:00"));
    }

    assert_eq!(offers[..3], [None, None, None]);
    let offer = offers[3].clone().expect("the fourth confirmation offers");
    assert_eq!(
        (offer.appointment_id.as_str(), offer.streak),
        ("thursday-1000", 4)
    );
    host.hide_offer(&offer.appointment_id);
    assert_eq!(host.view(Language::French).trust_offer, None);
    assert_eq!(machine.log(), "");
}

/// Thursday's appointment absence as the week offers it.
fn thursday_absence(host: &LiveHost) -> Lock {
    host.week()
        .unwrap()
        .day(jiff::civil::date(2026, 10, 8))
        .unwrap()
        .segments
        .iter()
        .find_map(|segment| segment.review.clone())
        .expect("the absence awaits an answer")
}

/// An absence read while it goes on, then once it ended, is measured on the day as it stands
/// at each read, though its summary was not refreshed in between.
#[test]
fn an_absence_that_ended_since_the_last_refresh_offers_its_gains() {
    let machine = Machine::new();
    let mut host = start_after_a_night_asleep(&machine);
    let mut settings = host.settings().clone();
    settings.recurring_appointments = vec![thursday_meeting(false)];
    host.save_settings(&settings).unwrap();
    host.prepare_lifecycle(|| Some(SessionState::Unlocked), std::iter::empty());
    host.poll();
    machine.work(&mut host, SignedDuration::from_mins(120));
    host.record_lifecycle(&change(
        &machine,
        LifecycleChange::Power(PowerState::Suspended),
    ));
    machine.clock.advance(SignedDuration::from_hours(1));
    host.refresh();
    let away = thursday_absence(&host);
    assert!(away.is_open);
    assert_eq!(away.gains, None);

    host.record(&change(
        &machine,
        LifecycleChange::Power(PowerState::Active),
    ));
    machine.work(&mut host, SignedDuration::from_mins(5));
    let back = thursday_absence(&host);

    assert!(!back.is_open);
    assert_eq!(
        back.gains,
        Some(Gains {
            count: SignedDuration::ZERO,
            exclude: -SignedDuration::from_hours(1),
        })
    );
}
