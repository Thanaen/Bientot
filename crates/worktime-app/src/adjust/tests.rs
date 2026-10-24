//! The form against literal texts in both languages, and its corrections saved on a demo
//! ledger as the window saves them.

use jiff::civil::date;
use worktime_core::demo::DemoScenario;

use super::testing::*;
use super::*;
use crate::demo::{self, DemoHost};
use crate::week::Severity;

/// Thursday 8 October 2026 in UTC, as the mockup.
fn form(language: Language) -> AdjustForm {
    form_on(date(2026, 10, 8), TimeZone::UTC, language)
}

fn form_on(day: Date, zone: TimeZone, language: Language) -> AdjustForm {
    seeded(day, zone, Seed::Blank, language)
}

fn seeded(day: Date, zone: TimeZone, seed: Seed, language: Language) -> AdjustForm {
    seeded_at(day, zone, seed, "2026-10-08T15:00:00Z", language)
}

#[test]
fn a_new_correction_adds_nine_to_ten_and_reads_in_both_languages() {
    let page = form(Language::French).page();
    assert_eq!(page.title, "Corriger le temps de travail");
    assert_eq!(page.day, nb("Journée à corriger~: jeudi 8~octobre 2026"));
    assert_eq!(page.intent_header, "Type de correction");
    assert_eq!(
        page.intents
            .iter()
            .map(|option| (option.label, option.selected))
            .collect::<Vec<_>>(),
        [
            ("Ajouter une plage travaillée", true),
            ("Ajouter une durée", false),
            ("Exclure une plage", false),
            ("Reclasser une plage", false),
        ]
    );
    assert_eq!(
        page.local_times.as_deref(),
        Some("Heures locales · Test/Zone")
    );
    assert!(!page.shows_classification && !page.locked);
    assert_eq!(
        (page.interval_error, page.duration_error, page.callout),
        (None, None, None)
    );
    assert_eq!(
        (page.primary, page.primary_enabled, page.busy),
        ("Enregistrer", true, false)
    );
    assert_eq!((page.secondary, page.secondary_enabled), ("Annuler", true));
    let form = form(Language::English);
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("09:00".into(), "10:00".into())
    );
    let page = form.page();
    assert_eq!(page.title, "Adjust work time");
    assert_eq!(page.day, nb("Day to correct: Thursday, October~8, 2026"));
    assert_eq!(page.intent_header, "Correction type");
    assert_eq!((page.primary, page.secondary), ("Save", "Cancel"));
}

#[test]
fn an_edit_names_the_day_to_correct() {
    let edited = ManualAdjustment::new(
        id("original"),
        LocalWorkday::new(date(2026, 10, 8), "UTC"),
        utc("2026-10-08T14:00:00Z"),
        None,
        AdjustmentKind::IntervalWork {
            interval: interval("2026-10-08T07:00:00Z", "2026-10-08T08:00:00Z"),
        },
    )
    .unwrap();
    let form = seeded(
        date(2026, 10, 8),
        TimeZone::UTC,
        Seed::Edit(Editable::new(edited).unwrap()),
        Language::English,
    );
    assert!(matches!(form.opening, Opening::Edit { .. }));
    assert_eq!(
        form.page().day,
        nb("Day to correct: Thursday, October~8, 2026")
    );
}

#[test]
fn only_reclassify_shows_the_classification_and_only_a_duration_hides_the_times() {
    let mut form = form(Language::French);
    form.set_intent(Intent::AddDuration);
    let page = form.page();
    assert_eq!((page.local_times, page.shows_classification), (None, false));
    form.set_intent(Intent::Exclude);
    let page = form.page();
    assert_eq!(
        page.local_times.as_deref(),
        Some("Heures locales · Test/Zone")
    );
    assert!(!page.shows_classification);
    form.set_intent(Intent::Reclassify);
    let page = form.page();
    assert_eq!(
        page.local_times.as_deref(),
        Some("Heures locales · Test/Zone")
    );
    assert!(page.shows_classification);
    assert_eq!(
        classification_label(ManualClassification::Credited, Language::French),
        "Compter comme travail"
    );
    assert_eq!(
        classification_label(ManualClassification::Excluded, Language::English),
        "Exclude from work"
    );
}

#[test]
fn time_fields_take_two_digits_or_one_that_no_other_can_follow() {
    let mut field = TimeField::new(9, 0);
    assert!(!field.type_digit(TimePart::Hours, 1));
    assert_eq!(field.text(), "01:00");
    assert!(field.type_digit(TimePart::Hours, 4));
    assert_eq!(field.text(), "14:00");
    assert!(field.type_digit(TimePart::Hours, 7));
    assert_eq!(field.text(), "07:00");
    assert!(!field.type_digit(TimePart::Hours, 2));
    assert!(
        field.type_digit(TimePart::Hours, 5),
        "25 is no hour: 5 starts over, alone"
    );
    assert_eq!(field.text(), "05:00");
    assert!(!field.type_digit(TimePart::Minutes, 3));
    field.leave();
    assert!(!field.type_digit(TimePart::Minutes, 4));
    assert!(field.type_digit(TimePart::Minutes, 5));
    assert_eq!(field.text(), "05:45");
    field.step(TimePart::Minutes, 15);
    assert_eq!(field.text(), "05:00");
    field.step(TimePart::Hours, -6);
    assert_eq!(
        (field.part_text(TimePart::Hours), field.text()),
        ("23".into(), "23:00".into())
    );
}

#[test]
fn durations_are_hours_and_minutes_above_zero() {
    let minutes = SignedDuration::from_mins;
    assert_eq!(parse_duration("1:30"), Some(minutes(90)));
    assert_eq!(parse_duration(" 0 : 05 "), Some(minutes(5)));
    assert_eq!(parse_duration("23:59"), Some(minutes(23 * 60 + 59)));
    for refused in [
        "", "0:00", "1", "1:60", "24:00", "-1:00", "1:3a", "1:30:00", "+1:00",
    ] {
        assert_eq!(parse_duration(refused), None, "{refused}");
    }
}

#[test]
fn an_end_before_the_start_is_refused_under_the_fields_and_cleared_by_another_intent() {
    let mut form = form(Language::French);
    form.start = TimeField::new(10, 0);
    form.end = TimeField::new(9, 30);
    assert_eq!(form.submit(), None);
    let page = form.page();
    assert_eq!(
        page.interval_error,
        Some("L’heure de début doit précéder l’heure de fin.")
    );
    assert_eq!((page.callout, page.locked), (None, false));
    form.set_intent(Intent::Exclude);
    assert_eq!(form.page().interval_error, None);

    let mut form = self::form(Language::English);
    form.set_intent(Intent::AddDuration);
    form.duration = "0:00".into();
    assert_eq!(form.submit(), None);
    assert_eq!(
        form.page().duration_error,
        Some("Enter a positive duration as hours and minutes.")
    );
    form.end = TimeField::new(9, 0);
    form.set_intent(Intent::Reclassify);
    assert_eq!(form.submit(), None);
    assert_eq!(
        form.page().interval_error,
        Some("The start time must be before the end time.")
    );
}

#[test]
fn a_time_skipped_or_repeated_by_the_zone_is_refused() {
    // Daylight saving starts on Wednesday 11 March 2026 at 02:00 and ends on Wednesday
    // 4 November at 02:00 in this zone.
    let zone = TimeZone::posix("EST5EDT,M3.2.3,M11.1.3").unwrap();
    let mut form = form_on(date(2026, 3, 11), zone.clone(), Language::French);
    form.start = TimeField::new(2, 30);
    form.end = TimeField::new(4, 0);
    assert_eq!(form.submit(), None);
    let mut form = form_on(date(2026, 11, 4), zone.clone(), Language::French);
    form.start = TimeField::new(1, 30);
    assert_eq!(form.submit(), None);
    let mut form = form_on(date(2026, 3, 11), zone, Language::French);
    form.start = TimeField::new(1, 30);
    form.end = TimeField::new(3, 30);
    let adjustment = form.submit().unwrap();
    assert_eq!(
        adjustment.interval().map(|interval| interval.duration()),
        Some(SignedDuration::from_hours(1))
    );
}

#[test]
fn each_intent_records_its_correction_with_the_same_id_and_a_trimmed_reason() {
    let interval =
        UtcInterval::new(utc("2026-10-08T09:00:00Z"), utc("2026-10-08T10:00:00Z")).unwrap();
    let expected = [
        (
            Intent::AddInterval,
            AdjustmentKind::IntervalWork { interval },
        ),
        (
            Intent::AddDuration,
            AdjustmentKind::DurationWork {
                duration: SignedDuration::from_mins(90),
            },
        ),
        (Intent::Exclude, AdjustmentKind::Exclusion { interval }),
        (
            Intent::Reclassify,
            AdjustmentKind::Reclassification {
                interval,
                classification: ManualClassification::Excluded,
                confirms_review_id: None,
            },
        ),
    ];
    for (intent, kind) in expected {
        let mut form = form(Language::French);
        form.set_intent(intent);
        form.set_classification(ManualClassification::Excluded);
        form.duration = "1:30".into();
        form.reason = "  Réunion client ".into();
        let adjustment = form.submit().unwrap();
        assert_eq!(adjustment.kind(), &kind);
        assert_eq!(adjustment.id(), &id("correction-1"));
        assert_eq!(
            adjustment.workday(),
            &LocalWorkday::new(date(2026, 10, 8), "Test/Zone")
        );
        assert_eq!(adjustment.created_at(), utc("2026-10-08T15:00:00Z"));
        assert_eq!(adjustment.reason(), Some("Réunion client"));
        assert_eq!(adjustment.supersedes_id(), None);
    }
}

#[test]
fn saving_locks_the_form_and_a_full_success_closes_with_the_result() {
    let results = [
        (
            Intent::AddInterval,
            ManualClassification::Credited,
            "09:00–10:00 compté comme travail (1~h~00~min).",
        ),
        (
            Intent::AddDuration,
            ManualClassification::Credited,
            "1~h~30~min compté comme travail.",
        ),
        (
            Intent::Exclude,
            ManualClassification::Credited,
            "09:00–10:00 marqué comme non travaillé.",
        ),
        (
            Intent::Reclassify,
            ManualClassification::Credited,
            "09:00–10:00 compté comme travail (1~h~00~min).",
        ),
        (
            Intent::Reclassify,
            ManualClassification::Excluded,
            "09:00–10:00 retiré du travail.",
        ),
    ];
    for (intent, classification, message) in results {
        let mut form = form(Language::French);
        form.set_intent(intent);
        form.set_classification(classification);
        form.duration = "1:30".into();
        assert!(form.submit().is_some());
        let page = form.page();
        assert!(page.locked && page.busy && !page.primary_enabled && !page.secondary_enabled);
        assert!(!form.can_close());
        assert_eq!(form.submit(), None, "a correction is saved once at a time");
        form.set_intent(Intent::AddInterval);
        assert_eq!(form.intent(), intent, "the locked form takes no choice");
        assert_eq!(
            form.finished(Ok(true)),
            Some(Closed {
                message: nb(message),
                saved: id("correction-1"),
                refreshed: true,
                undoable: true,
            })
        );
    }
    assert_eq!(
        form(Language::English)
            .with_saved(Intent::AddDuration)
            .finished(Ok(true))
            .unwrap()
            .message,
        nb("1h~30m counted as work.")
    );
}

impl AdjustForm {
    fn with_saved(mut self, intent: Intent) -> Self {
        self.set_intent(intent);
        self.duration = "1:30".into();
        self.submit().unwrap();
        self
    }
}

#[test]
fn a_failure_shows_in_the_callout_and_only_a_conflict_or_an_ended_week_keeps_save_disabled() {
    let mut form = form(Language::French).with_saved(Intent::AddInterval);
    assert_eq!(
        form.finished(Err(Key::AdjustTimeErrorSubmissionFailed)),
        None
    );
    let page = form.page();
    assert_eq!(
        page.callout,
        Some(Callout {
            tone: CalloutTone::Error,
            message: "WorkTime n’a pas pu enregistrer cette correction. Vérifiez les valeurs puis réessayez.",
        })
    );
    assert!(!page.locked && page.primary_enabled && !page.busy);
    assert!(
        form.submit().is_some(),
        "a retry saves the same correction again"
    );
    assert_eq!(form.page().callout, None);

    assert_eq!(form.finished(Err(Key::AdjustTimeErrorConflict)), None);
    let page = form.page();
    assert_eq!(
        page.callout.map(|callout| callout.message),
        Some("Cette correction a changé. Fermez puis rouvrez-la avant de réessayer.")
    );
    assert!(!page.primary_enabled && page.secondary_enabled);
    assert_eq!(form.submit(), None);
    assert_eq!(
        form.close(),
        None,
        "closing a refused correction leaves nothing to show"
    );

    let mut form = self::form(Language::English).with_saved(Intent::Exclude);
    form.finished(Err(Key::AdjustTimeErrorActiveOverlap));
    assert_eq!(
        form.page().callout.map(|callout| callout.message),
        Some("This interval overlaps another active manual correction.")
    );
}

#[test]
fn a_day_of_an_ended_week_keeps_save_disabled_whatever_the_fields_say() {
    let mut form = form(Language::English).with_saved(Intent::AddInterval);
    assert_eq!(form.finished(Err(Key::AdjustTimeErrorOutsideWeek)), None);
    form.set_intent(Intent::AddDuration);
    form.duration = "0:45".into();
    let page = form.page();
    assert_eq!(
        page.callout.map(|callout| callout.message),
        Some("This day is in a week that has ended and can no longer be corrected.")
    );
    assert!(!page.locked && !page.primary_enabled && page.secondary_enabled);
    assert_eq!(form.submit(), None);
}

#[test]
fn a_save_without_refresh_stays_open_until_a_refresh_or_closing() {
    let mut form = form(Language::French).with_saved(Intent::AddInterval);
    assert_eq!(form.finished(Ok(false)), None);
    let page = form.page();
    assert_eq!(
        page.callout,
        Some(Callout {
            tone: CalloutTone::Warning,
            message: "La correction est enregistrée, mais les totaux n’ont pas pu être actualisés. Actualisez les totaux\u{202f}; la correction ne sera pas renvoyée.",
        })
    );
    assert!(page.locked && !page.busy && page.primary_enabled && page.secondary_enabled);
    assert_eq!(
        (page.primary, page.secondary),
        ("Actualiser les totaux", "Fermer")
    );
    assert_eq!(form.submit(), None, "the correction is not sent again");
    assert_eq!(form.refreshed(false), None);
    let closed = form.clone().refreshed(true).unwrap();
    assert!(closed.refreshed);
    assert_eq!(
        form.close(),
        Some(Closed {
            message: nb("09:00–10:00 compté comme travail (1~h~00~min)."),
            saved: id("correction-1"),
            refreshed: false,
            undoable: true,
        })
    );
    let mut form = self::form(Language::English).with_saved(Intent::AddInterval);
    form.finished(Ok(false));
    let page = form.page();
    assert_eq!(
        (page.primary, page.secondary),
        ("Refresh the totals", "Close")
    );
}

#[test]
fn a_correction_saved_from_the_week_shows_there_with_undo() {
    let (mut demo, mut state) = Demo::open();
    let monday = date(2026, 8, 31);
    state.select(monday);
    assert_eq!(
        state.adjust_target(),
        Some(LocalWorkday::new(monday, "UTC"))
    );
    let mut form = demo.form(monday, "evening");
    form.start = TimeField::new(19, 0);
    form.end = TimeField::new(20, 0);
    let closed = demo.save(&mut form).unwrap().unwrap();
    state.reload(demo.host.week().unwrap(), true);
    state.adjusted(closed);
    let bar = state.info_bar().unwrap();
    assert_eq!(
        (bar.message.as_str(), bar.severity, bar.can_undo),
        (
            nb("19:00–20:00 compté comme travail (1~h~00~min).").as_str(),
            Severity::Success,
            true
        )
    );
    assert!(matches!(
        state.undo(),
        Some(crate::week::WeekCommand::Revoke { adjustments, .. }) if adjustments == [id("evening")]
    ));
}

#[test]
fn the_ledger_refusals_name_the_window_messages() {
    let (mut demo, _) = Demo::open();
    let monday = date(2026, 8, 31);
    let mut first = demo.form(monday, "evening");
    first.start = TimeField::new(19, 0);
    first.end = TimeField::new(20, 0);
    demo.save(&mut first).unwrap();

    let mut overlapping = demo.form(monday, "overlapping");
    overlapping.set_intent(Intent::Exclude);
    overlapping.start = TimeField::new(19, 30);
    overlapping.end = TimeField::new(20, 30);
    assert_eq!(
        demo.save(&mut overlapping),
        Err(Key::AdjustTimeErrorActiveOverlap)
    );

    let mut reused = demo.form(monday, "evening");
    reused.start = TimeField::new(21, 0);
    reused.end = TimeField::new(22, 0);
    assert_eq!(demo.save(&mut reused), Err(Key::AdjustTimeErrorConflict));
}

#[test]
fn the_week_offers_a_correction_only_on_a_summarized_working_day() {
    let (_demo, mut state) = Demo::open();
    assert_eq!(state.focused(), date(2026, 9, 4));
    assert_eq!(
        state.adjust_target(),
        Some(LocalWorkday::new(date(2026, 9, 4), "UTC"))
    );
    state.adjusted(Closed {
        message: "Fait.".into(),
        saved: id("x"),
        refreshed: false,
        undoable: true,
    });
    assert_eq!(
        state.adjust_target(),
        None,
        "not while a refresh is pending"
    );
}

/// `DeclareWorkAsync` of the .NET app: under its own title, on the one interval the source left
/// unrecorded, which it shows to the second. Paris is two hours ahead in October; the fields
/// show the minute of each end.
#[test]
fn a_declaration_of_work_opens_on_the_minutes_of_its_interval() {
    let form = seeded(
        date(2026, 10, 8),
        TimeZone::get("Europe/Paris").unwrap(),
        on_slot(interval("2026-10-08T08:04:30Z", "2026-10-08T08:30:00.5Z")),
        Language::English,
    );
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("10:04".into(), "10:30".into())
    );
    let page = form.page();
    assert_eq!(page.title, "Record time worked");
    assert_eq!(page.day, nb("Day: Thursday, October~8, 2026"));
    let declaration = page.declaration.unwrap();
    assert_eq!(
        declaration.explanation,
        "Set Start and End to when you were working. Work already detected isn’t counted twice."
    );
    let context = declaration.context.unwrap();
    assert_eq!(
        context
            .around
            .iter()
            .map(|row| (row.times.as_str(), row.chosen))
            .collect::<Vec<_>>(),
        [("10:04:30–10:30:00", true)]
    );
    assert_eq!(page.local_times.as_deref(), Some("Local times · Test/Zone"));
    assert_eq!(
        seeded(
            date(2026, 10, 8),
            TimeZone::UTC,
            Seed::Blank,
            Language::French
        )
        .page()
        .declaration,
        None
    );
}

/// As `UpdateFieldVisibility` of the .NET dialog: a declaration offers no type of correction
/// nor classification, and whatever is asked of the form, it records Début–Fin counted as work.
#[test]
fn a_declaration_of_work_always_counts_its_interval_as_work() {
    let mut form = seeded(
        date(2026, 10, 8),
        TimeZone::UTC,
        Seed::DeclareWork(None),
        Language::French,
    );
    let page = form.page();
    assert_eq!(page.intents, []);
    assert_eq!(
        page.local_times.as_deref(),
        Some("Heures locales · Test/Zone")
    );
    assert!(!page.shows_classification);
    form.set_intent(Intent::Exclude);
    form.set_classification(ManualClassification::Excluded);
    assert_eq!(form.page(), page, "the choices are not the declaration's");
    assert_eq!(
        form.submit().unwrap().kind(),
        &AdjustmentKind::Reclassification {
            interval: interval("2026-10-08T09:00:00Z", "2026-10-08T10:00:00Z"),
            classification: ManualClassification::Credited,
            confirms_review_id: None,
        }
    );
}

/// Unrecorded time in several intervals opens no one of them, as the .NET app.
#[test]
fn a_declaration_of_work_without_one_interval_opens_on_nine_to_ten() {
    let form = seeded(
        date(2026, 10, 8),
        TimeZone::UTC,
        Seed::DeclareWork(None),
        Language::French,
    );
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("09:00".into(), "10:00".into())
    );
    let page = form.page();
    assert_eq!(page.title, "Renseigner un créneau travaillé");
    assert_eq!(page.day, nb("Journée~: jeudi 8~octobre 2026"));
    assert_eq!(
        page.local_times.as_deref(),
        Some("Heures locales · Test/Zone")
    );
    assert_eq!(
        page.declaration.map(|declaration| declaration.context),
        Some(None)
    );
}

/// A hint of yesterday's unrecorded time, followed after midnight, has nothing left on today:
/// the declaration opens on no interval rather than on the whole day.
#[test]
fn a_stale_suggestion_from_the_day_before_is_dropped() {
    let form = seeded_at(
        date(2026, 10, 8),
        TimeZone::UTC,
        on_slot(interval("2026-10-07T17:00:00Z", "2026-10-07T17:30:00Z")),
        "2026-10-08T00:10:00Z",
        Language::French,
    );
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("09:00".into(), "10:00".into())
    );
    assert_eq!(form.page().declaration.unwrap().context, None);
}

/// A slot that runs until the day's close opens with Fin on 00:00 and, saved as it opens,
/// declares the whole slot: nothing is left between 23:59 and midnight.
#[test]
fn a_slot_running_to_midnight_is_declared_whole() {
    let mut form = seeded(
        date(2026, 10, 7),
        TimeZone::UTC,
        on_slot(interval("2026-10-07T22:00:00Z", "2026-10-08T00:00:00Z")),
        Language::French,
    );
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("22:00".into(), "00:00".into())
    );
    assert_eq!(
        form.submit().unwrap().kind(),
        &AdjustmentKind::Reclassification {
            interval: interval("2026-10-07T22:00:00Z", "2026-10-08T00:00:00Z"),
            classification: ManualClassification::Credited,
            confirms_review_id: None,
        }
    );
}

/// Fin on 00:00 is the day's close, the only midnight after any Début.
#[test]
fn an_interval_typed_to_midnight_ends_at_the_close_of_the_day() {
    let mut form = form_on(date(2026, 10, 7), TimeZone::UTC, Language::French);
    form.start = TimeField::new(23, 0);
    form.end = TimeField::new(0, 0);
    assert_eq!(
        form.submit().unwrap().kind(),
        &AdjustmentKind::IntervalWork {
            interval: interval("2026-10-07T23:00:00Z", "2026-10-08T00:00:00Z"),
        }
    );
}

/// Typed Début and Fin both on 00:00 are an empty interval, not the whole day: even an
/// exclusion of a past day, which nothing else bounds, is refused.
#[test]
fn an_interval_typed_from_midnight_to_midnight_is_refused() {
    let mut form = form_on(date(2026, 10, 7), TimeZone::UTC, Language::French);
    form.set_intent(Intent::Exclude);
    form.start = TimeField::new(0, 0);
    form.end = TimeField::new(0, 0);
    assert_eq!(form.submit(), None);
    assert_eq!(
        form.page().interval_error,
        Some("L’heure de début doit précéder l’heure de fin.")
    );
}

/// Unrecorded time over a whole past day opens on 00:00–00:00 and, saved as it opens, declares
/// the whole day.
#[test]
fn a_whole_day_suggested_is_declared_whole() {
    let day = interval("2026-10-07T00:00:00Z", "2026-10-08T00:00:00Z");
    let mut form = seeded(
        date(2026, 10, 7),
        TimeZone::UTC,
        on_slot(day),
        Language::French,
    );
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("00:00".into(), "00:00".into())
    );
    assert_eq!(
        form.submit().unwrap().kind(),
        &AdjustmentKind::Reclassification {
            interval: day,
            classification: ManualClassification::Credited,
            confirms_review_id: None,
        }
    );
}

/// A suggestion running past a later midnight stops at the close of its own day.
#[test]
fn a_suggestion_ending_on_a_later_midnight_stops_at_the_close_of_its_day() {
    let mut form = seeded(
        date(2026, 10, 6),
        TimeZone::UTC,
        on_slot(interval("2026-10-06T22:00:00Z", "2026-10-08T00:00:00Z")),
        Language::French,
    );
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("22:00".into(), "00:00".into())
    );
    assert_eq!(
        form.submit().unwrap().interval(),
        Some(interval("2026-10-06T22:00:00Z", "2026-10-07T00:00:00Z"))
    );
}

/// A suggestion that ends on the target day's own midnight has nothing on that day.
#[test]
fn a_suggestion_ending_on_the_midnight_that_opens_the_day_is_dropped() {
    let form = seeded(
        date(2026, 10, 7),
        TimeZone::UTC,
        on_slot(interval("2026-10-06T22:00:00Z", "2026-10-07T00:00:00Z")),
        Language::French,
    );
    assert_eq!(form.page().declaration.unwrap().context, None);
}

/// An interval that starts the day before begins at the day's midnight.
#[test]
fn a_declaration_of_work_stays_on_its_day() {
    let form = seeded(
        date(2026, 10, 7),
        TimeZone::UTC,
        on_slot(interval("2026-10-06T23:59:30Z", "2026-10-07T23:59:30Z")),
        Language::French,
    );
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("00:00".into(), "23:59".into())
    );
}

/// Unrecorded time that runs until now ends on the current minute, not the next one, which the
/// accounting would refuse as past its horizon.
#[test]
fn a_declaration_of_work_ends_no_later_than_now() {
    let form = seeded_at(
        date(2026, 10, 8),
        TimeZone::UTC,
        on_slot(interval("2026-10-08T14:20:00Z", "2026-10-08T15:00:20Z")),
        "2026-10-08T15:00:40Z",
        Language::French,
    );
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("14:20".into(), "15:00".into())
    );
}

/// Unrecorded time still running past now stops on the current minute.
#[test]
fn a_declaration_of_work_past_now_stops_on_the_current_minute() {
    let mut form = seeded_at(
        date(2026, 10, 8),
        TimeZone::UTC,
        on_slot(interval("2026-10-08T14:20:00Z", "2026-10-08T18:00:00Z")),
        "2026-10-08T15:00:40Z",
        Language::French,
    );
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("14:20".into(), "15:00".into())
    );
    assert_eq!(
        form.submit().unwrap().interval(),
        Some(interval("2026-10-08T14:20:00Z", "2026-10-08T15:00:00Z"))
    );
}

/// A declaration spanning work already detected is saved and counts that work once, where the
/// same interval added as work is refused.
#[test]
fn a_declaration_of_work_over_detected_work_is_saved() {
    fn unrecorded(host: &mut DemoHost) -> Vec<UtcInterval> {
        match host.refresh(Language::French).unwrap().source_hint {
            Some(crate::workday::SourceHint::Unrecorded { intervals, .. }) => intervals,
            other => panic!("not unrecorded time: {other:?}"),
        }
    }
    let directory = demo::throwaway_directory();
    let mut host = DemoHost::open(DemoScenario::ProblemPeriods, &directory).unwrap();
    let intervals = unrecorded(&mut host);
    let mut demo = Demo { host, directory };
    let day = date(2026, 8, 31);

    let mut added = demo.form(day, "added");
    added.start = TimeField::new(9, 0);
    added.end = TimeField::new(11, 0);
    assert_eq!(
        demo.save(&mut added),
        Err(Key::AdjustTimeErrorOverlapCredited)
    );

    let mut declared = demo.seeded(day, on_slot(intervals[0]), "declared");
    assert_eq!(
        (declared.start.text(), declared.end.text()),
        ("09:40".into(), "09:45".into())
    );
    declared.start = TimeField::new(9, 0);
    declared.end = TimeField::new(11, 0);
    let closed = demo.save(&mut declared).unwrap().unwrap();
    assert_eq!(
        closed.message,
        nb("09:00–11:00 compté comme travail (2~h~00~min).")
    );
    assert_eq!(
        unrecorded(&mut demo.host),
        [interval("2026-08-31T14:00:00Z", "2026-08-31T14:20:00Z")]
    );
}

/// A declaration on an unrecorded interval, saved as it opens, records the whole of it as work: the hint then reports only the other one.
#[test]
fn a_declaration_of_work_saved_as_it_opens_records_its_interval() {
    let directory = demo::throwaway_directory();
    let mut host = DemoHost::open(DemoScenario::ProblemPeriods, &directory).unwrap();
    let Some(crate::workday::SourceHint::Unrecorded { intervals, .. }) =
        host.refresh(Language::French).unwrap().source_hint
    else {
        panic!("the problem periods leave time unrecorded");
    };
    let mut demo = Demo { host, directory };
    let mut declared = demo.seeded(date(2026, 8, 31), on_slot(intervals[0]), "declared");
    let closed = demo.save(&mut declared).unwrap().unwrap();
    assert_eq!(
        closed.message,
        nb("09:40:12–09:45:42 compté comme travail (5~min~30~s).")
    );
    let day = demo.host.refresh(Language::French).unwrap();
    assert!(
        matches!(
            day.source_hint,
            Some(crate::workday::SourceHint::Unrecorded { intervals: left, .. }) if left == [intervals[1]]
        ),
        "the hint reports only the other interval"
    );
}
