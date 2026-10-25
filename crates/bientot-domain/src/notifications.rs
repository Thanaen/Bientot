//! Which reminders a workday summary makes eligible.

use std::collections::BTreeSet;

use jiff::{SignedDuration, Timestamp};

use crate::configuration::NotificationSettings;
use crate::time::LocalWorkday;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NotificationTrigger {
    CompletionApproach,
    LatestDepartureApproach,
    DayComplete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationPolicyInput {
    pub workday: LocalWorkday,
    pub observed_at: Timestamp,
    pub remaining_work: SignedDuration,
    pub is_complete: bool,
    pub is_healthy: bool,
    pub preferred_latest_departure: Timestamp,
    pub settings: NotificationSettings,
}

/// Nothing on weekends or while the source is unhealthy. A complete day only allows the
/// completion reminders; an incomplete one allows each approach reminder within its lead time.
pub fn eligible(input: &NotificationPolicyInput) -> BTreeSet<NotificationTrigger> {
    let mut triggers = BTreeSet::new();
    if !input.is_healthy || !input.workday.is_working_day() {
        return triggers;
    }
    let settings = input.settings;
    if input.is_complete {
        if settings.day_complete.enabled {
            triggers.insert(NotificationTrigger::DayComplete);
        }
        if settings.completion_approach.enabled && settings.completion_approach.lead_time.is_zero()
        {
            triggers.insert(NotificationTrigger::CompletionApproach);
        }
        return triggers;
    }
    if settings.completion_approach.enabled
        && input.remaining_work <= settings.completion_approach.lead_time
    {
        triggers.insert(NotificationTrigger::CompletionApproach);
    }
    if settings.latest_departure_approach.enabled
        && input
            .preferred_latest_departure
            .duration_since(input.observed_at)
            <= settings.latest_departure_approach.lead_time
    {
        triggers.insert(NotificationTrigger::LatestDepartureApproach);
    }
    triggers
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;
    use crate::configuration::NotificationRule;

    const TICK: SignedDuration = SignedDuration::from_nanos(100);

    fn utc(hour: i8, minute: i8) -> Timestamp {
        date(2026, 9, 7)
            .at(hour, minute, 0, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap()
            .timestamp()
    }

    fn minutes(minutes: i64) -> SignedDuration {
        SignedDuration::from_mins(minutes)
    }

    fn input(remaining: SignedDuration, complete: bool) -> NotificationPolicyInput {
        NotificationPolicyInput {
            workday: LocalWorkday::new(date(2026, 9, 7), "UTC"),
            observed_at: utc(17, 45),
            remaining_work: remaining,
            is_complete: complete,
            is_healthy: true,
            preferred_latest_departure: utc(18, 0),
            settings: NotificationSettings::default(),
        }
    }

    fn with_settings(
        remaining: SignedDuration,
        complete: bool,
        settings: NotificationSettings,
    ) -> NotificationPolicyInput {
        NotificationPolicyInput {
            settings,
            ..input(remaining, complete)
        }
    }

    fn rule(enabled: bool, lead_minutes: i64) -> NotificationRule {
        NotificationRule::new(enabled, minutes(lead_minutes))
    }

    #[test]
    fn returns_both_approach_triggers_at_inclusive_thresholds() {
        assert_eq!(
            eligible(&input(minutes(15), false)),
            BTreeSet::from([
                NotificationTrigger::CompletionApproach,
                NotificationTrigger::LatestDepartureApproach
            ])
        );
    }

    #[test]
    fn jumping_past_threshold_still_qualifies() {
        assert!(
            eligible(&input(minutes(2), false)).contains(&NotificationTrigger::CompletionApproach)
        );
    }

    #[test]
    fn one_tick_before_either_threshold_does_not_qualify() {
        let input = NotificationPolicyInput {
            observed_at: utc(17, 45) - TICK,
            ..input(minutes(15) + TICK, false)
        };

        assert_eq!(eligible(&input), BTreeSet::new());
    }

    #[test]
    fn zero_lead_completion_fires_on_completed_summary() {
        let settings = NotificationSettings {
            completion_approach: rule(true, 0),
            ..NotificationSettings::default()
        };

        assert!(
            eligible(&with_settings(SignedDuration::ZERO, true, settings))
                .contains(&NotificationTrigger::CompletionApproach)
        );
    }

    #[test]
    fn degraded_and_disabled_rules_do_not_qualify() {
        let settings = NotificationSettings {
            completion_approach: rule(false, 15),
            latest_departure_approach: rule(false, 15),
            day_complete: rule(false, 0),
        };
        let unhealthy = NotificationPolicyInput {
            is_healthy: false,
            ..with_settings(SignedDuration::ZERO, true, settings)
        };

        assert_eq!(eligible(&unhealthy), BTreeSet::new());
        assert_eq!(
            eligible(&with_settings(SignedDuration::ZERO, true, settings)),
            BTreeSet::new()
        );
    }

    #[test]
    fn each_rule_can_be_disabled_independently() {
        let completion_off = NotificationSettings {
            completion_approach: rule(false, 15),
            ..NotificationSettings::default()
        };
        assert!(
            !eligible(&with_settings(minutes(10), false, completion_off))
                .contains(&NotificationTrigger::CompletionApproach)
        );
        let departure_off = NotificationSettings {
            latest_departure_approach: rule(false, 15),
            ..NotificationSettings::default()
        };
        assert!(
            !eligible(&with_settings(minutes(10), false, departure_off))
                .contains(&NotificationTrigger::LatestDepartureApproach)
        );
    }

    #[test]
    fn incomplete_degraded_and_weekend_summaries_do_not_qualify() {
        let unhealthy = NotificationPolicyInput {
            is_healthy: false,
            ..input(minutes(10), false)
        };
        let weekend = NotificationPolicyInput {
            workday: LocalWorkday::new(date(2026, 9, 6), "UTC"),
            ..input(minutes(10), false)
        };

        assert_eq!(eligible(&unhealthy), BTreeSet::new());
        assert_eq!(eligible(&weekend), BTreeSet::new());
    }

    #[test]
    fn day_complete_is_opt_in_and_completion_suppresses_departure() {
        let defaults = eligible(&input(SignedDuration::ZERO, true));
        assert!(!defaults.contains(&NotificationTrigger::DayComplete));
        assert!(!defaults.contains(&NotificationTrigger::LatestDepartureApproach));
        let enabled = NotificationSettings {
            day_complete: rule(true, 0),
            ..NotificationSettings::default()
        };
        assert!(
            eligible(&with_settings(SignedDuration::ZERO, true, enabled))
                .contains(&NotificationTrigger::DayComplete)
        );
    }
}
