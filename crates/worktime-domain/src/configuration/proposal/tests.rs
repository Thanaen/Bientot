use jiff::civil::{Date, Time, Weekday, date, time};

use super::*;

/// A span of hours and minutes, from and to.
type Span = ((i8, i8), (i8, i8));

fn day(month: i8, day: i8, corrected: &[Span]) -> ObservedDay {
    ObservedDay {
        date: date(2026, month, day),
        worked: true,
        corrected: corrected
            .iter()
            .map(|&((sh, sm), (eh, em))| (time(sh, sm, 0, 0), time(eh, em, 0, 0)))
            .collect(),
    }
}

/// Clément's Thursdays: the 1st without a meeting, then three corrections close to 14–16.
fn thursdays() -> Vec<ObservedDay> {
    vec![
        day(10, 1, &[]),
        day(10, 8, &[((14, 5), (15, 50))]),
        day(10, 15, &[((13, 58), (16, 2))]),
        day(10, 22, &[((14, 2), (15, 47))]),
    ]
}

const TODAY: Date = date(2026, 10, 22);

fn find(days: &[ObservedDay], today: Date) -> Option<AppointmentProposal> {
    AppointmentProposal::find(Weekday::Thursday, days, today, &[], &[])
}

#[test]
fn three_similar_thursdays_of_four_propose_every_thursday_on_the_quarter_hours() {
    let proposal = find(&thursdays(), date(2026, 10, 22)).unwrap();
    assert_eq!(proposal.hours(), (time(14, 0, 0, 0), time(16, 0, 0, 0)));
    assert_eq!(proposal.alternate_weeks_from, None);
    assert_eq!(
        proposal
            .occurrences
            .iter()
            .map(|occurrence| occurrence.date)
            .collect::<Vec<_>>(),
        [date(2026, 10, 8), date(2026, 10, 15), date(2026, 10, 22)]
    );
}

#[test]
fn two_corrections_propose_nothing() {
    let days = &thursdays()[..3];
    assert_eq!(find(days, date(2026, 10, 15)), None);
}

/// A Thursday off counts neither way; a Thursday worked without the meeting counts against.
#[test]
fn days_off_are_skipped_and_worked_days_without_the_correction_count_against() {
    let mut days = thursdays();
    days[0].worked = false;
    days.insert(0, day(9, 24, &[]));
    assert!(find(&days, date(2026, 10, 22)).is_some());
    days[2] = day(10, 8, &[((9, 0), (10, 0))]);
    assert_eq!(find(&days, date(2026, 10, 22)), None);
}

/// The proposal answers the latest correction: a Thursday since without it ends it.
#[test]
fn the_latest_worked_day_must_match() {
    let mut days = thursdays();
    days.push(day(10, 29, &[]));
    assert_eq!(find(&days, date(2026, 10, 29)), None);
}

#[test]
fn corrections_close_together_join_and_the_longest_span_counts() {
    let days = vec![
        day(
            10,
            8,
            &[((8, 0), (8, 30)), ((14, 0), (15, 0)), ((15, 3), (16, 0))],
        ),
        day(10, 15, &[((14, 0), (16, 0))]),
        day(10, 22, &[((14, 0), (16, 0))]),
    ];
    let proposal = find(&days, date(2026, 10, 22)).unwrap();
    assert_eq!(proposal.occurrences[0].start, time(14, 0, 0, 0));
    assert_eq!(proposal.occurrences[0].end, time(16, 0, 0, 0));
}

#[test]
fn every_other_thursday_proposes_one_week_in_two_from_the_latest() {
    let days = vec![
        day(9, 24, &[((14, 0), (16, 0))]),
        day(10, 1, &[]),
        day(10, 8, &[((14, 0), (16, 0))]),
        day(10, 15, &[]),
        day(10, 22, &[((14, 0), (16, 0))]),
    ];
    let proposal = find(&days, date(2026, 10, 22)).unwrap();
    assert_eq!(proposal.alternate_weeks_from, Some(date(2026, 10, 22)));
    assert_eq!(proposal.occurrences.len(), 3);
}

#[test]
fn corrections_older_than_eight_weeks_are_not_read() {
    let days = thursdays();
    assert_eq!(find(&days, date(2026, 12, 17)), None);
}

#[test]
fn an_appointment_already_over_those_hours_proposes_nothing() {
    let appointment = RecurringAppointment {
        id: "a".into(),
        name: String::new(),
        weekday: Weekday::Thursday,
        start: time(15, 0, 0, 0),
        end: time(17, 0, 0, 0),
        enabled: false,
        since: None,
        alternate_weeks_from: None,
        automatic: false,
    };
    let days = thursdays();
    let found = |weekday| {
        AppointmentProposal::find(
            Weekday::Thursday,
            &days,
            date(2026, 10, 22),
            &[RecurringAppointment {
                weekday,
                ..appointment.clone()
            }],
            &[],
        )
    };
    assert_eq!(found(Weekday::Thursday), None);
    assert!(found(Weekday::Friday).is_some());
}

/// « Annuler » waits until its day, « Ne plus proposer pour le jeudi » forever, over the same
/// hours of the same weekday only.
#[test]
fn a_refusal_holds_until_its_day_on_its_weekday_and_hours() {
    let days = thursdays();
    let found = |weekday, start: Time, until: Option<Date>| {
        AppointmentProposal::find(
            Weekday::Thursday,
            &days,
            date(2026, 10, 22),
            &[],
            &[DismissedProposal {
                weekday,
                start,
                end: time(16, 0, 0, 0),
                until,
            }],
        )
    };
    let two_pm = time(14, 0, 0, 0);
    assert_eq!(found(Weekday::Thursday, two_pm, None), None);
    assert_eq!(
        found(Weekday::Thursday, two_pm, Some(date(2026, 10, 23))),
        None
    );
    assert!(found(Weekday::Thursday, two_pm, Some(date(2026, 10, 22))).is_some());
    assert!(found(Weekday::Friday, two_pm, None).is_some());
}

#[test]
fn quarter_hours_widen_the_span() {
    assert_eq!(floor_quarter(time(14, 14, 59, 0)), time(14, 0, 0, 0));
    assert_eq!(ceil_quarter(time(15, 45, 0, 0)), time(15, 45, 0, 0));
    assert_eq!(ceil_quarter(time(15, 45, 1, 0)), time(16, 0, 0, 0));
    assert_eq!(ceil_quarter(time(23, 50, 0, 0)), Time::MAX);
}

#[test]
fn a_refusal_replaces_those_of_the_same_hours_and_the_oldest_go_beyond_the_maximum() {
    use super::super::AppointmentProposalSettings;

    let refusal = |weekday, start: i8, end: i8, until| DismissedProposal {
        weekday,
        start: time(start, 0, 0, 0),
        end: time(end, 0, 0, 0),
        until,
    };
    let mut settings = AppointmentProposalSettings::default();
    settings.dismiss(
        refusal(Weekday::Thursday, 14, 16, Some(date(2026, 11, 5))),
        TODAY,
    );
    settings.dismiss(refusal(Weekday::Thursday, 9, 10, None), TODAY);
    settings.dismiss(refusal(Weekday::Thursday, 15, 17, None), TODAY);
    assert_eq!(
        settings.dismissed,
        [
            refusal(Weekday::Thursday, 9, 10, None),
            refusal(Weekday::Thursday, 15, 17, None),
        ]
    );

    for hour in 0..DismissedProposal::MAXIMUM_COUNT {
        let hour = i8::try_from(hour).unwrap();
        settings.dismiss(refusal(Weekday::Monday, hour, hour + 1, None), TODAY);
    }
    assert_eq!(settings.dismissed.len(), DismissedProposal::MAXIMUM_COUNT);
    assert_eq!(settings.dismissed[0], refusal(Weekday::Monday, 0, 1, None));
}

#[test]
fn postponements_over_are_dropped_and_go_before_refusals_for_good() {
    use super::super::AppointmentProposalSettings;

    let refusal = |hour: i8, until| DismissedProposal {
        weekday: Weekday::Monday,
        start: time(hour, 0, 0, 0),
        end: time(hour, 30, 0, 0),
        until,
    };
    let mut settings = AppointmentProposalSettings::default();
    settings.dismiss(refusal(0, Some(date(2026, 10, 20))), date(2026, 10, 1));
    settings.dismiss(refusal(1, None), TODAY);
    assert_eq!(settings.dismissed, [refusal(1, None)]);

    settings.dismiss(refusal(2, Some(date(2026, 11, 5))), TODAY);
    for hour in 3..22 {
        settings.dismiss(refusal(hour, None), TODAY);
    }
    assert_eq!(settings.dismissed.len(), DismissedProposal::MAXIMUM_COUNT);
    assert!(settings.dismissed.iter().all(|kept| kept.until.is_none()));
}

/// « Ne plus proposer pour les jeudis » holds back every Thursday; « Annuler » only the same
/// hours, two weeks.
#[test]
fn a_refusal_for_good_holds_back_the_weekday_and_a_postponement_its_hours() {
    let days = vec![
        day(10, 8, &[((9, 0), (10, 0))]),
        day(10, 15, &[((9, 0), (10, 0))]),
        day(10, 22, &[((9, 0), (10, 0))]),
    ];
    let refusal = |until| DismissedProposal {
        weekday: Weekday::Thursday,
        start: time(14, 0, 0, 0),
        end: time(16, 0, 0, 0),
        until,
    };
    let find_with =
        |refusal| AppointmentProposal::find(Weekday::Thursday, &days, TODAY, &[], &[refusal]);
    assert_eq!(find_with(refusal(None)), None);
    assert!(find_with(refusal(Some(date(2026, 11, 5)))).is_some());
}

#[test]
fn weekends_propose_nothing() {
    let saturdays = vec![
        day(10, 10, &[((14, 0), (16, 0))]),
        day(10, 17, &[((14, 0), (16, 0))]),
        day(10, 24, &[((14, 0), (16, 0))]),
    ];
    assert_eq!(
        AppointmentProposal::find(Weekday::Saturday, &saturdays, date(2026, 10, 24), &[], &[]),
        None
    );
}
