//! What the tray and the week offer on the corrections of a live day, and what saving one
//! the ledger refuses tells.

use std::path::Path;

use jiff::SignedDuration;
use worktime_domain::time::{Clock as _, FrozenClock};
use worktime_store::adjustments::ManualAdjustmentRepository;
use worktime_store::data_directory::DataDirectory;
use worktime_windows::PowerState;
use worktime_windows::lifecycle::{LifecycleChange, LifecycleTransition};

use super::*;
use crate::adjust::AdjustForm;
use crate::tray_model::ClosedReview;

/// Whether the tray menu shows « Modifier le dernier ajustement… », and enabled.
fn edit_latest_item(today: &Today) -> Option<bool> {
    today.menu().into_iter().find_map(|entry| match entry {
        MenuEntry::Item {
            command: MenuCommand::EditLatestAdjustment,
            enabled,
            ..
        } => Some(enabled),
        _ => None,
    })
}

/// The tray edits the latest correction of the published day, only on a working day by the
/// calendar, as `EditTimeAsync` of the .NET app checks `CanAddTime`: Saturday greys it while
/// Friday's summary, and its correction, stays published.
#[test]
fn the_tray_edits_the_latest_correction_only_on_a_working_day() {
    let folder = tempfile::tempdir().unwrap();
    // Friday 9 October 2026.
    let clock = Box::leak(Box::new(FrozenClock::new(
        "2026-10-09T16:00:00Z".parse().unwrap(),
    )));
    let mut today = start(folder.path(), clock);
    assert_eq!(edit_latest_item(&today), None);
    let friday = LocalWorkday::new(jiff::civil::date(2026, 10, 9), "UTC");
    let credit = |id: &str, minutes| {
        ManualAdjustment::duration_work(
            worktime_domain::accounting::AdjustmentId::new(id).unwrap(),
            friday.clone(),
            SignedDuration::from_mins(minutes),
            clock.now(),
        )
        .unwrap()
    };
    today.save_adjustment(&credit("earlier", 20)).unwrap();
    clock.advance(SignedDuration::from_mins(1));
    today.save_adjustment(&credit("latest", 10)).unwrap();
    assert_eq!(edit_latest_item(&today), Some(true));
    assert_eq!(today.latest_editable(), Editable::new(credit("latest", 10)));

    rename(folder.path(), "ActivitySpan", "Hidden");
    clock.set("2026-10-10T10:00:00Z".parse().unwrap());
    today.refresh();
    assert_eq!(edit_latest_item(&today), Some(false));
    assert_eq!(today.latest_editable(), None);
}

/// Refreshes failing since Friday keep its summary published on Monday, and the tray still
/// offers its latest correction. Friday is no longer in the current week, so its edit is not
/// saved, and the window says its week has ended.
#[test]
fn an_edit_of_last_weeks_correction_says_its_week_has_ended() {
    let folder = tempfile::tempdir().unwrap();
    // Friday 9 October 2026.
    let clock = Box::leak(Box::new(FrozenClock::new(
        "2026-10-09T16:00:00Z".parse().unwrap(),
    )));
    let mut today = start(folder.path(), clock);
    let friday = LocalWorkday::new(jiff::civil::date(2026, 10, 9), "UTC");
    let credit = ManualAdjustment::duration_work(
        AdjustmentId::new("friday").unwrap(),
        friday.clone(),
        SignedDuration::from_mins(20),
        clock.now(),
    )
    .unwrap();
    today.save_adjustment(&credit).unwrap();

    rename(folder.path(), "ActivitySpan", "Hidden");
    clock.set("2026-10-12T08:00:00Z".parse().unwrap());
    today.refresh();
    assert_eq!(edit_latest_item(&today), Some(true));
    let edited = today.latest_editable().unwrap();
    assert_eq!(edited.workday(), &friday);
    let target = today.adjust_target(Some(friday)).unwrap();
    for (language, told) in [
        (
            Language::English,
            "This day is in a week that has ended and can no longer be corrected.",
        ),
        (
            Language::French,
            "Ce jour fait partie d’une semaine terminée et ne peut plus être corrigé.",
        ),
    ] {
        let mut form = AdjustForm::new(
            target.clone(),
            Seed::Edit(edited.clone()),
            AdjustmentId::new("longer").unwrap(),
            today.adjustment_time(),
            language,
        );
        form.duration = "0:30".into();
        let replacement = form.submit().unwrap();
        let saved = today.save_adjustment(&replacement).map(|_| true);
        assert!(saved.is_err());
        assert_eq!(form.finished(saved), None);
        assert_eq!(
            form.page().callout.map(|callout| callout.message),
            Some(told)
        );
    }
}

/// Today's credited time, the time asleep it shows, and the slots left to settle.
fn today_totals(today: &Today) -> (SignedDuration, SignedDuration, usize) {
    let week = today.week().unwrap();
    let day = week.day(week.today).unwrap();
    let asleep = day
        .segments
        .iter()
        .filter(|segment| segment.kind == SegmentKind::Sleep)
        .map(|segment| segment.duration())
        .sum();
    (
        day.figures.as_ref().unwrap().credited,
        asleep,
        day.pending_count(),
    )
}

/// Today after an hour at the keyboard, then a lock from 09:00 to 11:00 with half an hour
/// asleep in it, closed five minutes before now.
fn a_closed_lock(folder: &Path, clock: &'static FrozenClock) -> (Today, ClosedReview) {
    let mut today = start(folder, clock);
    let transition = |change| LifecycleTransition {
        change,
        occurred_at: clock.now(),
        time_zone_id: "UTC".into(),
    };
    let work = |host: &mut LiveHost, minutes| {
        let end = clock.now() + SignedDuration::from_mins(minutes);
        while clock.now() < end {
            clock.advance(SignedDuration::from_secs(5));
            host.poll();
        }
    };
    let host = today.live().unwrap();
    host.prepare_lifecycle(|| Some(SessionState::Unlocked), std::iter::empty());
    work(host, 60);
    host.record_lifecycle(&transition(LifecycleChange::Session(SessionState::Locked)));
    for (minutes, change) in [
        (60, LifecycleChange::Power(PowerState::Suspended)),
        (30, LifecycleChange::Power(PowerState::Active)),
        (30, LifecycleChange::Session(SessionState::Unlocked)),
    ] {
        clock.advance(SignedDuration::from_mins(minutes));
        host.record_lifecycle(&transition(change));
    }
    work(host, 5);
    today.refresh();
    let [review] = <[_; 1]>::try_from(today.live().unwrap().closed_reviews()).unwrap();
    (today, review)
}

/// The bubble's actions on the correction `id` of today, by label.
fn correction_actions(today: &Today, id: &AdjustmentId) -> Vec<(String, bool)> {
    let mut state = WeekState::new(today.week().unwrap(), Language::English);
    let segment = state
        .timeline
        .day(state.timeline.today)
        .unwrap()
        .segments
        .iter()
        .find(|segment| segment.adjustment_id.as_ref() == Some(id))
        .map(|segment| segment.id(state.timeline.today))
        .unwrap();
    state.open(segment);
    state
        .bubble()
        .unwrap()
        .actions
        .iter()
        .map(|action| (action.label.clone(), action.enabled))
        .collect()
}

/// A confirmation of a long lock recorded under an id of its own, as no resolution of this app
/// names it: the week still knows it from its kind, and offers neither « Modifier… » nor
/// « Retirer la correction » on it, which the form and the ledger would refuse.
#[test]
fn a_long_lock_confirmation_under_any_id_is_not_edited() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let (mut today, review) = a_closed_lock(folder.path(), clock);
    let confirmation = ManualAdjustment::long_lock_confirmation(
        AdjustmentId::new("confirmed-elsewhere").unwrap(),
        review.workday.clone(),
        review.locked_episode,
        clock.now(),
        review.id.clone(),
    )
    .unwrap();
    let directory = DataDirectory::from(folder.path().to_owned());
    let connection = worktime_store::database::open(&directory.database_file()).unwrap();
    ManualAdjustmentRepository::new(&connection)
        .append(&confirmation)
        .unwrap();
    today.refresh();
    assert_eq!(today.live().unwrap().closed_reviews(), []);

    assert_eq!(
        correction_actions(&today, confirmation.id()),
        [
            ("Exclude from work".to_owned(), false),
            ("Remove the correction".to_owned(), false)
        ]
    );
    let mut state = WeekState::new(today.week().unwrap(), Language::English);
    state.toggle_details();
    let details = state.details().unwrap();
    let row = details
        .corrections
        .iter()
        .find(|row| row.id == *confirmation.id())
        .unwrap();
    assert_eq!(
        (row.edit.enabled, row.note),
        (
            false,
            Some("Correction confirmed in a lock review. It can’t be changed anymore.")
        )
    );
    assert_eq!(state.edit_listed(confirmation.id()), None);
}

/// A lock from 09:00 to 11:00 with half an hour asleep in it, confirmed as work: the time asleep
/// stays uncounted. A replacement would have to confirm the same review, which the ledger
/// refuses once the review is resolved, and a plain reclassification would count the time
/// asleep. So neither the tray nor « Modifier… » of the week opens the confirmation, and the day
/// keeps its totals and its resolved review.
#[test]
fn a_long_lock_confirmation_is_not_edited() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let (mut today, review) = a_closed_lock(folder.path(), clock);
    today.resolve_review(&ReviewCommand {
        review,
        action: ReviewAction::ConfirmWork,
    });
    let confirmed = today_totals(&today);
    assert_eq!(confirmed.1, SignedDuration::from_mins(30));
    let confirmation = today.live().unwrap().latest_adjustment().cloned().unwrap();

    assert_eq!(edit_latest_item(&today), None);
    let mut state = WeekState::new(today.week().unwrap(), Language::English);
    let segment = state
        .timeline
        .day(state.timeline.today)
        .unwrap()
        .segments
        .iter()
        .find(|segment| segment.adjustment_id.as_ref() == Some(confirmation.id()))
        .map(|segment| segment.id(state.timeline.today))
        .unwrap();
    state.open(segment);
    assert_eq!(
        state
            .bubble()
            .unwrap()
            .actions
            .iter()
            .map(|action| (action.label.as_str(), action.enabled))
            .collect::<Vec<_>>(),
        [
            ("Exclude from work", false),
            ("Remove the correction", false)
        ]
    );
    assert_eq!(
        today.editable(confirmation.workday(), confirmation.id()),
        Err(Key::HistoryCorrectionFailed)
    );
    clock.advance(SignedDuration::from_mins(1));
    let same_review = ManualAdjustment::new(
        worktime_domain::accounting::AdjustmentId::new("replacement").unwrap(),
        confirmation.workday().clone(),
        clock.now(),
        Some(confirmation.id().clone()),
        confirmation.kind().clone(),
    )
    .unwrap();
    assert_eq!(
        today.save_adjustment(&same_review),
        Err(Key::AdjustTimeErrorSubmissionFailed),
        "the ledger refuses a second confirmation of a resolved review"
    );
    assert_eq!(today_totals(&today), confirmed);
    assert_eq!(today.live().unwrap().closed_reviews(), []);
}

/// The half hour asleep in a lock confirmed as work, read back from the ledger: a piece of the
/// confirmation, left to settle nowhere, on which neither answer is offered.
#[test]
fn sleep_in_a_confirmed_lock_takes_no_answer_from_the_ledger() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let (mut today, review) = a_closed_lock(folder.path(), clock);
    today.resolve_review(&ReviewCommand {
        review,
        action: ReviewAction::ConfirmWork,
    });
    let confirmation = today.live().unwrap().latest_adjustment().cloned().unwrap();

    let mut state = WeekState::new(today.week().unwrap(), Language::English);
    let day = state.timeline.day(state.timeline.today).unwrap();
    let asleep = day
        .segments
        .iter()
        .find(|segment| segment.kind == SegmentKind::Sleep)
        .unwrap();
    assert_eq!(asleep.adjustment_id.as_ref(), Some(confirmation.id()));
    assert!(!asleep.needs_action);
    let asleep = asleep.id(state.timeline.today);
    state.open(asleep);
    let bubble = state.bubble().unwrap();
    assert!(bubble.fixed);
    assert!(bubble.actions.iter().all(|action| !action.enabled));
}

/// A lock with its sleep declared as work: counting the lock takes the declaration out and
/// leaves the sleep uncounted, and the bubble says so before the answer, as the ledger found
/// on trial.
#[test]
fn counting_a_lock_previews_what_the_ledger_records() {
    let folder = tempfile::tempdir().unwrap();
    let clock = clock();
    let (mut today, review) = a_closed_lock(folder.path(), clock);
    let week = today.week().unwrap();
    let asleep = week
        .day(week.today)
        .unwrap()
        .segments
        .iter()
        .find(|segment| segment.kind == SegmentKind::Sleep)
        .unwrap()
        .interval;
    let directory = DataDirectory::from(folder.path().to_owned());
    let connection = worktime_store::database::open(&directory.database_file()).unwrap();
    ManualAdjustmentRepository::new(&connection)
        .append(
            &ManualAdjustment::interval_work(
                AdjustmentId::new("declared").unwrap(),
                review.workday.clone(),
                asleep,
                clock.now(),
            )
            .unwrap(),
        )
        .unwrap();
    clock.advance(SignedDuration::from_mins(1));
    today.refresh();

    let mut state = WeekState::new(today.week().unwrap(), Language::English);
    let lock = state
        .timeline
        .day(state.timeline.today)
        .unwrap()
        .segments
        .iter()
        .find(|segment| segment.review.is_some())
        .unwrap()
        .id(state.timeline.today);
    state.open(lock);
    let preview = state.bubble().unwrap().preview.unwrap();
    let command = state.choose(Answer::Count).unwrap();
    today.run_week(&command).unwrap();
    let week = today.week().unwrap();
    let after = week
        .day(week.today)
        .unwrap()
        .figures
        .as_ref()
        .unwrap()
        .credited;

    let after =
        crate::time_text::minutes_text(after.round(jiff::Unit::Minute).unwrap(), Language::English);
    assert!(preview.contains(&after), "{preview} / {after}");
}
