//! The week model against literal texts and geometry, and its commands recorded on a demo
//! ledger as the window records them.

use jiff::civil::time;
use worktime_core::demo::DemoScenario;
use worktime_domain::adjustments::ManualClassification;
use worktime_domain::configuration::{RecurringAppointment, WorkTimeSettings};

use super::test_support::{Throwaway, date, nb};
use super::*;
use crate::demo::DemoHost;

/// A demo's ledger, removed when the test ends.
struct Demo {
    host: DemoHost,
    _directory: Throwaway,
}

impl Demo {
    fn open(scenario: DemoScenario) -> (Self, WeekState) {
        let (host, _directory) = Throwaway::open(scenario);
        let mut demo = Self { host, _directory };
        let state = WeekState::new(demo.read(), Language::French);
        (demo, state)
    }

    fn read(&mut self) -> WeekTimeline {
        self.host.refresh(Language::French).unwrap();
        self.host.week().unwrap()
    }

    /// What the window does with a command: record it, refresh the day, reload the week, then
    /// report the result.
    fn run(
        &mut self,
        state: &mut WeekState,
        command: WeekCommand,
    ) -> Result<Vec<AdjustmentId>, Key> {
        let result = self.host.run_week_command(&command);
        state.reload(self.read(), true);
        state.finished(result.clone().map(|saved| Recorded {
            saved,
            recalculated: true,
            proposal: None,
        }));
        result
    }
}

/// The first correction of the day of `date`.
fn first_correction(state: &WeekState, date: Date) -> SegmentId {
    state
        .timeline
        .day(date)
        .unwrap()
        .segments
        .iter()
        .find(|segment| segment.adjustment_id.is_some())
        .unwrap()
        .id(date)
}

/// The demo's week once Wednesday's long lock is answered with `answer`, the bubble of the
/// correction it leaves open.
fn answered_lock(answer: Answer) -> (Demo, WeekState) {
    let (mut demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    state.settle_next();
    let command = state.choose(answer).unwrap();
    demo.run(&mut state, command).unwrap();
    state.open(first_correction(&state, date(9, 2)));
    (demo, state)
}

fn kinds(state: &WeekState, date: Date) -> Vec<(SegmentKind, String, bool)> {
    state
        .timeline
        .day(date)
        .unwrap()
        .segments
        .iter()
        .map(|segment| {
            (
                segment.kind,
                segment.range_text(Language::French),
                segment.needs_action,
            )
        })
        .collect()
}

#[test]
fn a_five_character_time_needs_52_pixels() {
    assert_eq!(
        segment_label(SegmentKind::Work, "09:00", "Travail", 52.),
        Some("09:00")
    );
    assert_eq!(
        segment_label(SegmentKind::Work, "09:00", "Travail", 51.9),
        None
    );
    assert_eq!(
        segment_label(SegmentKind::Manual, "15:15", "Correction", 52.),
        Some("15:15")
    );
    // Unfilled segments show their word: « À vérifier » is ten characters, 88 px.
    assert_eq!(
        segment_label(SegmentKind::Review, "14:00", "À vérifier", 88.),
        Some("À vérifier")
    );
    assert_eq!(
        segment_label(SegmentKind::Review, "14:00", "À vérifier", 87.9),
        None
    );
    assert_eq!(
        segment_label(SegmentKind::Gap, "14:00", "Trou", 44.8),
        Some("Trou")
    );
    assert_eq!(segment_label(SegmentKind::Gap, "14:00", "Trou", 44.7), None);
}

#[test]
fn short_segments_stay_three_pixels_wide_inside_the_strip() {
    assert_eq!(place(0.25, 0.5, 400.), (100., 200.));
    assert_eq!(place(0.5, 0.001, 400.), (200., 3.));
    assert_eq!(place(0.999, 0.001, 400.), (397., 3.));
    assert_eq!(place(0.5, 0.5, 0.), (0., 0.));
}

#[test]
fn the_axis_leaves_twenty_four_pixels_between_its_labels() {
    // « 00:00 » in IBM Plex Mono at 12 px is 36 px wide: ticks need 60 px.
    let labels = |width| {
        axis(8, 19, width, 36., Language::French)
            .into_iter()
            .map(|(_, label)| label)
            .collect::<Vec<_>>()
    };
    // 620 px for 11 hours: one hour is 56.4 px, two 112.7 px.
    assert_eq!(
        labels(620.),
        ["08:00", "10:00", "12:00", "14:00", "16:00", "18:00"]
    );
    // 560 px for 11 hours: one hour is 50.9 px, two 101.8 px. 19:00 is not on the step and
    // gets no tick, so no label has to give way.
    assert_eq!(
        labels(560.),
        ["08:00", "10:00", "12:00", "14:00", "16:00", "18:00"]
    );
    // 200 px: one hour is 18.2 px, three 54.5 px, four 72.7 px.
    let ticks = axis(8, 19, 200., 36., Language::English);
    assert_eq!(
        ticks
            .iter()
            .map(|(_, label)| label.as_str())
            .collect::<Vec<_>>(),
        ["08:00", "12:00", "16:00"]
    );
    assert!((ticks[1].0 - 4. / 11.).abs() < 1e-6);
}

#[test]
fn the_history_week_lines_up_its_days_on_eight_to_nineteen() {
    let (_demo, state) = Demo::open(DemoScenario::HistoryWeek);
    let week = &state.timeline;
    assert_eq!((week.week_start, week.today), (date(8, 31), date(9, 4)));
    assert_eq!((week.start_hour, week.end_hour), (8, 19));
    assert_eq!(
        kinds(&state, date(9, 2)),
        [
            (SegmentKind::Work, "09:00–10:00".to_owned(), false),
            (SegmentKind::Review, "10:00–12:00".to_owned(), true),
            (SegmentKind::Work, "12:00–14:00".to_owned(), false),
            (SegmentKind::Gap, "14:00–14:15".to_owned(), true),
            (SegmentKind::Work, "14:15–16:45".to_owned(), false),
        ]
    );
    assert_eq!(
        kinds(&state, date(9, 3))[1],
        (SegmentKind::Sleep, "11:00–11:40".to_owned(), true)
    );
    let monday = week.day(date(8, 31)).unwrap();
    // 9:00 is one hour into an eleven-hour axis.
    assert!((monday.segments[0].start - 1. / 11.).abs() < 1e-6);
    let friday = week.day(date(9, 4)).unwrap();
    assert!(friday.is_today);
    assert!((friday.now.unwrap() - 10. / 11.).abs() < 1e-6);
    assert_eq!(week.pending_count(), 4);
}

#[test]
fn the_page_writes_the_week_and_the_open_day() {
    let (_demo, state) = Demo::open(DemoScenario::HistoryWeek);
    let page = state.page();
    assert_eq!(page.header.title, nb("Semaine du 31~août au 4~septembre"));
    assert_eq!(
        page.header.summary,
        nb("38~h~20~min comptées sur 35~h~00~min · 4 créneaux à régler")
    );
    assert_eq!(page.header.settle, "Régler les créneaux (4)");
    let rows: Vec<_> = page
        .days
        .iter()
        .map(|row| {
            (
                row.label.as_str(),
                row.status.as_str(),
                row.detail.as_deref(),
                row.total.as_str(),
                row.target.as_str(),
                row.badge.as_deref(),
            )
        })
        .collect();
    let rows: Vec<_> = rows
        .into_iter()
        .map(|(label, status, detail, total, target, badge)| {
            (
                label,
                status.replace('\u{a0}', "~"),
                detail,
                total.replace('\u{a0}', "~"),
                target.replace('\u{a0}', "~"),
                badge,
            )
        })
        .collect();
    let row = |label, status: &str, detail, total: &str, target: &str, badge| {
        (
            label,
            status.to_owned(),
            detail,
            total.to_owned(),
            target.to_owned(),
            badge,
        )
    };
    assert_eq!(
        rows,
        [
            row(
                "Lun. 31",
                "Objectif atteint",
                None,
                "9~h~00~min",
                "/ 7~h~00~min",
                None
            ),
            row(
                "Mar. 1",
                "Objectif atteint",
                None,
                "7~h~00~min",
                "/ 6~h~00~min",
                None
            ),
            row(
                "Mer. 2",
                "Objectif atteint",
                None,
                "7~h~30~min",
                "/ 6~h~00~min",
                Some("2 à régler")
            ),
            row(
                "Jeu. 3",
                "Il manque 10~min",
                None,
                "5~h~50~min",
                "/ 6~h~00~min",
                Some("1 à régler")
            ),
            row(
                "Ven. 4",
                "Aujourd’hui",
                Some("Objectif atteint"),
                "9~h~00~min",
                "/ 7~h~00~min",
                Some("1 à régler")
            ),
        ]
    );
    let focus = page.days[4].focus.as_ref().unwrap();
    assert_eq!(focus.pending_title, "À régler (1)");
    assert_eq!(focus.pending[0].range, "14:00–18:00");
    assert_eq!(focus.pending[0].kind_name, "À vérifier");
    assert_eq!(focus.pending[0].duration, nb("4~h~00~min"));
    let summary: Vec<_> = focus
        .summary
        .iter()
        .map(|line| {
            (
                line.label,
                line.value.as_str(),
                line.tone,
                line.accessible_name.as_str(),
            )
        })
        .collect();
    let (gap, gap_name) = (nb("+2~h~00~min"), nb("+2~h~00~min de crédit"));
    let (zero, pauses, not_counted) = (nb("0~min"), nb("Pauses 0~min"), nb("Non compté 0~min"));
    assert_eq!(
        summary,
        [
            ("Écart", gap.as_str(), DayTone::Reached, gap_name.as_str()),
            ("Pauses", zero.as_str(), DayTone::Neutral, pauses.as_str()),
            (
                "Non compté",
                zero.as_str(),
                DayTone::Neutral,
                not_counted.as_str()
            ),
        ]
    );
    assert_eq!(
        page.days[4].accessible_name,
        nb(
            "Vendredi 4~septembre, Aujourd’hui, Objectif atteint, 9~h~00~min comptées sur 7~h~00~min, 1 à régler"
        )
    );
}

/// What a screen reader meets on Wednesday's strip: each segment's role and name.
fn wednesday(state: &WeekState, focused: bool, language: Language) -> Vec<(SegmentRole, String)> {
    let focus = if focused { date(9, 2) } else { date(9, 4) };
    let page = WeekPage::project(&state.timeline, focus, language);
    page.days[2]
        .segments
        .iter()
        .map(|segment| (segment.role, segment.accessible_name.replace('\u{a0}', "~")))
        .collect()
}

#[test]
fn every_segment_is_named_by_its_kind_range_and_duration() {
    use SegmentRole::{Button, Label};
    let (_demo, state) = Demo::open(DemoScenario::HistoryWeek);
    let named = |roles: [SegmentRole; 5], names: [&str; 5]| {
        roles
            .into_iter()
            .zip(names.map(str::to_owned))
            .collect::<Vec<_>>()
    };
    let french = [
        "Travail, 09:00–10:00 (1~h~00~min)",
        "À vérifier, 10:00–12:00 (2~h~00~min)",
        "Travail, 12:00–14:00 (2~h~00~min)",
        "Trou, 14:00–14:15 (15~min)",
        "Travail, 14:15–16:45 (2~h~30~min)",
    ];
    let english = [
        "Work, 09:00–10:00 (1h~00m)",
        "To review, 10:00–12:00 (2h~00m)",
        "Work, 12:00–14:00 (2h~00m)",
        "Gap, 14:00–14:15 (15m)",
        "Work, 14:15–16:45 (2h~30m)",
    ];
    // A day that is not open is still read segment by segment.
    assert_eq!(
        wednesday(&state, false, Language::French),
        named([Label; 5], french)
    );
    assert_eq!(
        wednesday(&state, false, Language::English),
        named([Label; 5], english)
    );
    // On the open day every segment selects its range.
    let open = [Button; 5];
    assert_eq!(
        wednesday(&state, true, Language::French),
        named(open, french)
    );
    assert_eq!(
        wednesday(&state, true, Language::English),
        named(open, english)
    );
}

#[test]
fn arrows_move_between_days_and_n_opens_the_next_slot() {
    let (_demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    assert_eq!(state.focused(), date(9, 4));
    state.move_focus(1);
    assert_eq!(state.focused(), date(9, 4));
    state.move_focus(-1);
    assert_eq!(state.focused(), date(9, 3));

    state.settle_next();
    assert_eq!(state.focused(), date(9, 2));
    let bubble = state.bubble().unwrap();
    assert_eq!(
        (bubble.title.as_str(), bubble.duration.as_str()),
        ("10:00–12:00", nb("2~h~00~min").as_str())
    );
    assert_eq!(
        held(&bubble),
        [(SegmentKind::Review, "Verrouillage à vérifier", None)]
    );
    assert_eq!(
        actions(&state),
        [(Answer::Count, true), (Answer::Exclude, true)]
    );
    assert_eq!(bubble.entered(), Some(Answer::Count));

    // Changing day closes the bubble, and Escape has nothing left to close.
    state.move_focus(-1);
    assert_eq!(state.focused(), date(9, 1));
    assert_eq!(state.bubble(), None);
    assert!(!state.close_bubble());
}

/// Each answer of the open bubble, and whether it is offered.
fn actions(state: &WeekState) -> Vec<(Answer, bool)> {
    state
        .bubble()
        .unwrap()
        .actions
        .iter()
        .map(|action| (action.answer, action.enabled))
        .collect()
}

/// Wednesday's segment `index`: work 09:00, the lock 10:00, work 12:00, the gap 14:00 and
/// work 14:15.
fn wednesday_segment(state: &WeekState, index: usize) -> SegmentId {
    state.timeline.day(date(9, 2)).unwrap().segments[index].id(date(9, 2))
}

#[test]
fn a_lock_still_going_on_can_be_neither_answered_nor_finished_at() {
    let (_demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    // Friday's lock still goes on when the week is read.
    let friday = state.timeline.day(date(9, 4)).unwrap().segments[1].id(date(9, 4));
    state.open(friday);
    assert_eq!(
        actions(&state),
        [(Answer::Count, false), (Answer::Exclude, false)]
    );
    assert_eq!(
        state.bubble().unwrap().notes,
        [Language::French.text(Key::HistoryRangeOpenLockNote)]
    );
    assert_eq!(state.choose(Answer::Count), None);
    assert_eq!(state.choose(Answer::Finish), None);
}

/// Work already counts: « Déjà compté comme travail » takes the place of its action, and Enter
/// excludes it. Taking in the gap after it makes counting the likely answer, without Enter.
#[test]
fn a_range_offers_what_changes_it_in_a_fixed_order() {
    let (_demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    state.open(wednesday_segment(&state, 2));
    let bubble = state.bubble().unwrap();
    let shown: Vec<_> = bubble
        .actions
        .iter()
        .map(|action| {
            (
                action.label.as_str(),
                action.already,
                action.accent,
                action.enter,
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("Déjà compté comme travail", true, false, false),
            ("Exclure du travail", false, true, true),
        ]
    );
    assert_eq!(bubble.hint, Some("Ou faites glisser les bords."));
    assert_eq!(
        bubble.preview.as_deref(),
        Some(nb("Total du jour si vous excluez\u{a0}: 7~h~30~min~→ 5~h~30~min.").as_str())
    );

    state.extend(Edge::End);
    let bubble = state.bubble().unwrap();
    assert_eq!(bubble.title, "12:00–14:15");
    assert_eq!(
        held(&bubble),
        [
            (SegmentKind::Work, "Travail", Some(nb("2~h~00~min"))),
            (SegmentKind::Gap, "Trou", Some(nb("15~min"))),
        ]
    );
    let shown: Vec<_> = bubble
        .actions
        .iter()
        .map(|action| (action.answer, action.accent, action.enter))
        .collect();
    assert_eq!(
        shown,
        [
            (Answer::Count, true, false),
            (Answer::Exclude, false, false)
        ]
    );
    assert_eq!(bubble.hint, Some("Ou faites glisser les bords."));
    assert_eq!(
        bubble.preview.as_deref(),
        Some(nb("Total du jour si vous comptez\u{a0}: 7~h~30~min~→ 7~h~45~min.").as_str())
    );
}

/// A range that reaches a lock to review covers the whole lock, which its answer settles too.
#[test]
fn a_range_covers_the_whole_lock_it_touches() {
    let (_demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    state.open(wednesday_segment(&state, 2));
    state.nudge(Edge::Start, -1);
    let bubble = state.bubble().unwrap();
    assert_eq!(bubble.title, "10:00–14:00");
    assert_eq!(
        bubble.notes,
        [Language::French.text(Key::HistoryRangeLockNote)]
    );
}

/// A drag snaps onto a segment's edge within its tolerance, else onto five minutes; De and À
/// move an edge to the time typed; Alt and the arrows move one by five minutes.
#[test]
fn edges_snap_to_segments_and_five_minutes() {
    let (_demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    let gap = wednesday_segment(&state, 3);
    state.open(gap);
    let day = state.timeline.day(date(9, 2)).unwrap();
    let last = &day.segments[4];
    let end_of_day = last.start + last.length;
    let hour = 1. / f32::from(state.timeline.end_hour - state.timeline.start_hour);
    let title = |state: &WeekState| state.bubble().unwrap().title;

    state.drag(Edge::End, end_of_day - hour / 100., hour / 20.);
    assert_eq!(title(&state), "14:00–16:45");
    state.drag(Edge::End, end_of_day - hour / 4. - hour / 100., hour / 100.);
    assert_eq!(title(&state), "14:00–16:30");
    state.nudge(Edge::End, 1);
    assert_eq!(title(&state), "14:00–16:35");
    state.set_time(Edge::End, jiff::civil::time(15, 0, 0, 0));
    assert_eq!(title(&state), "14:00–15:00");
    // À on 00:00 is the midnight closing the day, not the one opening it.
    state.set_time(Edge::End, jiff::civil::Time::midnight());
    assert_eq!(title(&state), "14:00–16:45");
    // An edge dragged past the other stops five minutes before it.
    state.drag(Edge::End, 0., 0.);
    assert_eq!(title(&state), "14:00–14:05");
    assert_eq!(state.selection().unwrap().anchor, None);
}

/// ← and → select the segment before or after; the first press selects the day's last one.
#[test]
fn arrows_select_the_segment_beside() {
    let (_demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    state.select(date(9, 2));
    state.step(-1);
    assert_eq!(state.bubble().unwrap().title, "14:15–16:45");
    state.step(-1);
    assert_eq!(state.bubble().unwrap().title, "14:00–14:15");
    state.step(1);
    state.step(1);
    assert_eq!(state.bubble().unwrap().title, "14:15–16:45");
}

/// A correction clicked offers the other way and its removal, and says what removing it does;
/// one bound to a long-lock check says why it does not change.
#[test]
fn a_correction_offers_the_other_way_and_its_removal() {
    let (_demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    state.open(first_correction(&state, date(9, 1)));
    assert_eq!(
        actions(&state),
        [(Answer::Exclude, true), (Answer::Remove, true)]
    );
    assert_eq!(
        state.bubble().unwrap().notes,
        [Language::French.text(Key::HistoryRangeCorrectionNote)]
    );

    let (_demo, mut state) = answered_lock(Answer::Exclude);
    assert_eq!(
        actions(&state),
        [(Answer::Count, false), (Answer::Remove, false)]
    );
    assert_eq!(
        state.bubble().unwrap().notes,
        [format!(
            "{} {}",
            Language::French.text(Key::HistoryBubbleBoundNote),
            Language::French.text(Key::HistoryRangeEditInDetails)
        )]
    );
    // Nothing to drag nor answer: the range shows as it is, the keyboard's way too.
    assert!(state.bubble().unwrap().fixed);
    let range = state.selection().unwrap().range;
    state.nudge(Edge::End, 1);
    state.extend(Edge::Start);
    assert_eq!(state.selection().unwrap().range, range);
    let (_demo, state) = answered_lock(Answer::Count);
    assert_eq!(
        actions(&state),
        [(Answer::Exclude, false), (Answer::Remove, false)]
    );
    assert_eq!(
        state.bubble().unwrap().notes[0],
        Language::French.text(Key::HistoryConfirmationLocked)
    );
}

/// The demo's week, its day of `date` changed by `change`.
fn with_day(date: Date, change: impl FnOnce(&mut DayTimeline)) -> WeekState {
    let (mut demo, _) = Demo::open(DemoScenario::HistoryWeek);
    let mut timeline = demo.read();
    let day = timeline
        .days
        .iter_mut()
        .find(|day| day.date == date)
        .unwrap();
    change(day);
    WeekState::new(timeline, Language::French)
}

/// « Terminer la journée à HH:MM » of a lock that ended today comes after both answers.
#[test]
fn finishing_at_a_lock_comes_last() {
    let mut state = with_day(date(9, 4), |friday| {
        friday.segments[1].review.as_mut().unwrap().is_open = false;
    });
    state.open(state.timeline.day(date(9, 4)).unwrap().segments[1].id(date(9, 4)));
    let labels: Vec<_> = state
        .bubble()
        .unwrap()
        .actions
        .iter()
        .map(|action| action.label.clone())
        .collect();
    assert_eq!(
        labels,
        [
            "Compter comme travail".to_owned(),
            "Exclure du travail".to_owned(),
            nb("Terminer la journée à~14:00")
        ]
    );
}

/// A recurring appointment to review is answered as a lock, but the day does not finish at
/// its start; the bubble and the segment's name say which appointment it is.
#[test]
fn an_appointment_offers_no_finish() {
    let mut state = with_day(date(9, 4), |friday| {
        let lock = friday.segments[1].review.as_mut().unwrap();
        lock.is_open = false;
        lock.finish_at = None;
        lock.appointment = Some(String::new());
    });
    let segment = &state.timeline.day(date(9, 4)).unwrap().segments[1];
    assert!(
        segment
            .accessible_name(Language::French)
            .starts_with("Rendez-vous du vendredi à vérifier, ")
    );
    state.open(segment.id(date(9, 4)));
    assert_eq!(
        actions(&state),
        [(Answer::Count, true), (Answer::Exclude, true)]
    );
    assert_eq!(
        state.bubble().unwrap().breakdown[0].name,
        "Rendez-vous du vendredi à vérifier"
    );
}

/// Counting the work at 12:00 and the gap after it records the gap alone, which « Annuler »
/// withdraws.
#[test]
fn a_range_records_what_it_changes_and_undo_withdraws_it() {
    let (mut demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    state.open(wednesday_segment(&state, 2));
    state.extend(Edge::End);
    let command = state.choose(Answer::Count).unwrap();
    let saved = demo.run(&mut state, command).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(
        kinds(&state, date(9, 2))[3],
        (SegmentKind::Manual, "14:00–14:15".to_owned(), false)
    );
    let bar = state.info_bar().unwrap();
    assert_eq!(
        bar.message,
        nb("12:00–14:15 compté comme travail (15~min).")
    );
    assert!(bar.can_undo);
    // Sent once; offered again if it fails.
    let undo = state.undo().unwrap();
    assert_eq!(state.undo(), None);
    state.finished(Err(Key::HistoryCorrectionFailed));
    let again = state.undo().unwrap();
    assert_eq!(again, undo);
    assert_eq!(demo.run(&mut state, again), Ok(Vec::new()));
    assert_eq!(
        kinds(&state, date(9, 2))[3],
        (SegmentKind::Gap, "14:00–14:15".to_owned(), true)
    );
}

#[test]
fn confirming_a_long_lock_records_it_and_leaves_three_slots() {
    let (mut demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    state.settle_next();
    let lock = state.selection().unwrap().range;
    let command = state.choose(Answer::Count).unwrap();
    assert_eq!(state.bubble(), None);
    assert_eq!(demo.run(&mut state, command), Ok(Vec::new()));

    assert_eq!(
        kinds(&state, date(9, 2))[1],
        (SegmentKind::Manual, "10:00–12:00".to_owned(), false)
    );
    assert_eq!(state.timeline.pending_count(), 3);
    assert_eq!(
        state.info_bar(),
        Some(InfoBar {
            message: "10:00–12:00 confirmé comme travail.".to_owned(),
            severity: Severity::Success,
            can_undo: false,
            next: Some("Suivant · 3 restant(s)".to_owned()),
            dismissible: true,
            offer: None,
        })
    );
    // Excluding it since is refused: its confirmation stays bound to its review.
    let stale = WeekCommand::Range {
        workday: state
            .timeline
            .day(date(9, 2))
            .unwrap()
            .figures
            .as_ref()
            .unwrap()
            .workday
            .clone(),
        range: lock,
        classification: ManualClassification::Excluded,
    };
    assert_eq!(
        demo.host.run_week_command(&stale),
        Err(Key::HistoryRangeBound)
    );
}

#[test]
fn excluding_a_long_lock_takes_it_out_of_the_day() {
    let (mut demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    state.settle_next();
    let command = state.choose(Answer::Exclude).unwrap();
    assert_eq!(demo.run(&mut state, command), Ok(Vec::new()));
    assert_eq!(
        kinds(&state, date(9, 2))[1],
        (SegmentKind::NotWorked, "10:00–12:00".to_owned(), false)
    );
    assert_eq!(
        state.info_bar().unwrap().message,
        "10:00–12:00 retiré du travail."
    );
    assert_eq!(state.page().days[2].total, nb("5~h~30~min"));
}

#[test]
fn a_settled_gap_offers_undo_and_cannot_be_settled_twice() {
    let (mut demo, mut state) = Demo::open(DemoScenario::ProblemPeriods);
    state.settle_next();
    assert_eq!(state.bubble().unwrap().title, "09:40–09:46");
    let command = state.choose(Answer::Count).unwrap();
    let saved = demo.run(&mut state, command.clone()).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(
        kinds(&state, date(8, 31))[1],
        (SegmentKind::Manual, "09:40:12–09:45:42".to_owned(), false)
    );
    let bar = state.info_bar().unwrap();
    assert_eq!(bar.message, nb("09:40–09:46 compté comme travail (6~min)."));
    assert!(bar.can_undo);
    assert_eq!(bar.next.as_deref(), Some("Suivant · 2 restant(s)"));
    assert_eq!(
        state
            .clone()
            .undo()
            .map(|undo| matches!(undo, WeekCommand::Revoke { .. })),
        Some(true)
    );

    // The slot was settled since it was read: the same answer records nothing more.
    assert_eq!(demo.host.run_week_command(&command), Ok(Vec::new()));
}

#[test]
fn undo_withdraws_a_settled_gap_although_the_demo_clock_stands_still() {
    let (mut demo, mut state) = Demo::open(DemoScenario::ProblemPeriods);
    state.settle_next();
    let command = state.choose(Answer::Count).unwrap();
    demo.run(&mut state, command).unwrap();
    let undo = state.undo().unwrap();
    assert_eq!(demo.run(&mut state, undo), Ok(Vec::new()));
    assert_eq!(
        kinds(&state, date(8, 31))[1],
        (SegmentKind::Gap, "09:40:12–09:45:42".to_owned(), true)
    );
    let bar = state.info_bar().unwrap();
    assert_eq!(
        (bar.message.as_str(), bar.can_undo),
        ("Action annulée.", false)
    );
    assert_eq!(state.timeline.pending_count(), 3);
}

#[test]
fn a_refusal_shows_as_a_warning_and_a_failed_read_as_an_error() {
    let (_demo, mut state) = Demo::open(DemoScenario::ProblemPeriods);
    state.settle_next();
    state.choose(Answer::Exclude).unwrap();
    state.finished(Err(Key::HistoryPeriodResolved));
    let bar = state.info_bar().unwrap();
    assert_eq!(bar.severity, Severity::Warning);
    assert!(
        bar.message
            .starts_with("Cette période a changé depuis son affichage.")
    );
    state.load_failed();
    assert_eq!(state.info_bar().unwrap().severity, Severity::Error);
    state.dismiss_result();
    assert_eq!(state.info_bar(), None);
}

/// What the bubble says the range holds.
fn held(bubble: &Bubble) -> Vec<(SegmentKind, &str, Option<String>)> {
    bubble
        .breakdown
        .iter()
        .map(|held| (held.kind, held.name.as_str(), held.duration.clone()))
        .collect()
}

/// Sleep that a lock confirmed as work leaves uncounted is part of the lock's answer: neither
/// answer changes it.
#[test]
fn sleep_in_a_confirmed_lock_takes_no_answer() {
    let (_demo, mut state) = answered_lock(Answer::Count);
    let id = state.selection().unwrap().anchor.unwrap();
    let day = state
        .timeline
        .days
        .iter_mut()
        .find(|day| day.date == id.date)
        .unwrap();
    let asleep = day
        .segments
        .iter_mut()
        .find(|segment| segment.id(id.date) == id)
        .unwrap();
    assert!(asleep.adjustment_id.is_some());
    asleep.kind = SegmentKind::Sleep;
    state.close_bubble();
    state.open(id);

    assert_eq!(
        actions(&state),
        [
            (Answer::Count, false),
            (Answer::Exclude, false),
            (Answer::Remove, false)
        ]
    );
    assert!(state.bubble().unwrap().fixed);
}

/// A day off takes no correction: its segments are only drawn.
#[test]
fn a_day_off_opens_nothing() {
    let saturday_day_off = date(9, 5);
    let mut state = with_day(date(9, 2), |tuesday| {
        tuesday.figures.as_mut().unwrap().workday.date = saturday_day_off;
    });
    let gap = wednesday_segment(&state, 3);
    state.open(gap);
    assert_eq!(state.bubble(), None);
}

/// A reload keeps the segment clicked while it still selects the range, widens a range that now
/// reaches a lock to cover all of it, and closes the bubble once its day is gone.
#[test]
fn a_reload_renormalizes_the_range() {
    let (mut demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    let gap = wednesday_segment(&state, 3);
    state.open(gap);
    state.reload(demo.read(), true);
    assert_eq!(state.selection().unwrap().anchor, Some(gap));

    let lock = state
        .timeline
        .day(date(9, 2))
        .unwrap()
        .segments
        .iter()
        .find(|segment| segment.review.is_some())
        .unwrap()
        .interval;
    let inside = UtcInterval::new(
        lock.start() + SignedDuration::from_mins(5),
        lock.end() - SignedDuration::from_mins(5),
    )
    .unwrap();
    state.selection = Some(Selection {
        range: inside,
        ..state.selection().unwrap()
    });
    state.reload(demo.read(), true);
    let selection = state.selection().unwrap();
    assert_eq!(selection.range, lock);
    assert_eq!(selection.anchor, None);

    let mut week = demo.read();
    week.days.retain(|day| day.date != date(9, 2));
    state.reload(week, true);
    assert_eq!(state.selection(), None);
}

/// The result bar of a correction that matches the two Thursdays before offers to make it an
/// appointment, until the user answers.
#[test]
fn a_repeated_correction_offers_an_appointment_in_the_result_bar() {
    use worktime_domain::configuration::{AppointmentProposal, Occurrence};

    let (mut demo, mut state) = Demo::open(DemoScenario::HistoryWeek);
    state.settle_next();
    let command = state.choose(Answer::Count).unwrap();
    let result = demo.host.run_week_command(&command);
    state.reload(demo.read(), true);
    let occurrence = |day| Occurrence {
        date: jiff::civil::date(2026, 10, day),
        start: jiff::civil::time(14, 0, 0, 0),
        end: jiff::civil::time(16, 0, 0, 0),
    };
    let proposal = AppointmentProposal {
        weekday: jiff::civil::Weekday::Thursday,
        start: jiff::civil::time(14, 0, 0, 0),
        end: jiff::civil::time(16, 0, 0, 0),
        alternate_weeks_from: None,
        occurrences: vec![occurrence(8), occurrence(15), occurrence(22)],
    };
    state.finished(result.map(|saved| Recorded {
        saved,
        recalculated: true,
        proposal: Some(proposal.clone()),
    }));

    let offer = state.info_bar().unwrap().offer.unwrap();
    assert_eq!(
        offer.note,
        "Même correction que les deux jeudis précédents. En faire un rendez-vous régulier ?"
    );
    assert_eq!(offer.accept, "Compter chaque jeudi…");
    assert_eq!(offer.dismiss, "Ne plus proposer pour les jeudis");
    assert_eq!(state.proposal(), Some(&proposal));

    state.answer_proposal();
    assert_eq!(state.proposal(), None);
    assert_eq!(state.info_bar().unwrap().offer, None);
}

/// The history week with `appointments` in its settings, as the window reads it.
fn with_appointments(appointments: Vec<RecurringAppointment>) -> (Demo, WeekState) {
    let (mut demo, _) = Demo::open(DemoScenario::HistoryWeek);
    let settings = WorkTimeSettings {
        recurring_appointments: appointments,
        ..demo.host.settings().clone()
    };
    demo.host.save_settings(&settings).unwrap();
    let state = WeekState::new(demo.read(), Language::French);
    (demo, state)
}

fn appointment(
    id: &str,
    name: &str,
    weekday: Weekday,
    start: Time,
    end: Time,
) -> RecurringAppointment {
    RecurringAppointment {
        id: id.to_owned(),
        name: name.to_owned(),
        weekday,
        start,
        end,
        enabled: true,
        since: None,
        alternate_weeks_from: None,
        automatic: false,
    }
}

/// Each day's brackets as hours of the axis, and how each end is drawn.
fn markers(state: &WeekState) -> Vec<(Date, String, String, MarkerEnd, MarkerEnd)> {
    let (start_hour, end_hour) = (state.timeline.start_hour, state.timeline.end_hour);
    let clock = |fraction: f32| {
        let minutes = (f64::from(start_hour) * 60.
            + f64::from(fraction) * f64::from(end_hour - start_hour) * 60.)
            .round() as i32;
        format!("{:02}:{:02}", minutes / 60, minutes % 60)
    };
    state
        .timeline
        .days
        .iter()
        .flat_map(|day| {
            day.appointments.iter().map(|marker| {
                (
                    day.date,
                    clock(marker.start),
                    clock(marker.start + marker.length),
                    marker.left,
                    marker.right,
                )
            })
        })
        .collect()
}

/// An appointment gets its bracket on each day it falls on that shows some time, cut at the
/// axis and at now; one switched off, not started yet or still to come gets none.
#[test]
fn an_appointment_is_marked_over_its_hours_up_to_now() {
    let at = |hour, minute| time(hour, minute, 0, 0);
    let (_demo, state) = with_appointments(vec![
        appointment(
            "wednesday",
            "Réunion d’équipe",
            Weekday::Wednesday,
            at(10, 0),
            at(12, 0),
        ),
        // Before the axis, which starts at 8:00.
        appointment("thursday", "", Weekday::Thursday, at(7, 0), at(9, 0)),
        // Around now, Friday at 18:00, then after it.
        appointment("friday", "", Weekday::Friday, at(17, 0), at(19, 30)),
        appointment("evening", "", Weekday::Friday, at(18, 30), at(19, 0)),
        RecurringAppointment {
            enabled: false,
            ..appointment("monday", "", Weekday::Monday, at(10, 0), at(11, 0))
        },
        RecurringAppointment {
            since: Some(date(9, 7)),
            ..appointment("tuesday", "", Weekday::Tuesday, at(10, 0), at(11, 0))
        },
    ]);

    assert_eq!(
        (state.timeline.start_hour, state.timeline.end_hour),
        (8, 19)
    );
    let marker = |day, start: &str, end: &str, left, right| {
        (date(9, day), start.to_owned(), end.to_owned(), left, right)
    };
    use MarkerEnd::{Axis, Now, Planned};
    assert_eq!(
        markers(&state),
        [
            marker(2, "10:00", "12:00", Planned, Planned),
            marker(3, "08:00", "09:00", Axis, Planned),
            marker(4, "17:00", "18:00", Planned, Now),
        ]
    );
}

/// A bracket is named after its appointment and the hours planned, even when it is cut at now.
#[test]
fn a_marker_is_named_after_its_appointment_and_its_planned_hours() {
    let at = |hour, minute| time(hour, minute, 0, 0);
    let (_demo, state) = with_appointments(vec![
        appointment(
            "wednesday",
            "Réunion d’équipe",
            Weekday::Wednesday,
            at(10, 0),
            at(12, 0),
        ),
        appointment("friday", "", Weekday::Friday, at(17, 0), at(19, 30)),
    ]);
    let names = |language| {
        WeekPage::project(&state.timeline, date(9, 4), language)
            .days
            .into_iter()
            .flat_map(|row| row.markers)
            .map(|marker| marker.accessible_name)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(Language::French),
        [
            "Réunion d’équipe, horaire prévu 10:00–12:00",
            "Rendez-vous du vendredi, horaire prévu 17:00–19:30",
        ]
    );
    assert_eq!(
        names(Language::English),
        [
            "Réunion d’équipe, scheduled 10:00–12:00",
            "Friday appointment, scheduled 17:00–19:30",
        ]
    );
}

/// Two appointments over the same hours share one bracket, whose tooltip names both: drawn one
/// over the other, only one could be pointed at.
#[test]
fn appointments_over_the_same_hours_share_one_bracket_naming_each() {
    let at = |hour, minute| time(hour, minute, 0, 0);
    let (_demo, state) = with_appointments(vec![
        appointment(
            "standup",
            "Point d’équipe",
            Weekday::Wednesday,
            at(10, 0),
            at(11, 0),
        ),
        appointment(
            "review",
            "Revue de code",
            Weekday::Wednesday,
            at(10, 0),
            at(11, 0),
        ),
        // Overlapping the first two, past their end.
        appointment("late", "", Weekday::Wednesday, at(10, 30), at(11, 30)),
        // Only touching it: a bracket of its own.
        appointment("after", "", Weekday::Wednesday, at(11, 30), at(12, 0)),
    ]);

    use MarkerEnd::Planned;
    let marker = |start: &str, end: &str| {
        (
            date(9, 2),
            start.to_owned(),
            end.to_owned(),
            Planned,
            Planned,
        )
    };
    assert_eq!(
        markers(&state),
        [marker("10:00", "11:30"), marker("11:30", "12:00")]
    );
    let page = WeekPage::project(&state.timeline, date(9, 2), Language::French);
    let wednesday = page
        .days
        .into_iter()
        .find(|row| row.date == date(9, 2))
        .unwrap();
    assert_eq!(
        wednesday.markers[0].names,
        [
            "Point d’équipe, horaire prévu 10:00–11:00",
            "Revue de code, horaire prévu 10:00–11:00",
            "Rendez-vous du mercredi, horaire prévu 10:30–11:30",
        ]
    );
    assert_eq!(
        wednesday.markers[0].accessible_name,
        "Point d’équipe, horaire prévu 10:00–11:00\u{202f}; \
         Revue de code, horaire prévu 10:00–11:00\u{202f}; \
         Rendez-vous du mercredi, horaire prévu 10:30–11:30"
    );
}

/// A bracket that runs on past now is drawn as going on, even merged with one that ends there.
#[test]
fn a_bracket_running_past_now_goes_on_after_one_ending_there() {
    let at = |hour, minute| time(hour, minute, 0, 0);
    let (_demo, state) = with_appointments(vec![
        appointment("ends", "", Weekday::Friday, at(17, 0), at(18, 0)),
        appointment("goes-on", "", Weekday::Friday, at(17, 30), at(19, 0)),
    ]);
    let ends: Vec<_> = markers(&state)
        .into_iter()
        .map(|(_, start, end, left, right)| (start, end, left, right))
        .collect();
    assert_eq!(
        ends,
        [(
            "17:00".to_owned(),
            "18:00".to_owned(),
            MarkerEnd::Planned,
            MarkerEnd::Now
        )]
    );
}

/// The legend explains the bracket only in a week that draws one, and calls the review segment
/// « À vérifier » whatever its cause.
#[test]
fn the_legend_explains_the_bracket_only_in_a_week_that_draws_one() {
    let at = |hour, minute| time(hour, minute, 0, 0);
    let legend = |state: &WeekState, language: Language| {
        WeekPage::project(&state.timeline, date(9, 4), language)
            .legend()
            .into_iter()
            .map(|(_, key)| language.text(key))
            .collect::<Vec<_>>()
    };
    let (_demo, plain) = with_appointments(Vec::new());
    assert_eq!(
        legend(&plain, Language::French),
        [
            "Travail",
            "Correction manuelle",
            "Pause",
            "Trou ou veille, à régler",
            "À vérifier",
            "Non travaillé",
            "Jour à venir",
            "Maintenant",
        ]
    );
    let (_demo, marked) = with_appointments(vec![appointment(
        "friday",
        "",
        Weekday::Friday,
        at(17, 0),
        at(19, 30),
    )]);
    assert_eq!(
        legend(&marked, Language::English),
        [
            "Work",
            "Manual correction",
            "Break",
            "Gap or sleep, to settle",
            "To review",
            "Not worked",
            "Upcoming day",
            "Scheduled appointment",
            "Now",
        ]
    );
}
