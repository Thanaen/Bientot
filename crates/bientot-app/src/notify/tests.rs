use std::collections::BTreeSet;

use bientot_domain::notifications::NotificationTrigger;
use bientot_domain::time::FixedTimeZoneProvider;
use jiff::SignedDuration;
use jiff::tz::TimeZone;

use super::*;

fn paris() -> FixedTimeZoneProvider {
    FixedTimeZoneProvider::new(
        "Romance Standard Time",
        TimeZone::get("Europe/Paris").unwrap(),
    )
}

fn notification(triggers: &[NotificationTrigger], remaining: SignedDuration) -> Notification {
    Notification {
        delivery_id: "0123456789abcdef0123456789abcdef".to_owned(),
        triggers: triggers.iter().copied().collect::<BTreeSet<_>>(),
        remaining_work: remaining,
        // 16:45 in Paris on 8 October 2026.
        projected_departure: Some("2026-10-08T14:45:00Z".parse().unwrap()),
        time_zone_id: "Romance Standard Time".to_owned(),
        is_estimated: false,
    }
}

#[test]
fn an_approach_reminder_says_the_minutes_left_and_the_departure_in_the_workday_s_zone() {
    let request = toast(
        &notification(
            &[NotificationTrigger::CompletionApproach],
            SignedDuration::from_secs(14 * 60 + 1),
        ),
        Language::French,
        &paris(),
    );

    assert_eq!(
        request,
        NotificationRequest {
            delivery_id: "0123456789abcdef0123456789abcdef".to_owned(),
            title: "Bientôt".to_owned(),
            body: "Il reste 15 minutes avant votre objectif. Départ prévu\u{a0}: 16:45.".to_owned(),
        }
    );
}

#[test]
fn both_approaches_make_one_sentence_and_an_estimate_says_what_it_counts() {
    let mut both = notification(
        &[
            NotificationTrigger::CompletionApproach,
            NotificationTrigger::LatestDepartureApproach,
        ],
        SignedDuration::from_mins(10),
    );
    both.is_estimated = true;

    assert_eq!(
        toast(&both, Language::English, &paris()).body,
        "10 minutes remain and your preferred departure time is approaching. \
         Projected departure: 4:45\u{a0}PM. Based on recorded work."
    );
}

#[test]
fn a_completed_day_says_so_without_numbers() {
    assert_eq!(
        toast(
            &notification(&[NotificationTrigger::DayComplete], SignedDuration::ZERO),
            Language::French,
            &paris(),
        )
        .body,
        "Votre objectif quotidien est atteint."
    );
}
