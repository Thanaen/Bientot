use std::sync::Barrier;

use jiff::civil::date;

use super::*;
use crate::database::open;
use crate::test_support::TemporaryDatabase;

fn day() -> LocalWorkday {
    LocalWorkday::new(date(2026, 9, 7), "UTC")
}

fn now() -> Timestamp {
    date(2026, 9, 7)
        .at(17, 45, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn claim(trigger: NotificationTrigger) -> NotificationClaim {
    NotificationClaim {
        workday: day(),
        trigger,
        eligible_at: now(),
    }
}

#[test]
fn concurrent_reservations_claim_combined_delivery_exactly_once() {
    let database = TemporaryDatabase::new();
    let path = database.path();
    let claims = [
        claim(NotificationTrigger::CompletionApproach),
        claim(NotificationTrigger::LatestDepartureApproach),
    ];
    let contenders = 8;
    let start = Barrier::new(contenders);

    let results: Vec<BTreeSet<NotificationTrigger>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..contenders)
            .map(|index| {
                let (path, claims, start) = (&path, &claims, &start);
                scope.spawn(move || {
                    let connection = open(path).unwrap();
                    start.wait();
                    NotificationReceiptRepository::new(&connection)
                        .try_reserve(claims, &format!("delivery-{index}"), now())
                        .unwrap()
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect()
    });

    assert_eq!(results.iter().map(BTreeSet::len).sum::<usize>(), 2);
    assert_eq!(results.iter().filter(|result| result.len() == 2).count(), 1);
    let deliveries: i64 = database
        .connection
        .query_row(
            "SELECT COUNT(DISTINCT CombinedDeliveryId) FROM NotificationReceipt;",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(deliveries, 1);
}

#[test]
fn restart_and_partial_conflict_return_only_new_claims() {
    let database = TemporaryDatabase::new();
    let completion = claim(NotificationTrigger::CompletionApproach);
    let first = NotificationReceiptRepository::new(&database.connection);
    assert_eq!(
        first
            .try_reserve(std::slice::from_ref(&completion), "first", now())
            .unwrap(),
        BTreeSet::from([NotificationTrigger::CompletionApproach])
    );

    let reopened = database.reopen();
    let reserved = NotificationReceiptRepository::new(&reopened)
        .try_reserve(
            &[
                completion,
                claim(NotificationTrigger::LatestDepartureApproach),
            ],
            "second",
            now(),
        )
        .unwrap();

    assert_eq!(
        reserved,
        BTreeSet::from([NotificationTrigger::LatestDepartureApproach])
    );
}

#[test]
fn a_reservation_is_refused_without_a_delivery_id_or_across_workdays() {
    let database = TemporaryDatabase::new();
    let repository = NotificationReceiptRepository::new(&database.connection);
    let other_day = NotificationClaim {
        workday: LocalWorkday::new(date(2026, 9, 8), "UTC"),
        ..claim(NotificationTrigger::DayComplete)
    };

    assert!(matches!(
        repository.try_reserve(&[], " ", now()),
        Err(NotificationReceiptError::BlankDeliveryId)
    ));
    assert!(matches!(
        repository.try_reserve(
            &[claim(NotificationTrigger::DayComplete), other_day],
            "delivery",
            now()
        ),
        Err(NotificationReceiptError::MixedWorkdays)
    ));
    assert_eq!(database.count("NotificationReceipt"), 0);
}
