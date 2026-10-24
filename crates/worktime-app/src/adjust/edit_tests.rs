//! « Modifier… » of the week and the tray: the form opened on a correction, and its replacement
//! saved on a demo ledger as the window saves it.

use jiff::civil::date;
use worktime_domain::adjustments::AdjustmentKind;

use super::testing::*;
use super::*;
use crate::week::{Severity, WeekState};

fn thursday() -> LocalWorkday {
    LocalWorkday::new(date(2026, 10, 8), "Test/Zone")
}

/// A correction of Thursday 8 October 2026 recorded at 14:00 with the reason « Réunion ».
fn recorded(kind: AdjustmentKind) -> ManualAdjustment {
    ManualAdjustment::new(
        id("original"),
        thursday(),
        utc("2026-10-08T14:00:00Z"),
        None,
        kind,
    )
    .unwrap()
    .with_reason("Réunion")
}

fn editing(kind: AdjustmentKind, zone: TimeZone, now: &str) -> AdjustForm {
    let seed = Seed::Edit(Editable::new(recorded(kind)).unwrap());
    seeded_at(date(2026, 10, 8), zone, seed, now, Language::French)
}

fn selected_intents(page: &AdjustPage) -> Vec<&'static str> {
    page.intents
        .iter()
        .filter(|option| option.selected)
        .map(|option| option.label)
        .collect()
}

/// `Prefill` of the .NET dialog: the correction's type, classification, times or duration and
/// reason, under the title of a new correction. Paris is two hours ahead in October; the
/// fields show the minutes, as the .NET time picker.
#[test]
fn an_edit_opens_on_each_kind_of_correction() {
    let span = interval("2026-10-08T07:15:30Z", "2026-10-08T08:00:00Z");
    let cases = [
        (
            AdjustmentKind::IntervalWork { interval: span },
            "Ajouter une plage travaillée",
            ManualClassification::Credited,
        ),
        (
            AdjustmentKind::Exclusion { interval: span },
            "Exclure une plage",
            ManualClassification::Credited,
        ),
        (
            AdjustmentKind::Reclassification {
                interval: span,
                classification: ManualClassification::Excluded,
                confirms_review_id: None,
            },
            "Reclasser une plage",
            ManualClassification::Excluded,
        ),
        (
            AdjustmentKind::Reclassification {
                interval: span,
                classification: ManualClassification::Credited,
                confirms_review_id: None,
            },
            "Reclasser une plage",
            ManualClassification::Credited,
        ),
    ];
    for (kind, intent, classification) in cases {
        let form = editing(
            kind.clone(),
            TimeZone::get("Europe/Paris").unwrap(),
            "2026-10-08T15:00:00Z",
        );
        let page = form.page();
        assert_eq!(page.title, "Corriger le temps de travail", "{kind:?}");
        assert_eq!(selected_intents(&page), [intent], "{kind:?}");
        assert_eq!(
            page.local_times.as_deref(),
            Some("Heures locales · Test/Zone"),
            "{kind:?}"
        );
        assert_eq!(form.classification(), classification, "{kind:?}");
        assert_eq!(
            (form.start.text(), form.end.text(), form.reason.as_str()),
            ("09:15".into(), "10:00".into(), "Réunion"),
            "{kind:?}"
        );
    }

    let form = editing(
        AdjustmentKind::DurationWork {
            duration: SignedDuration::from_mins(95),
        },
        TimeZone::UTC,
        "2026-10-08T15:00:00Z",
    );
    let page = form.page();
    assert_eq!(selected_intents(&page), ["Ajouter une durée"]);
    assert_eq!(page.local_times, None);
    assert_eq!(
        (form.duration.as_str(), form.reason.as_str()),
        ("1:35", "Réunion")
    );

    let revocation = ManualAdjustment::revocation(
        id("revocation"),
        thursday(),
        id("original"),
        utc("2026-10-08T14:00:00Z"),
    )
    .unwrap();
    assert_eq!(
        Editable::new(revocation),
        None,
        "a revocation is not edited"
    );
    let confirmation = ManualAdjustment::long_lock_confirmation(
        id("confirmation"),
        thursday(),
        span,
        utc("2026-10-08T14:00:00Z"),
        worktime_domain::accounting::LongLockReviewId::new("session-lock:42").unwrap(),
    )
    .unwrap();
    assert_eq!(
        Editable::new(confirmation),
        None,
        "a long-lock confirmation is not edited"
    );
}

/// A correction of the whole day opens on 00:00–00:00 and, saved untouched, keeps the whole day.
#[test]
fn a_whole_day_correction_saved_untouched_keeps_the_whole_day() {
    let day = AdjustmentKind::Exclusion {
        interval: interval("2026-10-08T00:00:00Z", "2026-10-09T00:00:00Z"),
    };
    let mut form = editing(day.clone(), TimeZone::UTC, "2026-10-09T10:00:00Z");
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("00:00".into(), "00:00".into())
    );
    assert_eq!(form.submit().unwrap().kind(), &day);
}

/// A correction from 00:00 moved to end on 00:00 is empty, not the whole day.
#[test]
fn an_edit_from_midnight_moved_to_end_on_midnight_is_refused() {
    let mut form = editing(
        AdjustmentKind::Exclusion {
            interval: interval("2026-10-08T00:00:00Z", "2026-10-08T00:30:00Z"),
        },
        TimeZone::UTC,
        "2026-10-09T10:00:00Z",
    );
    form.end = TimeField::new(0, 0);
    assert_eq!(form.submit(), None);
    assert_eq!(
        form.page().interval_error,
        Some("L’heure de début doit précéder l’heure de fin.")
    );
}

/// A duration opens as « 1:05 », which saved untouched records the same hour and five minutes.
#[test]
fn an_edited_duration_saved_untouched_keeps_its_minutes() {
    let worked = AdjustmentKind::DurationWork {
        duration: SignedDuration::from_mins(65),
    };
    let mut form = editing(worked.clone(), TimeZone::UTC, "2026-10-08T15:00:00Z");
    assert_eq!(form.duration, "1:05");
    assert_eq!(form.submit().unwrap().kind(), &worked);
}

/// As `TryEndpoint` of the .NET dialog, a time left as it opened keeps its seconds; and as
/// `AdjustTimeDialogLauncher`, the replacement is created after the correction it replaces,
/// whatever the clock says.
#[test]
fn an_edit_saved_replaces_the_correction_and_keeps_the_times_left_alone() {
    let span = interval("2026-10-08T09:15:30Z", "2026-10-08T10:00:45Z");
    let mut form = editing(
        AdjustmentKind::Exclusion { interval: span },
        TimeZone::UTC,
        "2026-10-08T13:00:00Z",
    );
    form.end = TimeField::new(10, 30);
    let replacement = form.submit().unwrap();
    assert_eq!(
        replacement.kind(),
        &AdjustmentKind::Exclusion {
            interval: interval("2026-10-08T09:15:30Z", "2026-10-08T10:30:00Z")
        }
    );
    assert_eq!(
        (replacement.id(), replacement.supersedes_id()),
        (&id("correction-1"), Some(&id("original")))
    );
    assert_eq!(
        replacement.created_at(),
        utc("2026-10-08T14:00:00.0000001Z")
    );
    assert_eq!(replacement.reason(), Some("Réunion"));
    let closed = form.finished(Ok(true)).unwrap();
    assert_eq!(
        (closed.message, closed.undoable),
        (nb("09:15:30–10:30 marqué comme non travaillé."), false)
    );

    let later = editing(
        AdjustmentKind::Exclusion { interval: span },
        TimeZone::UTC,
        "2026-10-08T15:00:00Z",
    )
    .submit()
    .unwrap();
    assert_eq!(later.created_at(), utc("2026-10-08T15:00:00Z"));
    assert_eq!(later.interval(), Some(span), "saved as it opened");
}

/// An edit saved as it opened reports the times it kept, to the second, and one that closes
/// the day reports 24:00.
#[test]
fn an_edit_reports_the_interval_it_saved() {
    let cases = [
        (
            interval("2026-10-08T09:40:12Z", "2026-10-08T09:45:42Z"),
            Language::English,
            "09:40:12–09:45:42 counted as work (5m~30s).",
        ),
        (
            interval("2026-10-08T09:40:12Z", "2026-10-08T09:45:42Z"),
            Language::French,
            "09:40:12–09:45:42 compté comme travail (5~min~30~s).",
        ),
        (
            interval("2026-10-08T22:00:00Z", "2026-10-09T00:00:00Z"),
            Language::English,
            "22:00–24:00 counted as work (2h~00m).",
        ),
        (
            interval("2026-10-08T22:00:00Z", "2026-10-09T00:00:00Z"),
            Language::French,
            "22:00–24:00 compté comme travail (2~h~00~min).",
        ),
    ];
    for (span, language, message) in cases {
        let seed = Seed::Edit(
            Editable::new(recorded(AdjustmentKind::IntervalWork { interval: span })).unwrap(),
        );
        let mut form = seeded_at(
            date(2026, 10, 8),
            TimeZone::UTC,
            seed,
            "2026-10-09T13:00:00Z",
            language,
        );
        assert_eq!(form.submit().unwrap().interval(), Some(span));
        assert_eq!(form.finished(Ok(true)).unwrap().message, nb(message));
    }
}

/// « Modifier… » of the week on the meeting the demo declared on Tuesday: the replacement is
/// drawn instead of it, and the week offers no « Annuler », which would withdraw the meeting
/// altogether. The meeting, replaced, can no longer be opened.
#[test]
fn an_edit_from_the_week_replaces_the_correction_there() {
    let (mut demo, mut state) = Demo::open();
    let tuesday = date(2026, 9, 1);
    let meeting = id("demo-history-week-meeting");
    let drawn = |state: &WeekState, adjustment: &AdjustmentId| {
        state
            .timeline
            .day(tuesday)
            .unwrap()
            .segments
            .iter()
            .find(|segment| segment.adjustment_id.as_ref() == Some(adjustment))
            .map(|segment| {
                (
                    segment.id(tuesday),
                    segment.local_start.time().to_string(),
                    segment.local_end.time().to_string(),
                )
            })
    };
    // « Modifier » of the meeting in the details.
    let (workday, edited) = (LocalWorkday::new(tuesday, "UTC"), meeting.clone());
    assert!(drawn(&state, &meeting).is_some());

    let active = demo.host.active_adjustment(&workday, &edited).unwrap();
    let mut form = demo.seeded(
        tuesday,
        Seed::Edit(Editable::new(active.unwrap()).unwrap()),
        "longer",
    );
    assert_eq!(
        (form.start.text(), form.end.text(), form.reason.is_empty()),
        ("15:15".into(), "16:00".into(), false)
    );
    form.end = TimeField::new(16, 30);
    let closed = demo.save(&mut form).unwrap().unwrap();
    state.reload(demo.host.week().unwrap(), true);
    state.adjusted(closed);
    assert_eq!(drawn(&state, &meeting), None);
    assert_eq!(
        drawn(&state, &id("longer")).map(|(_, start, end)| (start, end)),
        Some(("15:15:00".into(), "16:30:00".into()))
    );
    let bar = state.info_bar().unwrap();
    assert_eq!(
        (bar.message, bar.can_undo),
        (nb("15:15–16:30 compté comme travail (1~h~15~min)."), false)
    );
    assert_eq!(demo.host.active_adjustment(&workday, &meeting), Ok(None));

    state.edit_refused(Key::HistoryAdjustmentStale);
    let bar = state.info_bar().unwrap();
    assert_eq!(
        (bar.message.as_str(), bar.severity),
        (
            "Cette correction a changé. Actualisez la semaine avant de la modifier.",
            Severity::Warning
        )
    );
}

/// The half hour the demo credited on Monday, edited to 45 minutes, adds a quarter of an hour
/// to the day: the replacement counts, the half hour no longer does.
#[test]
fn an_edited_duration_counts_only_the_replacement() {
    let (mut demo, _) = Demo::open();
    let monday = date(2026, 8, 31);
    let credited = |demo: &Demo| {
        demo.host
            .week()
            .unwrap()
            .day(monday)
            .and_then(|day| day.figures.as_ref())
            .map(|figures| figures.credited)
            .unwrap()
    };
    let before = credited(&demo);
    let original = demo
        .host
        .active_adjustment(
            &LocalWorkday::new(monday, "UTC"),
            &id("demo-history-week-manual"),
        )
        .unwrap()
        .unwrap();
    let mut form = demo.seeded(
        monday,
        Seed::Edit(Editable::new(original).unwrap()),
        "longer",
    );
    assert_eq!(form.duration, "0:30");
    form.duration = "0:45".into();
    let closed = demo.save(&mut form).unwrap().unwrap();
    assert_eq!(closed.message, nb("Compté comme travail~:~45~min."));
    assert_eq!(credited(&demo) - before, SignedDuration::from_mins(15));
}
