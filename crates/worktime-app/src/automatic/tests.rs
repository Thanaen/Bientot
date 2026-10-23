use jiff::civil::{Date, date};

use super::*;

const THURSDAY: Date = date(2026, 10, 22);

fn workday() -> LocalWorkday {
    LocalWorkday::new(THURSDAY, "Europe/Paris")
}

fn at(hour: i8) -> Timestamp {
    THURSDAY.at(hour, 0, 0, 0).in_tz("UTC").unwrap().timestamp()
}

fn review() -> LongLockReviewId {
    LongLockReviewId::for_appointment("meeting", at(14))
}

fn count() -> ManualAdjustment {
    ManualAdjustment::reclassification(
        count_id(&workday(), &review()),
        workday(),
        UtcInterval::new(at(14), at(16)).unwrap(),
        ManualClassification::Credited,
        at(16),
    )
    .unwrap()
}

#[test]
fn a_count_names_the_absence_it_records() {
    let count = count();
    assert_eq!(counted_review(count.id()), Some(review()));
    let undone = undo(&count, at(17)).unwrap();
    assert_eq!(counted_review(undone.id()), Some(review()));
}

#[test]
fn undoing_a_count_excludes_the_same_time_in_its_place() {
    let count = count();
    let undone = undo(&count, count.created_at()).unwrap();
    assert_eq!(undone.interval(), count.interval());
    assert_eq!(undone.supersedes_id(), Some(count.id()));
    assert!(undone.created_at() > count.created_at());
    assert_eq!(in_effect(std::slice::from_ref(&count)), vec![&count]);
    assert!(in_effect(&[count, undone]).is_empty());
}

#[test]
fn the_answer_of_a_day_follows_the_count_and_its_undo() {
    let count = count();
    assert_eq!(answer("meeting", &[]), None);
    assert_eq!(answer("meeting", std::slice::from_ref(&count)), Some(true));
    assert_eq!(answer("other", std::slice::from_ref(&count)), None);
    let undone = undo(&count, at(17)).unwrap();
    assert_eq!(answer("meeting", &[count, undone]), Some(false));
}

#[test]
fn the_answer_of_a_day_follows_the_review_resolution() {
    let resolved = |resolution, classification| {
        ManualAdjustment::reclassification(
            worktime_core::reviews::adjustment_id_for(&workday(), &review(), resolution),
            workday(),
            UtcInterval::new(at(14), at(16)).unwrap(),
            classification,
            at(16),
        )
        .unwrap()
    };
    let work = resolved(
        ManualResolution::ConfirmWork,
        ManualClassification::Credited,
    );
    let not_work = resolved(ManualResolution::Exclude, ManualClassification::Excluded);
    assert_eq!(answer("meeting", &[work]), Some(true));
    assert_eq!(answer("meeting", &[not_work]), Some(false));
}
