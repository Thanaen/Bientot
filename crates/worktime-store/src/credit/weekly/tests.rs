use std::sync::Barrier;

use jiff::civil::date;
use jiff::{SignedDuration, Span, Timestamp};
use worktime_domain::accounting::AccountingLabel;
use worktime_domain::history::{
    DailySummaryBreakdown, DailySummaryBreakdownEntry, DailySummaryDisposition,
    DailySummarySnapshot, DailySummarySnapshotParts,
};
use worktime_domain::sources::SourceQuality;
use worktime_domain::summaries::IndicatorState;
use worktime_domain::time::LocalWorkday;

use super::*;
use crate::credit::CreditLedgerRepository;
use crate::database::open;
use crate::test_support::TemporaryDatabase;

fn monday() -> Date {
    date(2026, 9, 7)
}

fn day(offset: i64) -> Date {
    monday() + Span::new().days(offset)
}

fn tuesday() -> Date {
    day(1)
}

fn instant(day: Date) -> Timestamp {
    day.at(16, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

/// `Summary` of the .NET tests: whole minutes against a 7-hour target.
#[derive(Clone)]
struct Summary {
    day: Date,
    revision: i64,
    worked: i64,
    applied: i64,
    earned: i64,
    fingerprint: &'static str,
    available: Option<i64>,
    expired: i64,
}

fn summary(day: Date, revision: i64, worked: i64) -> Summary {
    Summary {
        day,
        revision,
        worked,
        applied: 0,
        earned: 0,
        fingerprint: "facts",
        available: None,
        expired: 0,
    }
}

impl Summary {
    fn build(self) -> DailySummaryRevision {
        let complete = self.worked >= 420 - self.applied;
        let breakdown = if self.worked == 0 {
            DailySummaryBreakdown::default()
        } else {
            DailySummaryBreakdown::new([DailySummaryBreakdownEntry::new(
                DailySummaryDisposition::ConfirmedCredit,
                AccountingLabel::ActiveWork,
                minutes(self.worked),
                None,
                None,
                None,
            )
            .unwrap()])
        };
        let snapshot = DailySummarySnapshot::new(DailySummarySnapshotParts {
            workday: LocalWorkday::new(self.day, "Europe/Paris"),
            computed_at: instant(self.day),
            input_fingerprint: self.fingerprint.to_owned(),
            credited_work: minutes(self.worked),
            excluded_work: SignedDuration::ZERO,
            unknown_work: SignedDuration::ZERO,
            nominal_target: minutes(420),
            effective_target: minutes(420 - self.applied),
            carry_available: minutes(self.available.unwrap_or(self.earned)),
            carry_applied: minutes(self.applied),
            carry_earned: minutes(self.earned),
            carry_expired: minutes(self.expired),
            carry_debt: SignedDuration::ZERO,
            debt_pending: false,
            projected_departure: (!complete)
                .then(|| instant(self.day) + SignedDuration::from_hours(1)),
            indicator_state: if complete {
                IndicatorState::Blue
            } else {
                IndicatorState::Green
            },
            source_quality: SourceQuality::Complete,
            has_pending_review: false,
            breakdown,
        })
        .unwrap();
        DailySummaryRevision::new(self.revision, snapshot).unwrap()
    }
}

fn movement(
    id: &str,
    day: Date,
    kind: CreditLedgerEntryKind,
    amount: i64,
    origin: Date,
    revision: i64,
) -> CreditLedgerEntry {
    CreditLedgerEntry::new(
        CreditLedgerEntryId::new(id).unwrap(),
        day,
        kind,
        minutes(amount),
        CreditOrigin {
            workday_date: origin,
            revision,
        },
        instant(day),
    )
    .unwrap()
}

use CreditLedgerEntryKind::{Applied, Earned, Expired, Reversed};

struct Stores<'a> {
    weekly: WeeklyCreditRepository<'a>,
    ledger: CreditLedgerRepository<'a>,
    summaries: DailySummaryRepository<'a>,
}

fn stores(connection: &Connection) -> Stores<'_> {
    Stores {
        weekly: WeeklyCreditRepository::new(connection),
        ledger: CreditLedgerRepository::new(connection),
        summaries: DailySummaryRepository::new(connection),
    }
}

impl Stores<'_> {
    fn commit_fresh(
        &self,
        summaries: &[DailySummaryRevision],
        ledger: &[CreditLedgerEntry],
    ) -> Result<(), WeeklyCreditError> {
        let expected = self.weekly.read_revision_vector(monday()).unwrap();
        self.weekly.commit(monday(), &expected, summaries, ledger)
    }
}

fn scalar(connection: &Connection, sql: &str) -> i64 {
    connection.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn inconsistency(result: Result<(), WeeklyCreditError>) -> WeeklyInconsistency {
    match result {
        Err(WeeklyCreditError::Inconsistent(inconsistency)) => inconsistency,
        other => panic!("{other:?}"),
    }
}

#[test]
fn read_week_returns_typed_heads_vector_and_ledger_and_rejects_corrupt_latest_breakdown() {
    let database = TemporaryDatabase::new();
    let stores = stores(&database.connection);
    let earned = movement("earned", monday(), Earned, 30, monday(), 1);
    let applied = movement("applied", tuesday(), Applied, 30, monday(), 1);
    stores
        .commit_fresh(
            &[
                Summary {
                    earned: 30,
                    ..summary(monday(), 1, 450)
                }
                .build(),
                Summary {
                    applied: 30,
                    ..summary(tuesday(), 1, 390)
                }
                .build(),
            ],
            &[earned.clone(), applied.clone()],
        )
        .unwrap();

    let state = stores.weekly.read_week(monday()).unwrap();

    assert_eq!(state.week_start_date(), monday());
    let dates: Vec<_> = state
        .latest_revisions()
        .iter()
        .map(DailySummaryRevision::workday_date)
        .collect();
    assert_eq!(dates, [monday(), tuesday()]);
    assert_eq!(state.revision_vector().revisions(), [1, 1, 0, 0, 0]);
    assert_eq!(state.revision_vector().revision(day(2)), Some(0));
    assert_eq!(state.ledger(), [earned, applied]);

    database
        .connection
        .execute(
            "UPDATE DailySummaryRevision SET BreakdownJson = '{}' WHERE WorkdayDate = '2026-09-08';",
            [],
        )
        .unwrap();

    assert!(matches!(
        stores.weekly.read_week(monday()),
        Err(WeeklyCreditError::Storage(
            StorageError::InvalidStoredValue(_)
        ))
    ));
}

#[test]
fn pending_expiry_weeks_return_oldest_prior_weeks_whose_latest_observed_head_has_credit() {
    let database = TemporaryDatabase::new();
    let stores = stores(&database.connection);
    let december_monday = date(2025, 12, 29);
    let january_monday = date(2026, 1, 5);
    let consumed_monday = date(2026, 8, 24);
    let current_monday = date(2026, 9, 14);
    let after = |day: Date, days: i64| day + Span::new().days(days);
    for revision in [
        Summary {
            earned: 30,
            ..summary(after(december_monday, 4), 1, 450)
        },
        Summary {
            earned: 30,
            ..summary(after(january_monday, 3), 1, 450)
        },
        Summary {
            earned: 30,
            ..summary(consumed_monday, 1, 450)
        },
        Summary {
            applied: 30,
            available: Some(0),
            ..summary(after(consumed_monday, 1), 1, 390)
        },
        Summary {
            earned: 30,
            ..summary(current_monday, 1, 450)
        },
    ] {
        stores.summaries.append(&revision.build()).unwrap();
    }

    let pending = stores
        .weekly
        .read_pending_expiry_weeks(current_monday)
        .unwrap();

    assert_eq!(pending, [december_monday, january_monday]);
}

/// The .NET test also reads with a cancelled token; a synchronous read has none.
#[test]
fn pending_expiry_weeks_use_the_latest_revision() {
    let database = TemporaryDatabase::new();
    let stores = stores(&database.connection);
    stores
        .summaries
        .append(
            &Summary {
                earned: 30,
                ..summary(monday(), 1, 450)
            }
            .build(),
        )
        .unwrap();
    stores
        .summaries
        .append(
            &Summary {
                available: Some(0),
                ..summary(monday(), 2, 420)
            }
            .build(),
        )
        .unwrap();

    assert_eq!(stores.weekly.read_pending_expiry_weeks(day(7)).unwrap(), []);
}

#[test]
fn combined_commit_and_fresh_vector_retry_preserve_both_stores_and_other_weeks() {
    let database = TemporaryDatabase::new();
    let stores = stores(&database.connection);
    let other_monday = day(-7);
    let other_summary = Summary {
        earned: 10,
        ..summary(other_monday, 1, 430)
    }
    .build();
    let other_earned = movement("other", other_monday, Earned, 10, other_monday, 1);
    stores
        .weekly
        .commit(
            other_monday,
            &stores.weekly.read_revision_vector(other_monday).unwrap(),
            std::slice::from_ref(&other_summary),
            std::slice::from_ref(&other_earned),
        )
        .unwrap();
    let expected = stores.weekly.read_revision_vector(monday()).unwrap();
    assert_eq!(expected.revisions(), [0; 5]);
    let summaries = [
        Summary {
            earned: 30,
            ..summary(monday(), 1, 450)
        }
        .build(),
        Summary {
            applied: 30,
            ..summary(tuesday(), 1, 390)
        }
        .build(),
    ];
    let ledger = [
        movement("earned", monday(), Earned, 30, monday(), 1),
        movement("applied", tuesday(), Applied, 30, monday(), 1),
    ];

    stores
        .weekly
        .commit(monday(), &expected, &summaries, &ledger)
        .unwrap();
    let fresh = stores.weekly.read_revision_vector(monday()).unwrap();
    stores
        .weekly
        .commit(monday(), &fresh, &summaries, &ledger)
        .unwrap();

    assert_eq!(fresh.revisions(), [1, 1, 0, 0, 0]);
    assert_eq!(
        scalar(
            &database.connection,
            "SELECT COUNT(*) FROM DailySummaryRevision WHERE WorkdayDate >= '2026-09-07';"
        ),
        2
    );
    assert_eq!(stores.ledger.read_week(monday()).unwrap(), ledger);
    assert_eq!(
        stores.ledger.read_week(other_monday).unwrap(),
        [other_earned]
    );
    assert_eq!(
        stores
            .summaries
            .read_latest(other_monday)
            .unwrap()
            .unwrap()
            .snapshot()
            .input_fingerprint(),
        other_summary.snapshot().input_fingerprint()
    );
}

#[test]
fn sql_failure_after_summary_append_and_ledger_delete_rolls_back_both() {
    let database = TemporaryDatabase::new();
    let stores = stores(&database.connection);
    let original = movement("original", monday(), Earned, 30, monday(), 1);
    stores
        .commit_fresh(
            &[Summary {
                earned: 30,
                ..summary(monday(), 1, 450)
            }
            .build()],
            std::slice::from_ref(&original),
        )
        .unwrap();
    database
        .connection
        .execute_batch(
            "CREATE TRIGGER FailReplacement BEFORE INSERT ON CreditLedgerEntry
WHEN NEW.Id = 'blocked'
BEGIN SELECT RAISE(ABORT, 'injected ledger insert failure'); END;",
        )
        .unwrap();

    let result = stores.commit_fresh(
        &[Summary {
            earned: 20,
            ..summary(monday(), 2, 440)
        }
        .build()],
        &[movement("blocked", monday(), Earned, 20, monday(), 2)],
    );

    assert!(
        matches!(
            result,
            Err(WeeklyCreditError::Storage(StorageError::Sqlite(_)))
        ),
        "{result:?}"
    );
    assert_eq!(
        stores
            .summaries
            .read_latest(monday())
            .unwrap()
            .map(|latest| latest.revision()),
        Some(1)
    );
    assert_eq!(
        scalar(
            &database.connection,
            "SELECT COUNT(*) FROM DailySummaryRevision;"
        ),
        1
    );
    assert_eq!(stores.ledger.read_week(monday()).unwrap(), [original]);
}

#[test]
fn expected_missing_tuesday_prevents_stale_monday_prefix_from_deleting_its_credit() {
    let database = TemporaryDatabase::new();
    let stores = stores(&database.connection);
    let earned = movement("earned", monday(), Earned, 30, monday(), 1);
    stores
        .commit_fresh(
            &[Summary {
                earned: 30,
                ..summary(monday(), 1, 450)
            }
            .build()],
            std::slice::from_ref(&earned),
        )
        .unwrap();
    let stale = stores.weekly.read_revision_vector(monday()).unwrap();
    let applied = movement("applied", tuesday(), Applied, 30, monday(), 1);
    stores
        .weekly
        .commit(
            monday(),
            &stale,
            &[Summary {
                applied: 30,
                ..summary(tuesday(), 1, 390)
            }
            .build()],
            &[earned.clone(), applied.clone()],
        )
        .unwrap();

    let result = stores.weekly.commit(
        monday(),
        &stale,
        &[Summary {
            earned: 20,
            ..summary(monday(), 2, 440)
        }
        .build()],
        &[movement("stale", monday(), Earned, 20, monday(), 2)],
    );

    match result {
        Err(WeeklyCreditError::Stale { expected, actual }) => {
            assert_eq!(expected.revision(tuesday()), Some(0));
            assert_eq!(actual.revision(tuesday()), Some(1));
        }
        other => panic!("{other:?}"),
    }
    let latest = |day| {
        stores
            .summaries
            .read_latest(day)
            .unwrap()
            .map(|latest| latest.revision())
    };
    assert_eq!(latest(monday()), Some(1));
    assert_eq!(latest(tuesday()), Some(1));
    assert_eq!(
        stores.ledger.read_week(monday()).unwrap(),
        [earned, applied]
    );
}

#[test]
fn inconsistent_ledger_rolls_back_new_revisions() {
    let id = |id: &str| CreditLedgerEntryId::new(id).unwrap();
    let scenarios = [
        (
            "totals",
            vec![Summary {
                earned: 30,
                ..summary(monday(), 1, 450)
            }],
            vec![movement("earned", monday(), Earned, 20, monday(), 1)],
            WeeklyInconsistency::TotalMismatch {
                workday_date: monday(),
                kind: Earned,
                ledger_ticks: 12_000_000_000,
                summary_ticks: 18_000_000_000,
            },
        ),
        (
            "unknown-origin",
            vec![
                Summary {
                    earned: 30,
                    ..summary(monday(), 1, 450)
                },
                Summary {
                    applied: 30,
                    ..summary(tuesday(), 1, 390)
                },
            ],
            vec![
                movement("earned", monday(), Earned, 30, monday(), 1),
                movement("applied", tuesday(), Applied, 30, monday(), 99),
            ],
            WeeklyInconsistency::UnselectedOrigin(id("applied")),
        ),
        (
            "overdraw",
            vec![
                Summary {
                    earned: 10,
                    ..summary(monday(), 1, 430)
                },
                Summary {
                    applied: 20,
                    ..summary(tuesday(), 1, 400)
                },
            ],
            vec![
                movement("earned", monday(), Earned, 10, monday(), 1),
                movement("applied", tuesday(), Applied, 20, monday(), 1),
            ],
            WeeklyInconsistency::ExceedsEarnedLots {
                workday_date: tuesday(),
                kind: Applied,
            },
        ),
        (
            "same-day-consumption",
            vec![Summary {
                applied: 10,
                earned: 10,
                ..summary(monday(), 1, 420)
            }],
            vec![
                movement("earned", monday(), Earned, 10, monday(), 1),
                movement("applied", monday(), Applied, 10, monday(), 1),
            ],
            WeeklyInconsistency::AppliedFromLaterOrigin(id("applied")),
        ),
        (
            "reversed",
            vec![summary(monday(), 1, 420)],
            vec![movement("reversed", monday(), Reversed, 1, monday(), 1)],
            WeeklyInconsistency::ReversedMovement,
        ),
    ];
    for (scenario, summaries, ledger, expected) in scenarios {
        let database = TemporaryDatabase::new();
        let stores = stores(&database.connection);
        let summaries: Vec<_> = summaries.into_iter().map(Summary::build).collect();

        let result = stores.commit_fresh(&summaries, &ledger);

        assert_eq!(inconsistency(result), expected, "{scenario}");
        assert_eq!(
            scalar(
                &database.connection,
                "SELECT COUNT(*) FROM DailySummaryRevision;"
            ),
            0,
            "{scenario}"
        );
        assert_eq!(stores.ledger.read_week(monday()).unwrap(), [], "{scenario}");
    }
}

#[test]
fn metadata_read_and_replacement_do_not_decode_unsupported_old_details() {
    let database = TemporaryDatabase::new();
    let stores = stores(&database.connection);
    stores
        .summaries
        .append(&summary(monday(), 1, 420).build())
        .unwrap();
    database
        .connection
        .execute("UPDATE DailySummaryRevision SET BreakdownJson = '{}';", [])
        .unwrap();
    let expected = stores.weekly.read_revision_vector(monday()).unwrap();
    assert_eq!(expected.revision(monday()), Some(1));

    stores
        .weekly
        .commit(
            monday(),
            &expected,
            &[Summary {
                earned: 30,
                ..summary(monday(), 2, 450)
            }
            .build()],
            &[movement("earned", monday(), Earned, 30, monday(), 2)],
        )
        .unwrap();

    assert_eq!(
        stores
            .summaries
            .read_latest(monday())
            .unwrap()
            .map(|latest| latest.revision()),
        Some(2)
    );
    assert_eq!(
        scalar(
            &database.connection,
            "SELECT COUNT(*) FROM DailySummaryRevision;"
        ),
        1
    );
}

#[test]
fn simultaneous_commits_have_one_winner_and_no_mixed_snapshot_ledger() {
    let database = TemporaryDatabase::new();
    let expected = WeeklyCreditRepository::new(&database.connection)
        .read_revision_vector(monday())
        .unwrap();
    let path = database.path();
    let start = Barrier::new(2);

    let results: Vec<(&str, bool)> = std::thread::scope(|scope| {
        let workers: Vec<_> = ["left", "right"]
            .into_iter()
            .map(|name| {
                let (start, path) = (&start, &path);
                scope.spawn(move || {
                    let connection = open(path).unwrap();
                    start.wait();
                    let result = WeeklyCreditRepository::new(&connection).commit(
                        monday(),
                        &expected,
                        &[Summary {
                            earned: 30,
                            fingerprint: name,
                            ..summary(monday(), 1, 450)
                        }
                        .build()],
                        &[movement(name, monday(), Earned, 30, monday(), 1)],
                    );
                    match result {
                        Ok(()) => (name, true),
                        Err(WeeklyCreditError::Stale { .. }) => (name, false),
                        Err(error) => panic!("{error}"),
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect()
    });

    let winners: Vec<_> = results.iter().filter(|(_, won)| *won).collect();
    assert_eq!(winners.len(), 1);
    let winner = winners[0].0;
    let stores = stores(&database.connection);
    let latest = stores.summaries.read_latest(monday()).unwrap().unwrap();
    let ledger = stores.ledger.read_week(monday()).unwrap();
    assert_eq!(latest.snapshot().input_fingerprint(), winner);
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].id().as_str(), winner);
    assert_eq!(
        scalar(
            &database.connection,
            "SELECT COUNT(*) FROM DailySummaryRevision;"
        ),
        1
    );
}

#[test]
fn consuming_later_origin_while_older_credit_remains_rolls_back() {
    for expire in [false, true] {
        let database = TemporaryDatabase::new();
        let stores = stores(&database.connection);
        let monday_earned = movement("monday", monday(), Earned, 30, monday(), 1);
        stores
            .commit_fresh(
                &[Summary {
                    earned: 30,
                    ..summary(monday(), 1, 450)
                }
                .build()],
                std::slice::from_ref(&monday_earned),
            )
            .unwrap();
        let consumption_day = day(if expire { 4 } else { 2 });
        let consumption = if expire { Expired } else { Applied };

        let result = stores.commit_fresh(
            &[
                Summary {
                    earned: 30,
                    available: Some(60),
                    ..summary(tuesday(), 1, 450)
                }
                .build(),
                Summary {
                    applied: if expire { 0 } else { 30 },
                    expired: if expire { 30 } else { 0 },
                    available: Some(30),
                    ..summary(consumption_day, 1, 420)
                }
                .build(),
            ],
            &[
                monday_earned.clone(),
                movement("tuesday", tuesday(), Earned, 30, tuesday(), 1),
                movement("later", consumption_day, consumption, 30, tuesday(), 1),
            ],
        );

        assert_eq!(
            inconsistency(result),
            WeeklyInconsistency::NotFifo {
                workday_date: consumption_day,
                kind: consumption,
            }
        );
        assert_eq!(
            scalar(
                &database.connection,
                "SELECT COUNT(*) FROM DailySummaryRevision;"
            ),
            1
        );
        assert_eq!(stores.summaries.read_latest(tuesday()).unwrap(), None);
        assert_eq!(stores.ledger.read_week(monday()).unwrap(), [monday_earned]);
    }
}

#[test]
fn incorrect_available_balance_rolls_back_matching_movement_totals() {
    for (day_offset, available) in [(0, 0), (1, 0), (4, 30)] {
        let database = TemporaryDatabase::new();
        let stores = stores(&database.connection);
        let workday = day(day_offset);
        let expired = if day_offset == 4 { 30 } else { 0 };
        let mut summaries = vec![
            Summary {
                earned: 30,
                available: Some(if day_offset == 0 { available } else { 30 }),
                ..summary(monday(), 1, 450)
            }
            .build(),
        ];
        let mut ledger = vec![movement("earned", monday(), Earned, 30, monday(), 1)];
        if day_offset > 0 {
            summaries.push(
                Summary {
                    available: Some(available),
                    expired,
                    ..summary(workday, 1, 420)
                }
                .build(),
            );
        }
        if expired > 0 {
            ledger.push(movement("expired", workday, Expired, expired, monday(), 1));
        }

        let result = stores.commit_fresh(&summaries, &ledger);

        assert_eq!(
            inconsistency(result),
            WeeklyInconsistency::AvailableMismatch {
                workday_date: workday,
                ledger_ticks: if day_offset == 4 { 0 } else { 18_000_000_000 },
                summary_ticks: available * 600_000_000,
            },
            "day {day_offset}"
        );
        assert_eq!(
            scalar(
                &database.connection,
                "SELECT COUNT(*) FROM DailySummaryRevision;"
            ),
            0
        );
        assert_eq!(stores.ledger.read_week(monday()).unwrap(), []);
    }
}

#[test]
fn partial_friday_expiry_rolls_back_even_when_fifo_and_balances_match() {
    let database = TemporaryDatabase::new();
    let stores = stores(&database.connection);
    let friday = day(4);

    let result = stores.commit_fresh(
        &[
            Summary {
                earned: 30,
                ..summary(monday(), 1, 450)
            }
            .build(),
            Summary {
                expired: 10,
                available: Some(20),
                ..summary(friday, 1, 420)
            }
            .build(),
        ],
        &[
            movement("earned", monday(), Earned, 30, monday(), 1),
            movement("partial-expiry", friday, Expired, 10, monday(), 1),
        ],
    );

    assert_eq!(inconsistency(result), WeeklyInconsistency::PartialExpiry);
    assert_eq!(
        scalar(
            &database.connection,
            "SELECT COUNT(*) FROM DailySummaryRevision;"
        ),
        0
    );
    assert_eq!(stores.ledger.read_week(monday()).unwrap(), []);
}

#[test]
fn shuffled_fifo_movements_persist_available_balances_through_friday_expiry() {
    let database = TemporaryDatabase::new();
    let stores = stores(&database.connection);
    let (wednesday, friday) = (day(2), day(4));
    let ledger = [
        movement("expire", friday, Expired, 20, tuesday(), 1),
        movement("apply-tuesday", wednesday, Applied, 10, tuesday(), 1),
        movement("earn-tuesday", tuesday(), Earned, 30, tuesday(), 1),
        movement("apply-monday", wednesday, Applied, 30, monday(), 1),
        movement("earn-monday", monday(), Earned, 30, monday(), 1),
    ];

    stores
        .commit_fresh(
            &[
                Summary {
                    earned: 30,
                    ..summary(monday(), 1, 450)
                }
                .build(),
                Summary {
                    earned: 30,
                    available: Some(60),
                    ..summary(tuesday(), 1, 450)
                }
                .build(),
                Summary {
                    applied: 40,
                    available: Some(20),
                    ..summary(wednesday, 1, 380)
                }
                .build(),
                Summary {
                    available: Some(0),
                    expired: 20,
                    ..summary(friday, 1, 420)
                }
                .build(),
            ],
            &ledger,
        )
        .unwrap();

    let available = |day| {
        stores
            .summaries
            .read_latest(day)
            .unwrap()
            .unwrap()
            .snapshot()
            .carry_available()
    };
    assert_eq!(stores.ledger.read_week(monday()).unwrap().len(), 5);
    assert_eq!(available(wednesday), minutes(20));
    assert_eq!(available(friday), SignedDuration::ZERO);
}

#[test]
fn a_revision_vector_holds_non_negative_revisions_of_a_week_starting_on_monday() {
    assert_eq!(
        WeeklyRevisionVector::new(tuesday(), [0; 5]),
        Err(InvalidRevisionVector::NotAMonday)
    );
    assert_eq!(
        WeeklyRevisionVector::new(monday(), [0, -1, 0, 0, 0]),
        Err(InvalidRevisionVector::NegativeRevision)
    );
    let vector = WeeklyRevisionVector::new(monday(), [1, 2, 3, 4, 5]).unwrap();
    assert_eq!(vector.revision(day(4)), Some(5));
    assert_eq!(vector.revision(day(5)), None);
}
