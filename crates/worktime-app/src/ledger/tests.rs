use super::*;

fn review(id: &str) -> LongLockReviewId {
    LongLockReviewId::new(id).unwrap()
}

fn gains(minutes: i64) -> Option<Gains> {
    Some(Gains {
        count: SignedDuration::from_mins(minutes),
        exclude: SignedDuration::ZERO,
    })
}

/// The gains of a lock are measured once while its day stays the same, and again once the day
/// changes.
#[test]
fn a_lock_is_measured_once_per_state_of_its_day() {
    let cache = GainsCache::<&str>::default();
    let mut kept = HashMap::new();
    let measured = cache.gains(&review("lock"), &"monday-1", &mut kept, || gains(30));
    assert_eq!(measured, gains(30));
    let again = cache.gains(&review("lock"), &"monday-1", &mut kept, || {
        panic!("the day has not changed")
    });
    assert_eq!(again, gains(30));
    *cache.measured.borrow_mut() = std::mem::take(&mut kept);

    let after_change = cache.gains(&review("lock"), &"monday-2", &mut kept, || gains(45));

    assert_eq!(after_change, gains(45));
}

/// A lock the week no longer reads is forgotten, so the cache holds one week at a time.
#[test]
fn a_lock_the_week_no_longer_reads_is_measured_again() {
    let cache = GainsCache::<&str>::default();
    let mut kept = HashMap::new();
    cache.gains(&review("last-week"), &"monday-1", &mut kept, || gains(30));
    *cache.measured.borrow_mut() = std::mem::take(&mut kept);
    cache.gains(&review("this-week"), &"monday-1", &mut kept, || gains(15));
    *cache.measured.borrow_mut() = std::mem::take(&mut kept);

    let measured = cache.gains(&review("last-week"), &"monday-1", &mut kept, || gains(30));

    assert_eq!(measured, gains(30));
    assert_eq!(cache.measured.borrow().len(), 1);
}
