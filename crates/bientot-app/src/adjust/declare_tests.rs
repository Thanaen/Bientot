//! « Déclarer du temps travaillé… » of the week: the declaration they open, and what saving
//! it records on a demo ledger.

use bientot_domain::adjustments::AdjustmentKind;
use jiff::civil::date;

use super::testing::*;
use super::*;
use crate::week::{SegmentId, SegmentKind, WeekState};

fn wednesday() -> Date {
    date(2026, 9, 2)
}

/// The day's slots from 12:00 on: kind, local times, and whether they await an answer.
fn afternoon(state: &WeekState) -> Vec<(SegmentKind, String, String, bool)> {
    state
        .timeline
        .day(wednesday())
        .unwrap()
        .segments
        .iter()
        .skip(2)
        .map(|segment| {
            (
                segment.kind,
                segment.local_start.time().to_string(),
                segment.local_end.time().to_string(),
                segment.needs_action,
            )
        })
        .collect()
}

/// Wednesday's gap left to settle, the only one of the day.
fn gap(state: &WeekState) -> SegmentId {
    state
        .timeline
        .day(wednesday())
        .unwrap()
        .segments
        .iter()
        .find(|segment| segment.kind == SegmentKind::Gap && segment.needs_action)
        .unwrap()
        .id(wednesday())
}

fn credited(state: &WeekState) -> SignedDuration {
    state
        .timeline
        .day(wednesday())
        .and_then(|day| day.figures.as_ref())
        .unwrap()
        .credited
}

fn row(
    kind: SegmentKind,
    start: &str,
    end: &str,
    pending: bool,
) -> (SegmentKind, String, String, bool) {
    (kind, start.into(), end.into(), pending)
}

/// The quarter-hour gap of Wednesday, narrowed to its last ten minutes: only those count, once,
/// and the five minutes before stay to settle.
#[test]
fn part_of_a_gap_is_declared_and_counted_once() {
    let (mut demo, mut state) = Demo::open();
    let before = credited(&state);
    state.select(wednesday());
    let (workday, seed) = state.declare_work().unwrap();
    let Seed::DeclareWork(Some(unrecorded)) = &seed else {
        panic!("a declaration on the slot, not {seed:?}");
    };
    assert_eq!(
        (&workday, unrecorded.slots()),
        (
            &LocalWorkday::new(wednesday(), "UTC"),
            &[interval("2026-09-02T14:00:00Z", "2026-09-02T14:15:00Z")][..]
        )
    );
    let mut form = demo.seeded(wednesday(), seed, "part");
    let page = form.page();
    assert_eq!(page.title, "Déclarer du temps travaillé");
    let context = page.declaration.unwrap().context.unwrap();
    assert_eq!(
        context
            .around
            .iter()
            .map(|row| (row.label, row.times.as_str(), row.chosen))
            .collect::<Vec<_>>(),
        [
            ("Travail", "12:00–14:00", false),
            ("Trou", "14:00–14:15", true),
            ("Travail", "14:15–16:45", false),
        ]
    );
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("14:00".into(), "14:15".into())
    );
    form.start = TimeField::new(14, 5);
    let recorded = form.clone().submit().unwrap();
    assert_eq!(
        recorded.kind(),
        &AdjustmentKind::Reclassification {
            interval: interval("2026-09-02T14:05:00Z", "2026-09-02T14:15:00Z"),
            classification: ManualClassification::Credited,
            confirms_review_id: None,
        }
    );
    let closed = demo.save(&mut form).unwrap().unwrap();
    state.reload(demo.host.week().unwrap(), true);
    state.adjusted(closed);

    assert_eq!(
        afternoon(&state),
        [
            row(SegmentKind::Work, "12:00:00", "14:00:00", false),
            row(SegmentKind::Gap, "14:00:00", "14:05:00", true),
            row(SegmentKind::Manual, "14:05:00", "14:15:00", false),
            row(SegmentKind::Work, "14:15:00", "16:45:00", false),
        ]
    );
    assert_eq!(credited(&state) - before, SignedDuration::from_mins(10));
    let bar = state.info_bar().unwrap();
    assert_eq!(
        (bar.message, bar.can_undo),
        (nb("14:05–14:15 compté comme travail (10~min)."), true)
    );
}

/// « Déclarer du temps travaillé… » declares on the open day, not on today, on its gap to
/// settle; neither it nor the partial action is offered while a refresh is pending.
#[test]
fn work_is_declared_on_the_open_day_of_the_week() {
    let (_demo, mut state) = Demo::open();
    assert_eq!(state.focused(), date(2026, 9, 4));
    state.select(wednesday());
    let (workday, seed) = state.declare_work().unwrap();
    assert_eq!(workday, LocalWorkday::new(wednesday(), "UTC"));
    let Seed::DeclareWork(Some(unrecorded)) = &seed else {
        panic!("a declaration on the gap, not {seed:?}");
    };
    assert_eq!(
        unrecorded.slots(),
        [interval("2026-09-02T14:00:00Z", "2026-09-02T14:15:00Z")]
    );

    state.open(gap(&state));
    state.adjusted(Closed {
        message: String::new(),
        saved: id("elsewhere"),
        refreshed: false,
        undoable: true,
    });
    assert_eq!(state.declare_work(), None);
}

/// Wednesday's gap with declarations at 14:00:00–14:03:43 and 14:11:17–14:15:00: what is left
/// of it opens « Déclarer du temps travaillé » on 14:03–14:11.
fn declared_between_corrections() -> (Demo, AdjustForm) {
    let (mut demo, mut state) = Demo::open();
    for (name, start, end) in [
        ("before", "2026-09-02T14:00:00Z", "2026-09-02T14:03:43Z"),
        ("after", "2026-09-02T14:11:17Z", "2026-09-02T14:15:00Z"),
    ] {
        let declared = ManualAdjustment::reclassification(
            id(name),
            LocalWorkday::new(wednesday(), "UTC"),
            interval(start, end),
            ManualClassification::Credited,
            demo.host.correction_time(),
        )
        .unwrap();
        demo.host.save_adjustment(&declared).unwrap();
    }
    demo.host.refresh(Language::French).unwrap();
    state.reload(demo.host.week().unwrap(), true);
    assert_eq!(
        afternoon(&state)[1..4],
        [
            row(SegmentKind::Manual, "14:00:00", "14:03:43", false),
            row(SegmentKind::Gap, "14:03:43", "14:11:17", true),
            row(SegmentKind::Manual, "14:11:17", "14:15:00", false),
        ]
    );
    state.select(wednesday());
    let (_, seed) = state.declare_work().unwrap();
    let form = demo.seeded(wednesday(), seed, "part");
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("14:03".into(), "14:11".into())
    );
    let context = form.page().declaration.unwrap().context.unwrap();
    let chosen = context.around.iter().find(|row| row.chosen).unwrap();
    assert_eq!(chosen.times, "14:03:43–14:11:17");
    (demo, form)
}

fn declared_interval(form: &AdjustForm) -> UtcInterval {
    match form.clone().submit().unwrap().kind() {
        AdjustmentKind::Reclassification { interval, .. } => *interval,
        other => panic!("not a declaration: {other:?}"),
    }
}

/// Début and Fin left as they open keep the seconds of the slot they show to the minute.
#[test]
fn an_untouched_declaration_records_its_slot_exactly() {
    let (mut demo, mut form) = declared_between_corrections();
    assert_eq!(
        declared_interval(&form),
        interval("2026-09-02T14:03:43Z", "2026-09-02T14:11:17Z")
    );
    assert!(demo.save(&mut form).unwrap().is_some());
}

/// Fin narrowed, Début left on 14:03 still starts at 14:03:43, where the declaration before
/// ends: on 14:03:00 it would overlap it and be refused.
#[test]
fn a_declaration_next_to_a_correction_keeps_its_untouched_start() {
    let (mut demo, mut form) = declared_between_corrections();
    form.end = TimeField::new(14, 10);
    assert_eq!(
        declared_interval(&form),
        interval("2026-09-02T14:03:43Z", "2026-09-02T14:10:00Z")
    );
    assert!(demo.save(&mut form).unwrap().is_some());

    let mut minutes = demo.seeded(wednesday(), Seed::DeclareWork(None), "minutes");
    minutes.start = TimeField::new(14, 3);
    minutes.end = TimeField::new(14, 10);
    assert_eq!(
        demo.save(&mut minutes),
        Err(Key::AdjustTimeErrorActiveOverlap)
    );
}

/// Fin on 00:00 declares up to the close of the day, which the ledger takes and the week then
/// draws as the day's last slot.
#[test]
fn work_is_declared_up_to_midnight() {
    let (mut demo, mut state) = Demo::open();
    let mut form = demo.seeded(wednesday(), Seed::DeclareWork(None), "evening");
    form.start = TimeField::new(23, 0);
    form.end = TimeField::new(0, 0);
    assert!(demo.save(&mut form).unwrap().is_some());
    state.reload(demo.host.week().unwrap(), true);
    assert_eq!(
        afternoon(&state).last(),
        Some(&row(SegmentKind::Manual, "23:00:00", "00:00:00", false))
    );
}

/// What the window and its result write of a slot declared as it opens.
#[derive(Debug, PartialEq, Eq)]
struct Declared {
    /// The chosen slot's row, to the second.
    times: String,
    start: String,
    end: String,
    /// The result once saved.
    message: String,
}

fn declared_as_opened(slot: UtcInterval, language: Language) -> Declared {
    let mut form = seeded_at(
        date(2026, 10, 7),
        TimeZone::UTC,
        on_slot(slot),
        "2026-10-08T15:00:00Z",
        language,
    );
    let context = form.page().declaration.unwrap().context.unwrap();
    let times = context
        .around
        .into_iter()
        .find(|row| row.chosen)
        .unwrap()
        .times;
    let (start, end) = (form.start.text(), form.end.text());
    form.submit().unwrap();
    Declared {
        times,
        start,
        end,
        message: form.finished(Ok(true)).unwrap().message,
    }
}

/// A slot running to the day's close reads 24:00 wherever it is written, as the week draws it,
/// while Fin shows 00:00.
#[test]
fn a_slot_running_to_midnight_ends_at_24_00() {
    let slot = interval("2026-10-07T22:00:00Z", "2026-10-08T00:00:00Z");
    assert_eq!(
        declared_as_opened(slot, Language::English),
        Declared {
            times: "22:00–24:00".into(),
            start: "22:00".into(),
            end: "00:00".into(),
            message: nb("22:00–24:00 counted as work (2h~00m)."),
        }
    );
    assert_eq!(
        declared_as_opened(slot, Language::French),
        Declared {
            times: "22:00–24:00".into(),
            start: "22:00".into(),
            end: "00:00".into(),
            message: nb("22:00–24:00 compté comme travail (2~h~00~min)."),
        }
    );
}

/// A slot whose ends have seconds is saved and reported to the second, as its row in the day
/// shows it.
#[test]
fn a_slot_with_seconds_is_reported_to_the_second() {
    let slot = interval("2026-10-07T09:40:12Z", "2026-10-07T09:45:42Z");
    let declared = declared_as_opened(slot, Language::English);
    assert_eq!(
        (declared.times.as_str(), declared.message),
        (
            "09:40:12–09:45:42",
            nb("09:40:12–09:45:42 counted as work (5m~30s).")
        )
    );
    let declared = declared_as_opened(slot, Language::French);
    assert_eq!(
        (declared.times.as_str(), declared.message),
        (
            "09:40:12–09:45:42",
            nb("09:40:12–09:45:42 compté comme travail (5~min~30~s).")
        )
    );
}

/// Début and Fin typed to midnight report the close of the day, not 00:00.
#[test]
fn an_interval_typed_to_midnight_is_reported_to_24_00() {
    for (language, message) in [
        (Language::English, "23:00–24:00 counted as work (1h~00m)."),
        (
            Language::French,
            "23:00–24:00 compté comme travail (1~h~00~min).",
        ),
    ] {
        let mut form = seeded_at(
            date(2026, 10, 7),
            TimeZone::UTC,
            Seed::Blank,
            "2026-10-08T15:00:00Z",
            language,
        );
        form.start = TimeField::new(23, 0);
        form.end = TimeField::new(0, 0);
        form.submit().unwrap();
        assert_eq!(form.finished(Ok(true)).unwrap().message, nb(message));
    }
}

fn monday() -> Date {
    date(2026, 8, 31)
}

/// Monday's time, which ends at 17:30 with nothing unrecorded, followed by a declaration of
/// `end`'s interval from 17:30, saved to the second.
fn monday_declared_until(end: &str) -> (Demo, WeekState) {
    let (mut demo, mut state) = Demo::open();
    let slot = interval("2026-08-31T17:30:00Z", end);
    let mut form = demo.seeded(monday(), on_slot(slot), "evening");
    assert_eq!(declared_interval(&form), slot);
    assert!(demo.save(&mut form).unwrap().is_some());
    state.reload(demo.host.week().unwrap(), true);
    state.select(monday());
    (demo, state)
}

/// A day ending on a correction less than an hour before now opens from its end up to now,
/// rather than an hour back over the correction, and saves as it opens.
#[test]
fn a_declaration_after_a_late_correction_saves_as_it_opens() {
    let (mut demo, state) = monday_declared_until("2026-08-31T17:42:00Z");
    let (_, seed) = state.declare_work().unwrap();
    assert_eq!(seed, Seed::DeclareAfter(utc("2026-08-31T17:42:00Z")));
    let mut form = AdjustForm::new(
        Target {
            workday: LocalWorkday::new(monday(), "UTC"),
            zone: TimeZone::UTC,
        },
        seed,
        id("after"),
        utc("2026-08-31T18:05:30Z"),
        Language::French,
    );
    assert_eq!(
        (form.start.text(), form.end.text()),
        ("17:42".into(), "18:05".into())
    );
    assert!(demo.save(&mut form).unwrap().is_some());
}

/// A day ending on a correction with seconds opens on the next minute, which Début shows
/// exactly: on the minute before, the declaration would overlap the correction and be refused.
#[test]
fn a_declaration_after_a_correction_with_seconds_saves_as_it_opens() {
    let (mut demo, state) = monday_declared_until("2026-08-31T17:30:43Z");
    let (_, seed) = state.declare_work().unwrap();
    assert_eq!(seed, Seed::DeclareAfter(utc("2026-08-31T17:30:43Z")));
    let mut form = demo.seeded(monday(), seed, "after");
    assert_eq!(
        declared_interval(&form),
        interval("2026-08-31T17:31:00Z", "2026-08-31T18:31:00Z")
    );
    assert!(demo.save(&mut form).unwrap().is_some());
}
