//! Ported from `Notifications/NotificationIntegrationTests.cs`, with the receipts on a migrated
//! SQLite ledger where .NET used an in-memory fake for all but one test, and from the message
//! choice of `Notifications/NotificationPresentationTests.cs`. The .NET fake text provider
//! printed the triggers, the estimate flag and the departure into the body; the tests here read
//! them from the [`Notification`] the sink receives.
//!
//! Left out of the presentation tests: the resource strings and the culture's time format, which
//! R6 renders, and `PackagedActivationUsesMatchingToastAndComContracts`, a packaging check.

use std::cell::{Cell, RefCell};

use bientot_domain::accounting::{
    AccountingInput, AccountingPolicy, ActivityCoverage, ActivitySegment, ActivityState,
    AutomaticOrigin, WorkdayEventHistory, calculate,
};
use bientot_domain::configuration::{NotificationRule, NotificationSettings};
use bientot_domain::forecasting::{AccountingAssessment, assess};
use bientot_domain::sources::{
    SourceDiagnosticInterval, SourceDiagnostics, SourceFreshness, SourceHealth, SourceIntervalKind,
    SourceQuality,
};
use bientot_domain::summaries::{DailySummaryParts, IndicatorState};
use bientot_domain::time::{FixedTimeZoneProvider, WorkdayWindow};
use jiff::civil::{Date, date};
use jiff::tz::TimeZone;
use rusqlite::Connection;

use super::*;
use crate::accounting::ActivityReadDisposition;
use crate::test_support::{Database, utc_zones};
use crate::weekly::SourceEvidenceUnavailable;
use NotificationTrigger::{CompletionApproach, DayComplete, LatestDepartureApproach};

const DAY: Date = date(2026, 9, 7);

fn workday() -> LocalWorkday {
    LocalWorkday::new(DAY, "UTC")
}

fn at(hour: i8, minute: i8, second: i8) -> Timestamp {
    DAY.at(hour, minute, second, 0)
        .to_zoned(TimeZone::UTC)
        .unwrap()
        .timestamp()
}

fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}

fn interval(start: Timestamp, end: Timestamp) -> UtcInterval {
    UtcInterval::new(start, end).unwrap()
}

const TARGET: SignedDuration = SignedDuration::from_hours(4);

fn lunch() -> UtcInterval {
    interval(at(12, 0, 0), at(14, 0, 0))
}

fn accounting(
    window_end: Timestamp,
    coverage: impl IntoIterator<Item = ActivitySegment>,
) -> bientot_domain::accounting::AccountingResult {
    let window = WorkdayWindow {
        workday: workday(),
        start: at(8, 0, 0),
        end: window_end,
    };
    let bounds = interval(window.start, window.end);
    calculate(
        &AccountingInput::new(
            window,
            window_end,
            AccountingPolicy::new(lunch(), minutes(90)).unwrap(),
            ActivityCoverage::new(bounds, coverage),
            WorkdayEventHistory::default(),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn segment(start: Timestamp, end: Timestamp, state: ActivityState) -> ActivitySegment {
    ActivitySegment::new(
        interval(start, end),
        state,
        AutomaticOrigin::new("activitywatch").unwrap(),
    )
}

/// The .NET `Summary`: active from 08:00 for the target less `remaining`, computed at `computed_at`
/// with a departure projected at 18:20 unless the day is complete.
fn summary(computed_at: Timestamp, remaining: SignedDuration) -> DailySummary {
    build_summary(computed_at, remaining, false)
}

fn degraded_summary(computed_at: Timestamp, remaining: SignedDuration) -> DailySummary {
    build_summary(computed_at, remaining, true)
}

fn build_summary(
    computed_at: Timestamp,
    remaining: SignedDuration,
    degraded: bool,
) -> DailySummary {
    let active_end = at(8, 0, 0) + (TARGET - remaining);
    let accounting = accounting(
        active_end,
        [segment(at(8, 0, 0), active_end, ActivityState::Active)],
    );
    let health = if degraded {
        SourceHealth::degraded(
            "test",
            SourceFreshness::Stale,
            SourceQuality::Complete,
            "stale",
            None,
        )
        .unwrap()
    } else {
        SourceHealth::healthy("test", computed_at)
    };
    let complete = remaining.is_zero();
    DailySummary::new(DailySummaryParts {
        computed_at,
        nominal_target: TARGET,
        effective_target: TARGET,
        departure_projection: (!complete).then_some(DepartureProjection {
            projected_departure: at(18, 20, 0),
            lunch_over: true,
        }),
        indicator_state: if complete {
            IndicatorState::Blue
        } else {
            IndicatorState::Green
        },
        source_diagnostics: SourceDiagnostics::new(
            health,
            interval(at(8, 0, 0), active_end),
            computed_at,
            [],
        )
        .unwrap(),
        assessment: AccountingAssessment::new(
            if degraded {
                ForecastAvailability::Unavailable
            } else {
                ForecastAvailability::Confirmed
            },
            [],
        ),
        accounting,
        credit_result: None,
    })
    .unwrap()
}

/// The .NET `EstimatedSummary`: active from 08:00 for the target less `remaining`, then
/// `unknown` the source could not settle, then away until `computed_at`.
fn estimated_summary(
    computed_at: Timestamp,
    remaining: SignedDuration,
    unknown: SignedDuration,
) -> DailySummary {
    let unknown_start = at(8, 0, 0) + (TARGET - remaining);
    let unknown_end = unknown_start + unknown;
    let accounting = accounting(
        computed_at,
        [
            segment(at(8, 0, 0), unknown_start, ActivityState::Active),
            segment(unknown_end, computed_at, ActivityState::Afk),
        ],
    );
    let diagnostics = SourceDiagnostics::new(
        SourceHealth::healthy("test", computed_at),
        interval(at(8, 0, 0), computed_at),
        computed_at,
        [SourceDiagnosticInterval::new(
            interval(unknown_start, unknown_end),
            SourceIntervalKind::UnresolvedCoverage,
            "activitywatch",
        )
        .unwrap()],
    )
    .unwrap();
    let assessment = assess(&accounting, &diagnostics);
    assert_eq!(assessment.forecast(), ForecastAvailability::Estimated);
    let departure_projection =
        project_departure(computed_at, accounting.credited_work(), TARGET, lunch()).unwrap();
    DailySummary::new(DailySummaryParts {
        computed_at,
        nominal_target: TARGET,
        effective_target: TARGET,
        departure_projection,
        indicator_state: IndicatorState::Green,
        source_diagnostics: diagnostics,
        assessment,
        accounting,
        credit_result: None,
    })
    .unwrap()
}

fn settings_with(notifications: impl FnOnce(&mut NotificationSettings)) -> Settings {
    let mut settings = Settings::default();
    notifications(&mut settings.notifications);
    settings
}

fn completion_only(lead_time: SignedDuration) -> Settings {
    settings_with(|notifications| {
        *notifications = NotificationSettings {
            completion_approach: NotificationRule::new(true, lead_time),
            latest_departure_approach: NotificationRule::new(false, SignedDuration::ZERO),
            day_complete: NotificationRule::new(false, SignedDuration::ZERO),
        };
    })
}

/// Records the toasts, or fails every one of them.
#[derive(Default)]
struct RecordingSink {
    shown: RefCell<Vec<Notification>>,
    attempts: Cell<usize>,
    fails: bool,
}

impl NotificationSink for RecordingSink {
    fn show(
        &self,
        notification: &Notification,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.attempts.set(self.attempts.get() + 1);
        if self.fails {
            return Err("toast failed".into());
        }
        self.shown.borrow_mut().push(notification.clone());
        Ok(())
    }
}

struct Fixture {
    database: Database,
    sink: RecordingSink,
    zones: FixedTimeZoneProvider,
    settings: RefCell<Settings>,
}

impl Fixture {
    fn new() -> Self {
        Self::with_sink(RecordingSink::default())
    }

    fn with_sink(sink: RecordingSink) -> Self {
        Self {
            database: Database::new(),
            sink,
            zones: utc_zones(),
            settings: RefCell::new(Settings::default()),
        }
    }

    fn with_settings(settings: Settings) -> Self {
        let fixture = Self::new();
        fixture.set_settings(settings);
        fixture
    }

    fn set_settings(&self, settings: Settings) {
        *self.settings.borrow_mut() = settings;
    }

    fn coordinator(&self) -> NotificationCoordinator<'_> {
        self.coordinator_on(&self.database.connection)
    }

    fn coordinator_on<'a>(&'a self, connection: &'a Connection) -> NotificationCoordinator<'a> {
        NotificationCoordinator::new(
            NotificationReceiptRepository::new(connection),
            &self.sink,
            &self.zones,
            Box::new(|| self.settings.borrow().clone()),
        )
    }

    fn shown(&self) -> Vec<Notification> {
        self.sink.shown.borrow().clone()
    }

    fn shown_triggers(&self) -> Vec<BTreeSet<NotificationTrigger>> {
        self.shown()
            .into_iter()
            .map(|notification| notification.triggers)
            .collect()
    }

    /// Every receipt: workday, trigger, eligible at, reserved at, delivery id.
    fn receipts(&self) -> Vec<(String, String, String, String, String)> {
        let mut statement = self
            .database
            .connection
            .prepare(
                "SELECT WorkdayDate, Trigger, EligibleAtUtc, ReservedAtUtc, CombinedDeliveryId
                 FROM NotificationReceipt ORDER BY Trigger",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

fn healthy(revision: u64, summary: DailySummary) -> Publication<WorkdayRefreshState> {
    Publication {
        revision,
        summary: WorkdayRefreshState {
            summary: Some(summary),
            source_failure: None,
        },
    }
}

fn observe(coordinator: &mut NotificationCoordinator, revision: u64, summary: DailySummary) {
    coordinator.observe(&healthy(revision, summary)).unwrap();
}

fn triggers<const N: usize>(triggers: [NotificationTrigger; N]) -> BTreeSet<NotificationTrigger> {
    BTreeSet::from(triggers)
}

#[test]
fn split_samples_in_one_minute_produce_one_combined_notification_after_the_minute_ends() {
    let fixture = Fixture::new();
    let mut coordinator = fixture.coordinator();
    observe(&mut coordinator, 1, summary(at(18, 15, 5), minutes(20)));
    observe(&mut coordinator, 2, summary(at(18, 15, 35), minutes(10)));
    assert_eq!(fixture.shown(), []);

    observe(&mut coordinator, 3, summary(at(18, 16, 5), minutes(9)));

    let [notification] = &fixture.shown()[..] else {
        panic!("expected one notification, got {:?}", fixture.shown());
    };
    assert_eq!(
        notification,
        &Notification {
            delivery_id: notification.delivery_id.clone(),
            triggers: triggers([CompletionApproach, LatestDepartureApproach]),
            remaining_work: minutes(9),
            projected_departure: Some(at(18, 20, 0)),
            time_zone_id: "UTC".to_owned(),
            is_estimated: false,
        }
    );
    assert_eq!(notification.body(), NotificationBody::Combined);
    assert_eq!(notification.delivery_id.len(), 32);
    assert!(
        notification
            .delivery_id
            .chars()
            .all(|digit| digit.is_ascii_hexdigit())
    );
    let receipt = |trigger: &str, eligible: &str| {
        (
            "2026-09-07".to_owned(),
            trigger.to_owned(),
            eligible.to_owned(),
            "2026-09-07T18:16:05.0000000+00:00".to_owned(),
            notification.delivery_id.clone(),
        )
    };
    assert_eq!(
        fixture.receipts(),
        [
            receipt("completion-approach", "2026-09-07T18:15:35.0000000+00:00"),
            receipt(
                "latest-departure-approach",
                "2026-09-07T18:15:05.0000000+00:00"
            ),
        ]
    );
}

#[test]
fn an_existing_receipt_does_not_suppress_a_later_completion() {
    let fixture = Fixture::new();
    NotificationReceiptRepository::new(&fixture.database.connection)
        .try_reserve(
            &[NotificationClaim {
                workday: workday(),
                trigger: LatestDepartureApproach,
                eligible_at: at(17, 0, 0),
            }],
            "earlier",
            at(17, 0, 0),
        )
        .unwrap();
    let mut coordinator = fixture.coordinator();

    observe(&mut coordinator, 1, summary(at(18, 15, 5), minutes(10)));
    observe(&mut coordinator, 2, summary(at(18, 16, 5), minutes(9)));

    assert_eq!(fixture.shown_triggers(), [triggers([CompletionApproach])]);
}

#[test]
fn a_disabled_or_degraded_rule_is_revalidated_before_sending() {
    let fixture = Fixture::new();
    let mut coordinator = fixture.coordinator();
    observe(&mut coordinator, 1, summary(at(18, 15, 5), minutes(10)));
    fixture.set_settings(settings_with(|notifications| {
        notifications.completion_approach = NotificationRule::new(false, minutes(15));
        notifications.latest_departure_approach = NotificationRule::new(false, minutes(15));
    }));
    observe(&mut coordinator, 2, summary(at(18, 16, 5), minutes(9)));
    assert_eq!(fixture.shown(), []);

    fixture.set_settings(Settings::default());
    observe(
        &mut coordinator,
        3,
        degraded_summary(at(18, 17, 5), minutes(8)),
    );
    observe(
        &mut coordinator,
        4,
        degraded_summary(at(18, 18, 5), minutes(7)),
    );

    assert_eq!(fixture.shown(), []);
    assert_eq!(fixture.receipts(), []);
}

#[test]
fn completion_after_the_approach_limit_still_sends_separately_when_enabled() {
    let fixture = Fixture::with_settings(settings_with(|notifications| {
        *notifications = NotificationSettings {
            completion_approach: NotificationRule::new(false, minutes(15)),
            latest_departure_approach: NotificationRule::new(false, minutes(15)),
            day_complete: NotificationRule::new(true, SignedDuration::ZERO),
        };
    }));
    let mut coordinator = fixture.coordinator();

    observe(
        &mut coordinator,
        1,
        summary(at(19, 0, 0), SignedDuration::ZERO),
    );

    let shown = fixture.shown();
    assert_eq!(fixture.shown_triggers(), [triggers([DayComplete])]);
    assert_eq!(shown[0].body(), NotificationBody::Complete);
    assert_eq!(shown[0].projected_departure, None);
}

#[test]
fn a_buffered_one_minute_warning_survives_completion_at_the_minute_end() {
    let fixture = Fixture::with_settings(settings_with(|notifications| {
        notifications.completion_approach = NotificationRule::new(true, minutes(1));
        notifications.latest_departure_approach = NotificationRule::new(false, minutes(15));
    }));
    let mut coordinator = fixture.coordinator();

    observe(
        &mut coordinator,
        1,
        summary(at(18, 15, 30), SignedDuration::from_secs(30)),
    );
    observe(
        &mut coordinator,
        2,
        summary(at(18, 16, 0), SignedDuration::ZERO),
    );

    let shown = fixture.shown();
    assert_eq!(fixture.shown_triggers(), [triggers([CompletionApproach])]);
    assert_eq!(shown[0].remaining_work, SignedDuration::ZERO);
    assert_eq!(shown[0].body(), NotificationBody::Complete);
}

#[test]
fn an_estimated_near_completion_does_not_warn_when_possible_unknown_credit_completes_the_day() {
    let fixture = Fixture::with_settings(completion_only(minutes(15)));
    let mut coordinator = fixture.coordinator();

    observe(
        &mut coordinator,
        1,
        estimated_summary(at(18, 15, 0), minutes(10), minutes(15)),
    );
    observe(
        &mut coordinator,
        2,
        estimated_summary(at(18, 16, 0), minutes(9), minutes(15)),
    );

    assert_eq!(fixture.shown(), []);
}

#[test]
fn an_estimated_buffered_warning_is_cancelled_when_unknown_expansion_could_complete_the_day() {
    let fixture = Fixture::with_settings(completion_only(minutes(15)));
    let mut coordinator = fixture.coordinator();

    observe(
        &mut coordinator,
        1,
        estimated_summary(at(18, 15, 0), minutes(10), minutes(4)),
    );
    observe(
        &mut coordinator,
        2,
        estimated_summary(at(18, 16, 0), minutes(9), minutes(15)),
    );

    assert_eq!(fixture.shown(), []);
}

/// Not in .NET: an estimated summary buffers only the reminders both scenarios allow, so a later
/// confirmed summary cannot flush one that the unknown time made premature.
#[test]
fn an_estimated_summary_buffers_only_the_reminders_both_scenarios_allow() {
    let fixture = Fixture::with_settings(completion_only(minutes(15)));
    let mut coordinator = fixture.coordinator();

    observe(
        &mut coordinator,
        1,
        estimated_summary(at(18, 15, 0), minutes(10), minutes(15)),
    );
    observe(&mut coordinator, 2, summary(at(18, 16, 5), minutes(9)));
    assert_eq!(fixture.shown(), []);

    observe(&mut coordinator, 3, summary(at(18, 17, 5), minutes(8)));
    assert_eq!(fixture.shown_triggers(), [triggers([CompletionApproach])]);
}

#[test]
fn an_estimated_warning_across_lunch_uses_the_projected_departure_and_qualifies_the_claim() {
    let fixture = Fixture::with_settings(completion_only(minutes(15)));
    let mut coordinator = fixture.coordinator();
    let warning = estimated_summary(at(13, 59, 0), minutes(10), minutes(5));
    let rollover = estimated_summary(at(14, 0, 0), minutes(9), minutes(5));

    observe(&mut coordinator, 1, warning.clone());
    observe(&mut coordinator, 2, rollover);

    let shown = fixture.shown();
    assert_eq!(fixture.shown_triggers(), [triggers([CompletionApproach])]);
    assert!(shown[0].is_estimated);
    assert_eq!(shown[0].projected_departure, Some(at(14, 9, 0)));
    assert_eq!(shown[0].remaining_work, minutes(9));
    assert_eq!(
        warning
            .departure_projection()
            .map(|projection| projection.projected_departure),
        Some(at(14, 10, 0))
    );
}

#[test]
fn a_backward_clock_correction_resumes_before_the_previous_timestamp() {
    let fixture = Fixture::new();
    let mut coordinator = fixture.coordinator();
    observe(&mut coordinator, 1, summary(at(19, 0, 30), minutes(10)));
    observe(&mut coordinator, 2, summary(at(18, 15, 5), minutes(10)));
    assert_eq!(fixture.shown(), []);

    observe(&mut coordinator, 3, summary(at(18, 16, 5), minutes(9)));

    assert_eq!(
        fixture.shown_triggers(),
        [triggers([CompletionApproach, LatestDepartureApproach])]
    );
}

#[test]
fn an_older_revision_and_backward_time_cannot_flush_or_send() {
    let fixture = Fixture::new();
    let mut coordinator = fixture.coordinator();
    observe(&mut coordinator, 2, summary(at(18, 15, 30), minutes(10)));
    observe(&mut coordinator, 1, summary(at(18, 16, 30), minutes(9)));
    observe(&mut coordinator, 3, summary(at(18, 14, 30), minutes(9)));

    assert_eq!(fixture.shown(), []);
}

/// The receipts keep a reminder from being shown twice when the app restarts and the clock has
/// moved on, however the summaries are numbered.
#[test]
fn receipts_deduplicate_across_a_coordinator_restart() {
    let fixture = Fixture::new();
    let mut first = fixture.coordinator();
    observe(&mut first, 1, summary(at(18, 15, 5), minutes(10)));
    observe(&mut first, 2, summary(at(18, 16, 5), minutes(9)));
    assert_eq!(fixture.shown().len(), 1);
    drop(first);

    let reopened = fixture.database.reopen();
    let mut restarted = fixture.coordinator_on(&reopened);
    observe(&mut restarted, 1, summary(at(18, 17, 5), minutes(8)));
    observe(&mut restarted, 2, summary(at(18, 18, 5), minutes(7)));

    assert_eq!(fixture.shown().len(), 1);
    assert_eq!(fixture.receipts().len(), 2);
}

#[test]
fn a_send_failure_keeps_the_reservation_and_is_not_retried() {
    let fixture = Fixture::with_sink(RecordingSink {
        fails: true,
        ..RecordingSink::default()
    });
    let mut coordinator = fixture.coordinator();
    let mut failures = Vec::new();

    for (revision, computed_at, remaining) in [
        (1, at(18, 15, 5), minutes(10)),
        (2, at(18, 16, 5), minutes(9)),
        (3, at(18, 17, 5), minutes(8)),
        (4, at(18, 18, 5), minutes(7)),
    ] {
        if let Err(error) = coordinator.observe(&healthy(revision, summary(computed_at, remaining)))
        {
            failures.push(error.to_string());
        }
    }

    assert_eq!(fixture.sink.attempts.get(), 1);
    assert_eq!(failures, ["toast failed"]);
    assert_eq!(fixture.receipts().len(), 2);
}

/// Not a .NET coordinator test: the .NET publication handler only passes on a summary the source
/// answered for, which `WorkdayRefreshPresenter` decides. A refresh that kept the last summary
/// beside a source failure is skipped entirely, its revision included.
#[test]
fn a_refresh_that_kept_the_last_summary_beside_a_source_failure_does_not_notify() {
    let fixture = Fixture::new();
    let mut coordinator = fixture.coordinator();
    observe(&mut coordinator, 1, summary(at(18, 15, 5), minutes(10)));
    let retained = summary(at(18, 16, 5), minutes(9));
    let failure = SourceEvidenceUnavailable {
        workday_date: DAY,
        diagnostics: retained.source_diagnostics().clone(),
        disposition: ActivityReadDisposition::Unavailable,
    };

    coordinator
        .observe(&Publication {
            revision: 2,
            summary: WorkdayRefreshState {
                summary: Some(retained),
                source_failure: Some(failure),
            },
        })
        .unwrap();
    assert_eq!(fixture.shown(), []);

    observe(&mut coordinator, 2, summary(at(18, 16, 10), minutes(9)));
    assert_eq!(fixture.shown().len(), 1);
}

fn notification(
    triggers: BTreeSet<NotificationTrigger>,
    remaining_work: SignedDuration,
) -> Notification {
    Notification {
        delivery_id: "delivery".to_owned(),
        triggers,
        remaining_work,
        projected_departure: Some(at(18, 42, 0)),
        time_zone_id: "UTC".to_owned(),
        is_estimated: false,
    }
}

#[test]
fn completion_after_a_buffered_warning_uses_the_complete_body() {
    let complete = notification(triggers([CompletionApproach]), SignedDuration::ZERO);

    assert_eq!(complete.body(), NotificationBody::Complete);
    assert_eq!(complete.body().resource_key(), "NotificationCompleteBody");
    assert_eq!(complete.remaining_minutes(), 0);
}

#[test]
fn departure_and_combined_warnings_report_the_remaining_minutes_rounded_up() {
    let departure = notification(triggers([LatestDepartureApproach]), minutes(12));
    let combined = notification(
        triggers([LatestDepartureApproach, CompletionApproach]),
        minutes(11) + SignedDuration::from_nanos(1),
    );
    let completion = notification(triggers([CompletionApproach]), minutes(12));

    assert_eq!(
        [departure.body(), combined.body(), completion.body()].map(NotificationBody::resource_key),
        [
            "NotificationDepartureBody",
            "NotificationCombinedBody",
            "NotificationCompletionBody"
        ]
    );
    assert_eq!(departure.remaining_minutes(), 12);
    assert_eq!(combined.remaining_minutes(), 12);
    assert_eq!(
        notification(triggers([LatestDepartureApproach]), minutes(-3)).remaining_minutes(),
        0
    );
}
