use bientot_domain::time::FrozenClock;
use jiff::SignedDuration;
use jiff::civil::date;

use super::*;
use crate::test_support::TemporaryDatabase;

fn nine() -> Timestamp {
    date(2026, 10, 12)
        .at(9, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn at(second: i64) -> Timestamp {
    nine() + SignedDuration::from_secs(second)
}

fn span(state: IdleState, start_second: i64, settled_second: i64) -> IdleSpan {
    IdleSpan::new(state, at(start_second), at(settled_second)).unwrap()
}

fn selection(second: i64, source: ActivitySourceKind) -> ActivitySourceSelection {
    ActivitySourceSelection {
        effective_from: at(second),
        source,
    }
}

fn read(spans: &ActivitySpanRepository, start_second: i64, end_second: i64) -> Vec<IdleSpan> {
    spans
        .read_settled(at(start_second), at(end_second))
        .unwrap()
        .into_iter()
        .map(|stored| stored.span)
        .collect()
}

fn timeline(
    selections: &ActivitySourceSelectionRepository,
    start_second: i64,
    end_second: i64,
) -> Vec<ActivitySourceSelection> {
    selections
        .read_timeline(at(start_second), Some(at(end_second)))
        .unwrap()
}

#[test]
fn a_transition_closes_the_previous_span_at_the_new_start() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(nine());
    let spans = ActivitySpanRepository::new(&database.connection, &clock);

    let active = spans.append(span(IdleState::Active, 0, 0), None).unwrap();
    spans.advance_settled(active, at(10)).unwrap();
    spans
        .append(span(IdleState::Idle, 12, 15), Some(active))
        .unwrap();

    assert_eq!(
        read(&spans, 0, 60),
        [
            span(IdleState::Active, 0, 12),
            span(IdleState::Idle, 12, 15)
        ]
    );
}

#[test]
fn settled_boundary_only_moves_forward() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(nine());
    let spans = ActivitySpanRepository::new(&database.connection, &clock);
    let id = spans.append(span(IdleState::Idle, 0, 0), None).unwrap();

    spans.advance_settled(id, at(30)).unwrap();
    spans.advance_settled(id, at(20)).unwrap();

    assert_eq!(read(&spans, 0, 60), [span(IdleState::Idle, 0, 30)]);
}

#[test]
fn replaying_a_stored_transition_is_a_no_op() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(nine());
    let spans = ActivitySpanRepository::new(&database.connection, &clock);
    let active = spans.append(span(IdleState::Active, 0, 0), None).unwrap();
    let idle = spans
        .append(span(IdleState::Idle, 12, 12), Some(active))
        .unwrap();
    spans.advance_settled(idle, at(40)).unwrap();

    let replayed = spans
        .append(span(IdleState::Idle, 12, 12), Some(active))
        .unwrap();

    assert_eq!(replayed, idle);
    assert_eq!(
        read(&spans, 0, 60),
        [
            span(IdleState::Active, 0, 12),
            span(IdleState::Idle, 12, 40)
        ]
    );
    assert_eq!(database.count("ActivitySpan"), 2);
}

#[test]
fn a_startup_span_leaves_the_earlier_active_span_and_the_gap_unknown_across_a_reload() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(nine());
    let before_restart = ActivitySpanRepository::new(&database.connection, &clock);
    let active = before_restart
        .append(span(IdleState::Active, 0, 0), None)
        .unwrap();
    before_restart.advance_settled(active, at(600)).unwrap();

    before_restart
        .append(span(IdleState::Active, 7_200, 7_200), None)
        .unwrap();
    let reopened = database.reopen();
    let reloaded = ActivitySpanRepository::new(&reopened, &clock);

    assert_eq!(
        read(&reloaded, 0, 7_201),
        [
            span(IdleState::Active, 0, 600),
            span(IdleState::Active, 7_200, 7_200)
        ]
    );
    assert_eq!(read(&reloaded, 600, 7_200), []);
}

#[test]
fn reads_return_only_spans_whose_settled_part_overlaps_the_request() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(nine());
    let spans = ActivitySpanRepository::new(&database.connection, &clock);
    let first = spans.append(span(IdleState::Active, 0, 100), None).unwrap();
    spans
        .append(span(IdleState::Idle, 100, 300), Some(first))
        .unwrap();

    assert_eq!(read(&spans, 50, 100), [span(IdleState::Active, 0, 100)]);
    assert_eq!(read(&spans, 100, 400), [span(IdleState::Idle, 100, 300)]);
    assert_eq!(read(&spans, 300, 400), []);
}

#[test]
fn selection_lookup_returns_the_selection_in_force_and_every_later_change() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(nine());
    let selections = ActivitySourceSelectionRepository::new(&database.connection, &clock);
    selections
        .append(selection(-3_600, ActivitySourceKind::BuiltIn))
        .unwrap();
    selections
        .append(selection(10_800, ActivitySourceKind::ActivityWatch))
        .unwrap();

    assert_eq!(
        timeline(&selections, 0, 14_400),
        [
            selection(-3_600, ActivitySourceKind::BuiltIn),
            selection(10_800, ActivitySourceKind::ActivityWatch),
        ]
    );
    assert_eq!(
        timeline(&selections, 10_800, 14_400),
        [selection(10_800, ActivitySourceKind::ActivityWatch)]
    );
    assert_eq!(
        timeline(&selections, 0, 10_800),
        [selection(-3_600, ActivitySourceKind::BuiltIn)]
    );
}

#[test]
fn no_source_applies_before_the_first_selection() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(nine());
    let selections = ActivitySourceSelectionRepository::new(&database.connection, &clock);
    assert_eq!(timeline(&selections, 0, 3_600), []);

    selections
        .append(selection(1_800, ActivitySourceKind::BuiltIn))
        .unwrap();

    assert_eq!(timeline(&selections, 0, 1_800), []);
    assert_eq!(
        timeline(&selections, 0, 3_600),
        [selection(1_800, ActivitySourceKind::BuiltIn)]
    );
}

#[test]
fn the_legacy_selection_is_appended_at_the_epoch_only_once() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(nine());
    let selections = ActivitySourceSelectionRepository::new(&database.connection, &clock);

    assert!(selections.append_legacy_selection().unwrap());
    assert!(!selections.append_legacy_selection().unwrap());

    assert_eq!(
        timeline(&selections, 0, 3_600),
        [ActivitySourceSelection {
            effective_from: Timestamp::UNIX_EPOCH,
            source: ActivitySourceKind::ActivityWatch,
        }]
    );
    assert_eq!(database.count("ActivitySourceSelection"), 1);
}

#[test]
fn the_legacy_selection_is_not_appended_once_a_selection_exists() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(nine());
    let selections = ActivitySourceSelectionRepository::new(&database.connection, &clock);
    selections
        .append(selection(0, ActivitySourceKind::BuiltIn))
        .unwrap();

    assert!(!selections.append_legacy_selection().unwrap());

    assert_eq!(timeline(&selections, -3_600, 0), []);
}

#[test]
fn a_timeline_without_end_holds_every_later_selection() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(nine());
    let selections = ActivitySourceSelectionRepository::new(&database.connection, &clock);
    selections.append_legacy_selection().unwrap();
    selections
        .append(selection(0, ActivitySourceKind::BuiltIn))
        .unwrap();
    let last = ActivitySourceSelection {
        effective_from: Timestamp::MAX,
        source: ActivitySourceKind::ActivityWatch,
    };
    selections.append(last).unwrap();

    let all = selections
        .read_timeline(ActivitySourceSelection::LEGACY_EPOCH, None)
        .unwrap();

    assert_eq!(
        all,
        [
            ActivitySourceSelection {
                effective_from: Timestamp::UNIX_EPOCH,
                source: ActivitySourceKind::ActivityWatch,
            },
            selection(0, ActivitySourceKind::BuiltIn),
            ActivitySourceSelection {
                effective_from: crate::codec::truncate_to_tick(Timestamp::MAX),
                ..last
            },
        ]
    );
}

#[test]
fn empty_read_intervals_are_refused() {
    let database = TemporaryDatabase::new();
    let clock = FrozenClock::new(nine());
    let spans = ActivitySpanRepository::new(&database.connection, &clock);
    let selections = ActivitySourceSelectionRepository::new(&database.connection, &clock);

    assert!(matches!(
        spans.read_settled(at(0), at(0)),
        Err(ActivityError::EmptyInterval)
    ));
    assert!(matches!(
        selections.read_timeline(at(0), Some(at(-1))),
        Err(ActivityError::EmptyInterval)
    ));
}
