//! Properties of notification eligibility and of the settings validation boundary.

use bientot_domain::configuration::{
    ActivityWatchSettings, NotificationRule, NotificationSettings, Settings, SettingsError,
};
use bientot_domain::notifications::{NotificationPolicyInput, NotificationTrigger, eligible};
use bientot_domain::time::LocalWorkday;
use jiff::civil::date;
use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;

fn minute(minute: i64) -> Timestamp {
    "2026-09-07T00:00:00Z".parse::<Timestamp>().unwrap() + SignedDuration::from_mins(minute)
}

fn rule() -> impl Strategy<Value = NotificationRule> {
    (any::<bool>(), 0i64..60)
        .prop_map(|(enabled, lead)| NotificationRule::new(enabled, SignedDuration::from_mins(lead)))
}

fn input() -> impl Strategy<Value = NotificationPolicyInput> {
    (
        0i64..7,
        (900i64..1200, 900i64..1200),
        0i64..90,
        any::<bool>(),
        any::<bool>(),
        (rule(), rule(), rule()),
    )
        .prop_map(
            |(day, (observed, latest), remaining, is_complete, is_healthy, rules)| {
                NotificationPolicyInput {
                    workday: LocalWorkday::new(
                        date(2026, 9, 7)
                            .checked_add(jiff::Span::new().days(day))
                            .unwrap(),
                        "UTC",
                    ),
                    observed_at: minute(observed),
                    remaining_work: SignedDuration::from_mins(remaining),
                    is_complete,
                    is_healthy,
                    preferred_latest_departure: minute(latest),
                    settings: NotificationSettings {
                        completion_approach: rules.0,
                        latest_departure_approach: rules.1,
                        day_complete: rules.2,
                    },
                }
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn only_healthy_working_days_notify_and_only_through_enabled_rules(input in input()) {
        let triggers = eligible(&input);
        let settings = input.settings;

        if !input.is_healthy || !input.workday.is_working_day() {
            prop_assert!(triggers.is_empty());
        }
        if input.is_complete {
            prop_assert!(!triggers.contains(&NotificationTrigger::LatestDepartureApproach));
        } else {
            prop_assert!(!triggers.contains(&NotificationTrigger::DayComplete));
        }
        for (trigger, rule) in [
            (NotificationTrigger::CompletionApproach, settings.completion_approach),
            (NotificationTrigger::LatestDepartureApproach, settings.latest_departure_approach),
            (NotificationTrigger::DayComplete, settings.day_complete),
        ] {
            prop_assert!(rule.enabled || !triggers.contains(&trigger));
        }
    }

    #[test]
    fn less_remaining_work_keeps_the_completion_reminder(input in input(), less in 0i64..90) {
        let closer = NotificationPolicyInput {
            remaining_work: input.remaining_work.min(SignedDuration::from_mins(less)),
            ..input.clone()
        };

        if eligible(&input).contains(&NotificationTrigger::CompletionApproach) {
            prop_assert!(eligible(&closer).contains(&NotificationTrigger::CompletionApproach));
        }
    }

    #[test]
    fn any_address_is_classified_and_an_accepted_one_is_a_plain_http_root(address in "\\PC{0,40}") {
        let settings = Settings {
            activity_watch: ActivityWatchSettings {
                base_url: address.clone(),
                ..ActivityWatchSettings::default()
            },
            ..Settings::default()
        };

        let accepted = !settings.validate().contains(&SettingsError::Source);

        if accepted {
            let lower = address.trim().to_ascii_lowercase();
            prop_assert!(lower.starts_with("http://") || lower.starts_with("https://"), "{address:?}");
            prop_assert!(!address.contains(['?', '#']));
        }
    }
}
