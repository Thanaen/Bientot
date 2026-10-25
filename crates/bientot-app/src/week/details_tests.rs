//! « Détails de l’activité et du crédit » of the history week demo, read and acted on through
//! the host as the window does, and of a week projected from a summary with debt.

use std::collections::HashMap;

use bientot_core::demo::DemoScenario;
use bientot_domain::accounting::AccountingLabel;
use bientot_domain::configuration::RecurringAppointment;
use bientot_domain::history::{
    CurrentWeek, DailySummaryBreakdown, DailySummaryBreakdownEntry, DailySummaryRevision,
    DailySummarySnapshotParts,
};
use bientot_domain::intervals::UtcInterval;
use bientot_domain::sources::SourceQuality;
use bientot_domain::summaries::IndicatorState;
use bientot_domain::time::FixedTimeZoneProvider;
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use super::*;
use crate::adjust::AdjustForm;
use crate::host::{Source, Today};
use crate::week::test_support::{Throwaway, date, nb};
use crate::week::{Answer, Availability, Offer, Recorded, SegmentKind, WeekCommand, WeekTimeline};

fn workday(date: Date) -> LocalWorkday {
    LocalWorkday::new(date, "UTC")
}

fn id(text: &str) -> AdjustmentId {
    AdjustmentId::new(text).unwrap()
}

/// The host of a demo, and its week with the details open.
struct Week {
    today: Today,
    state: WeekState,
    _directory: Throwaway,
}

impl Week {
    fn open(scenario: DemoScenario, language: Language) -> Self {
        let (host, _directory) = Throwaway::open(scenario);
        let today = Today::open(Source::Demo(Box::new(host)), language).unwrap();
        let mut state = WeekState::new(today.week().unwrap(), language);
        assert_eq!(state.details(), None, "the details start closed");
        assert!(state.toggle_details());
        Self {
            today,
            state,
            _directory,
        }
    }

    /// What the window does with an answer in the open bubble: record it, reload, report.
    fn choose(&mut self, answer: Answer) {
        let command = self.state.choose(answer).unwrap();
        let recorded = self.today.run_week(&command);
        self.state
            .reload(self.today.week().unwrap(), self.today.recalculated());
        self.state.finished(recorded);
    }

    fn details(&self, date: Date) -> Details {
        let mut state = self.state.clone();
        state.select(date);
        state.details().unwrap()
    }

    /// « Corriger le temps… » as `seed` opens it on the day the week named.
    fn form(&mut self, workday: LocalWorkday, seed: Seed) -> AdjustForm {
        let target = self.today.adjust_target(Some(workday)).unwrap();
        let now = self.today.adjustment_time();
        AdjustForm::new(target, seed, id("form"), now, Language::French)
    }
}

fn credit(details: &Details) -> Vec<(&str, String)> {
    details
        .credit
        .iter()
        .map(|line| (line.label, line.value.replace('\u{a0}', "~")))
        .collect()
}

fn rows(details: &Details) -> Vec<(&str, Vec<&str>, String, bool)> {
    details
        .corrections
        .iter()
        .map(|row| {
            (
                row.disposition,
                row.ranges.iter().map(String::as_str).collect(),
                row.duration.replace('\u{a0}', "~"),
                row.edit.enabled,
            )
        })
        .collect()
}

fn owned(lines: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    lines
        .iter()
        .map(|(label, value)| (*label, (*value).to_owned()))
        .collect()
}

#[test]
fn the_details_show_the_open_days_credit() {
    let week = Week::open(DemoScenario::HistoryWeek, Language::French);
    let monday = week.details(date(8, 31));
    assert_eq!(
        (
            monday.accessible_name.as_str(),
            monday.credit_title,
            monday.corrections_title
        ),
        (
            "Activité et crédit, Lundi 31\u{a0}août",
            "Crédit",
            "Corrections manuelles"
        )
    );
    assert_eq!(
        credit(&monday),
        owned(&[
            ("Gagné", "2~h~00~min"),
            ("Appliqué", "0~min"),
            ("Disponible", "2~h~00~min"),
            ("Expiré", "0~min"),
        ])
    );
    assert_eq!(
        credit(&week.details(date(9, 1))),
        owned(&[
            ("Gagné", "1~h~00~min"),
            ("Appliqué", "1~h~00~min"),
            ("Disponible", "2~h~00~min"),
            ("Expiré", "0~min"),
        ])
    );
}

#[test]
fn the_details_list_the_open_days_corrections() {
    let week = Week::open(DemoScenario::HistoryWeek, Language::French);
    // The half hour the demo credits has no interval, so the day's strip does not draw it.
    let monday = week.details(date(8, 31));
    assert_eq!(
        rows(&monday),
        [("Correction manuelle", vec![], "30~min".to_owned(), true)]
    );
    assert_eq!(
        monday.corrections[0].edit_name,
        nb("Modifier, Correction manuelle, 30~min")
    );
    assert_eq!(
        rows(&week.details(date(9, 1))),
        [(
            "Travail confirmé",
            vec!["15:15–16:00"],
            "45~min".to_owned(),
            true
        )]
    );
}

#[test]
fn the_details_offer_modifier_and_ajouter() {
    let week = Week::open(DemoScenario::HistoryWeek, Language::French);
    let monday = week.details(date(8, 31));
    assert_eq!(
        (monday.corrections[0].edit, monday.add),
        (
            DetailsAction {
                label: "Modifier",
                enabled: true
            },
            DetailsAction {
                label: "Ajouter une correction",
                enabled: true
            }
        )
    );
}

#[test]
fn a_day_without_corrections_lists_none() {
    let week = Week::open(DemoScenario::HistoryWeek, Language::French);
    let wednesday = week.details(date(9, 2));
    assert_eq!(wednesday.corrections, []);
    assert!(wednesday.add.enabled);
}

#[test]
fn a_day_without_a_summary_has_no_details() {
    let mut week = Week::open(DemoScenario::ProblemPeriods, Language::French);
    week.state.select(date(9, 1));
    assert!(!week.state.has_details());
    assert_eq!(week.state.details(), None);
    assert!(
        week.state.details_open(),
        "the panel stays open for the next day"
    );
    week.state.select(date(8, 31));
    assert!(week.state.has_details());
    assert!(week.state.details().is_some());
    assert!(!week.state.toggle_details());
    assert_eq!(week.state.details(), None);
}

/// A long-lock confirmation is listed but not edited, as in its bubble; the gap counted as work
/// after it is. Both are listed in the order of the day.
#[test]
fn corrections_are_listed_in_the_order_of_the_day() {
    let mut week = Week::open(DemoScenario::HistoryWeek, Language::French);
    week.state.settle_next();
    week.choose(Answer::Count);
    week.state.settle_next();
    assert_eq!(week.state.bubble().unwrap().selection.date, date(9, 2));
    week.choose(Answer::Count);
    let wednesday = week.details(date(9, 2));
    assert_eq!(
        rows(&wednesday),
        [
            (
                "Travail confirmé",
                vec!["10:00–12:00"],
                "2~h~00~min".to_owned(),
                false
            ),
            (
                "Travail confirmé",
                vec!["14:00–14:15"],
                "15~min".to_owned(),
                true
            ),
        ]
    );
    let [confirmation, gap] = &wednesday.corrections[..] else {
        panic!("Wednesday lists its confirmation and its gap");
    };
    week.state.select(date(9, 2));
    assert_eq!(
        (confirmation.note, gap.note),
        (
            Some(
                "Correction confirmée lors d’une vérification de verrouillage. Elle ne peut plus être modifiée."
            ),
            None
        )
    );
    assert_eq!(
        confirmation.edit_name,
        nb("Modifier, Travail confirmé, 10:00–12:00, 2~h~00~min, \
            Correction confirmée lors d’une vérification de verrouillage. Elle ne peut plus être modifiée.")
    );
    assert_eq!(week.state.edit_listed(&confirmation.id), None);
    let day = week.state.edit_listed(&gap.id).unwrap();
    assert_eq!(day, workday(date(9, 2)));
    assert!(week.today.editable(&day, &gap.id).is_ok());
}

#[test]
fn an_excluded_long_lock_can_be_edited() {
    let mut week = Week::open(DemoScenario::HistoryWeek, Language::French);
    week.state.settle_next();
    week.choose(Answer::Exclude);
    assert_eq!(
        rows(&week.details(date(9, 2))),
        [("Exclu", vec!["10:00–12:00"], "2~h~00~min".to_owned(), true)]
    );
}

/// An edit of a long-lock exclusion stays bound to its review, as the ledger keeps it: the week
/// offers « Modifier… » on the replacement, never « Retirer la correction », which the ledger
/// refuses.
#[test]
fn an_edited_long_lock_exclusion_is_not_removed() {
    let mut week = Week::open(DemoScenario::HistoryWeek, Language::French);
    week.state.settle_next();
    week.choose(Answer::Exclude);
    let wednesday = workday(date(9, 2));
    let excluded = week.details(date(9, 2)).corrections[0].id.clone();
    let editable = week.today.editable(&wednesday, &excluded).unwrap();
    let mut form = week.form(wednesday.clone(), Seed::Edit(editable));
    form.reason = "Déjeuner".into();
    let replacement = form.submit().unwrap();
    week.today.save_adjustment(&replacement).unwrap();
    week.state
        .reload(week.today.week().unwrap(), week.today.recalculated());
    let details = week.details(date(9, 2));
    assert_eq!(
        rows(&details),
        [("Exclu", vec!["10:00–12:00"], "2~h~00~min".to_owned(), true)]
    );
    assert_eq!(
        (&details.corrections[0].id, details.corrections[0].note),
        (
            replacement.id(),
            Some(
                "Correction issue d’une vérification de verrouillage. Elle ne peut pas être retirée."
            )
        )
    );
    let segment = week
        .state
        .timeline
        .day(date(9, 2))
        .unwrap()
        .segments
        .iter()
        .find(|segment| segment.adjustment_id.as_ref() == Some(replacement.id()))
        .map(|segment| segment.id(date(9, 2)))
        .unwrap();
    week.state.open(segment);
    assert_eq!(
        week.state
            .bubble()
            .unwrap()
            .actions
            .iter()
            .map(|action| (action.label.as_str(), action.enabled))
            .collect::<Vec<_>>(),
        [
            ("Compter comme travail", false),
            ("Retirer la correction", false)
        ]
    );
    assert_eq!(week.state.choose(Answer::Remove), None);
    assert_eq!(
        week.today
            .run_week(&WeekCommand::Revoke {
                workday: wednesday,
                adjustments: vec![replacement.id().clone()],
            })
            .map(drop),
        Err(Key::HistoryCorrectionNotRemovable),
        "the ledger refuses to withdraw it"
    );
}

/// « Modifier » opens « Corriger le temps… » on the correction of the open day, « Ajouter une
/// correction » on that day blank.
#[test]
fn modifier_and_ajouter_open_the_form_on_the_open_day() {
    let mut week = Week::open(DemoScenario::HistoryWeek, Language::French);
    week.state.select(date(9, 1));
    let meeting = id("demo-history-week-meeting");
    assert_eq!(
        week.state.edit_listed(&id("demo-history-week-manual")),
        None,
        "Monday's correction is not on the open day"
    );
    let day = week.state.edit_listed(&meeting).unwrap();
    assert_eq!(day, workday(date(9, 1)));
    let editable = week.today.editable(&day, &meeting).unwrap();
    let form = week.form(day, Seed::Edit(editable));
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("15:15".into(), "16:00".into())
    );

    let (day, seed) = week.state.add_listed().unwrap();
    assert_eq!((day, seed), (workday(date(9, 1)), Seed::Blank));
}

#[test]
fn nothing_is_edited_or_added_while_a_refresh_is_pending() {
    let mut week = Week::open(DemoScenario::HistoryWeek, Language::French);
    week.state.settle_next();
    week.state.choose(Answer::Count).unwrap();
    week.state.finished(Ok(Recorded {
        saved: Vec::new(),
        recalculated: false,
        proposal: None,
    }));
    week.state.select(date(9, 1));
    let tuesday = week.state.details().unwrap();
    assert_eq!(
        rows(&tuesday),
        [(
            "Travail confirmé",
            vec!["15:15–16:00"],
            "45~min".to_owned(),
            false
        )]
    );
    assert_eq!(
        tuesday.corrections[0].note, None,
        "a correction that can be edited and removed carries no note"
    );
    assert!(!tuesday.add.enabled);
    assert_eq!(
        week.state.edit_listed(&id("demo-history-week-meeting")),
        None
    );
    assert_eq!(week.state.add_listed(), None);
}

fn at(hour: i8, minute: i8) -> Timestamp {
    date(9, 3)
        .at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn entry(
    from: (i8, i8),
    to: (i8, i8),
    label: AccountingLabel,
    correction: Option<&str>,
) -> DailySummaryBreakdownEntry {
    let interval = UtcInterval::new(at(from.0, from.1), at(to.0, to.1)).unwrap();
    let disposition = if correction.is_some() {
        DailySummaryDisposition::ManualCredit
    } else {
        DailySummaryDisposition::ConfirmedCredit
    };
    DailySummaryBreakdownEntry::new(
        disposition,
        label,
        interval.duration(),
        Some(interval),
        correction.map(id),
        None,
    )
    .unwrap()
}

/// A Thursday that met its 7 h with a correction in three pieces, while the week still owes
/// 1 h 30 min: 30 min of credit available against 2 h of debt.
fn owing_thursday(language: Language) -> WeekState {
    owing_thursday_offering(
        language,
        &HashMap::from([(id("manual"), Offer::EditAndRemove)]),
    )
}

/// [`owing_thursday`], with `offers` on its correction.
fn owing_thursday_offering(language: Language, offers: &HashMap<AdjustmentId, Offer>) -> WeekState {
    let manual = Some("manual");
    let snapshot = DailySummarySnapshot::new(DailySummarySnapshotParts {
        workday: workday(date(9, 3)),
        computed_at: at(18, 0),
        input_fingerprint: "owing".to_owned(),
        credited_work: SignedDuration::from_hours(7),
        excluded_work: SignedDuration::ZERO,
        unknown_work: SignedDuration::ZERO,
        nominal_target: SignedDuration::from_hours(7),
        effective_target: SignedDuration::from_hours(7),
        carry_available: SignedDuration::from_mins(30),
        carry_applied: SignedDuration::ZERO,
        carry_earned: SignedDuration::ZERO,
        carry_expired: SignedDuration::ZERO,
        carry_debt: SignedDuration::from_hours(2),
        debt_pending: false,
        projected_departure: None,
        indicator_state: IndicatorState::Blue,
        source_quality: SourceQuality::Complete,
        has_pending_review: false,
        breakdown: DailySummaryBreakdown::new([
            entry((9, 0), (10, 0), AccountingLabel::Manual, manual),
            entry((10, 0), (11, 0), AccountingLabel::Manual, manual),
            entry((11, 0), (14, 0), AccountingLabel::ActiveWork, None),
            entry((14, 0), (15, 0), AccountingLabel::Manual, manual),
            entry((15, 0), (16, 0), AccountingLabel::ActiveWork, None),
        ]),
    })
    .unwrap();
    assert_eq!(
        Credit::of(&snapshot),
        Credit {
            earned: SignedDuration::ZERO,
            applied: SignedDuration::ZERO,
            balance: SignedDuration::from_mins(-90),
            expired: SignedDuration::ZERO,
        }
    );
    let week = CurrentWeek::new(
        date(9, 3),
        [DailySummaryRevision::new(1, snapshot).unwrap()],
    )
    .unwrap();
    let zones = FixedTimeZoneProvider::new("UTC", TimeZone::UTC);
    let timeline = WeekTimeline::project(&week, &[], offers, &[], &[], at(18, 0), &zones);
    let mut state = WeekState::new(timeline, language);
    state.select(date(9, 3));
    state.toggle_details();
    state
}

/// A negative balance is debt, in the tone of a day short of its target.
#[test]
fn a_negative_balance_reads_as_debt() {
    let details = owing_thursday(Language::English).details().unwrap();
    assert_eq!(
        credit(&details),
        owned(&[
            ("Earned", "0m"),
            ("Applied", "0m"),
            ("Debt", "−1h~30m"),
            ("Expired", "0m"),
        ])
    );
    assert_eq!(
        details.credit.map(|line| line.tone),
        [
            DayTone::Neutral,
            DayTone::Neutral,
            DayTone::Short,
            DayTone::Neutral
        ]
    );
    assert_eq!(
        (
            details.accessible_name.as_str(),
            details.credit_title,
            details.corrections_title
        ),
        (
            "Activity and credit, Thursday, September\u{a0}3",
            "Credit",
            "Manual corrections"
        )
    );
}

/// The pieces of one correction make one row: touching pieces merge into one range, the
/// others stay apart.
#[test]
fn a_correction_in_pieces_is_one_row() {
    let details = owing_thursday(Language::French).details().unwrap();
    assert_eq!(
        rows(&details),
        [(
            "Correction manuelle",
            vec!["09:00–11:00", "14:00–15:00"],
            "3~h~00~min".to_owned(),
            true
        )]
    );
}

/// A correction whose day's history the week could not read offers neither « Modifier… » nor
/// « Retirer la correction », and the details do not call it a long-lock confirmation.
#[test]
fn a_correction_the_week_cannot_tell_about_offers_nothing_and_says_nothing() {
    let mut state = owing_thursday_offering(Language::English, &HashMap::new());
    let details = state.details().unwrap();
    assert_eq!(
        details
            .corrections
            .iter()
            .map(|row| (row.edit.enabled, row.note))
            .collect::<Vec<_>>(),
        [(false, None)]
    );
    assert_eq!(state.edit_listed(&id("manual")), None);
    let segment = state
        .timeline
        .day(date(9, 3))
        .unwrap()
        .segments
        .iter()
        .find(|segment| segment.adjustment_id.is_some())
        .map(|segment| segment.id(date(9, 3)))
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
}

/// A Thursday of four hours whose summary holds `breakdown`, with a lock from 09:00 to 11:00
/// confirmed as work by « confirm », and the days before it without a summary.
fn confirmed_thursday(
    breakdown: impl IntoIterator<Item = DailySummaryBreakdownEntry>,
    appointments: &[RecurringAppointment],
) -> WeekTimeline {
    let snapshot = DailySummarySnapshot::new(DailySummarySnapshotParts {
        workday: workday(date(9, 3)),
        computed_at: at(18, 0),
        input_fingerprint: "asleep".to_owned(),
        credited_work: SignedDuration::from_hours(4),
        excluded_work: SignedDuration::ZERO,
        unknown_work: SignedDuration::ZERO,
        nominal_target: SignedDuration::from_hours(4),
        effective_target: SignedDuration::from_hours(4),
        carry_available: SignedDuration::ZERO,
        carry_applied: SignedDuration::ZERO,
        carry_earned: SignedDuration::ZERO,
        carry_expired: SignedDuration::ZERO,
        carry_debt: SignedDuration::ZERO,
        debt_pending: false,
        projected_departure: None,
        indicator_state: IndicatorState::Blue,
        source_quality: SourceQuality::Complete,
        has_pending_review: false,
        breakdown: DailySummaryBreakdown::new(breakdown),
    })
    .unwrap();
    let week = CurrentWeek::new(
        date(9, 3),
        [DailySummaryRevision::new(1, snapshot).unwrap()],
    )
    .unwrap();
    let zones = FixedTimeZoneProvider::new("UTC", TimeZone::UTC);
    let lock = UtcInterval::new(at(9, 0), at(11, 0)).unwrap();
    WeekTimeline::project(
        &week,
        &[],
        &HashMap::from([(id("confirm"), Offer::Locked)]),
        &[(id("confirm"), lock)],
        appointments,
        at(18, 0),
        &zones,
    )
}

/// Sleep within a lock confirmed as work is a piece of that answer, with nothing to settle;
/// sleep that goes on after it is cut at its end, and the rest stays to settle.
#[test]
fn sleep_within_a_confirmed_lock_is_settled_and_the_rest_is_not() {
    let confirmation = Some("confirm");
    let timeline = confirmed_thursday(
        [
            entry((8, 0), (9, 0), AccountingLabel::ActiveWork, None),
            entry((9, 0), (10, 0), AccountingLabel::Manual, confirmation),
            entry((10, 0), (10, 30), AccountingLabel::Suspended, None),
            entry((10, 30), (10, 45), AccountingLabel::Manual, confirmation),
            // Asleep across the end of the lock.
            entry((10, 45), (11, 30), AccountingLabel::Suspended, None),
            entry((11, 30), (12, 0), AccountingLabel::ActiveWork, None),
        ],
        &[],
    );

    let asleep: Vec<_> = timeline
        .day(date(9, 3))
        .unwrap()
        .segments
        .iter()
        .filter(|segment| segment.kind == SegmentKind::Sleep)
        .map(|segment| {
            (
                segment.range_text(Language::French),
                segment.adjustment_id.clone(),
                segment.needs_action,
            )
        })
        .collect();
    assert_eq!(
        asleep,
        [
            ("10:00–10:30".to_owned(), Some(id("confirm")), false),
            ("10:45–11:00".to_owned(), Some(id("confirm")), false),
            ("11:00–11:30".to_owned(), None, true),
        ]
    );
}

/// A lock that slept all along leaves no piece of its confirmation: its sleep still takes no
/// answer, and says why.
#[test]
fn a_lock_asleep_all_along_takes_no_answer() {
    let timeline = confirmed_thursday(
        [
            entry((8, 0), (9, 0), AccountingLabel::ActiveWork, None),
            entry((9, 0), (11, 0), AccountingLabel::Suspended, None),
            entry((11, 0), (12, 0), AccountingLabel::ActiveWork, None),
        ],
        &[],
    );
    let asleep = timeline.day(date(9, 3)).unwrap().segments[1].id(date(9, 3));
    let mut state = WeekState::new(timeline, Language::French);
    state.open(asleep);

    let bubble = state.bubble().unwrap();
    assert!(bubble.actions.iter().all(|action| !action.enabled));
    assert!(bubble.fixed);
    assert_eq!(
        bubble.notes,
        [Language::French.text(Key::HistoryConfirmationLocked)]
    );
}

/// A day without a summary shows no time, so it gets no bracket: only Thursday has one.
#[test]
fn a_day_without_a_summary_gets_no_appointment_marker() {
    let appointment = |id: &str, weekday| RecurringAppointment {
        id: id.to_owned(),
        name: String::new(),
        weekday,
        start: jiff::civil::time(9, 30, 0, 0),
        end: jiff::civil::time(10, 30, 0, 0),
        enabled: true,
        since: None,
        alternate_weeks_from: None,
        automatic: false,
    };
    let timeline = confirmed_thursday(
        [entry((8, 0), (12, 0), AccountingLabel::ActiveWork, None)],
        &[
            appointment("wednesday", jiff::civil::Weekday::Wednesday),
            appointment("thursday", jiff::civil::Weekday::Thursday),
        ],
    );

    let marked: Vec<_> = timeline
        .days
        .iter()
        .filter(|day| !day.appointments.is_empty())
        .map(|day| (day.date, day.availability))
        .collect();
    assert_eq!(marked, [(date(9, 3), Availability::Loaded)]);
    assert_eq!(
        timeline.day(date(9, 2)).unwrap().availability,
        Availability::NoSummary
    );
}
