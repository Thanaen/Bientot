use bientot_core::settings::{Pending, SettingsPort};
use bientot_domain::configuration::{RecurringAppointment, Settings};
use bientot_domain::sources::ActivitySourceKind;
use bientot_domain::usage_ping::{Build, Channel};
use bientot_store::settings::SaveSettingsError;
use jiff::civil::{Date, Weekday, date, time};

use super::super::{Clock, Page, SettingsForm, UsagePingFacts};
use crate::adjust::TimePart;
use crate::strings::Language;

#[derive(Default)]
struct Port {
    saved: Vec<Settings>,
}

impl SettingsPort for Port {
    fn save(&mut self, settings: &Settings) -> Result<(), SaveSettingsError> {
        self.saved.push(settings.clone());
        Ok(())
    }

    fn refresh(&mut self) -> bool {
        true
    }
}

const TODAY: Date = date(2026, 10, 22);

fn meeting() -> RecurringAppointment {
    RecurringAppointment {
        id: "thursday-1400".to_owned(),
        name: "Réunion d’équipe".to_owned(),
        weekday: Weekday::Thursday,
        start: time(14, 0, 0, 0),
        end: time(16, 0, 0, 0),
        enabled: true,
        since: Some(date(2026, 9, 3)),
        alternate_weeks_from: None,
        automatic: false,
    }
}

fn accounting(appointments: Vec<RecurringAppointment>, language: Language) -> SettingsForm {
    let settings = Settings {
        activity_source: Some(ActivitySourceKind::BuiltIn),
        recurring_appointments: appointments,
        ..Settings::default()
    };
    let facts = UsagePingFacts {
        build: Build::new("1.0.0", Channel::Store).unwrap(),
        released: None,
        today: TODAY,
        last_sent: Some(TODAY),
    };
    let mut form = SettingsForm::new(settings, facts, language);
    assert!(form.go_to(Page::Accounting));
    form
}

/// Saves what `pending` holds and returns the appointments saved.
fn save(form: &mut SettingsForm, pending: Option<Pending>) -> Vec<RecurringAppointment> {
    let mut port = Port::default();
    form.run(pending.expect("the change is accepted"), &mut port);
    port.saved
        .pop()
        .expect("the settings are saved")
        .recurring_appointments
}

#[test]
fn the_section_lists_the_appointments_or_says_there_are_none() {
    let empty = accounting(Vec::new(), Language::French)
        .appointments()
        .unwrap();
    assert!(empty.rows.is_empty());
    assert_eq!(
        empty.empty,
        Some("Aucun rendez-vous régulier pour l’instant.")
    );

    let monday = RecurringAppointment {
        id: "monday-0900".to_owned(),
        name: String::new(),
        weekday: Weekday::Monday,
        start: time(9, 0, 0, 0),
        end: time(10, 0, 0, 0),
        enabled: false,
        since: None,
        alternate_weeks_from: None,
        automatic: false,
    };
    let view = accounting(vec![meeting(), monday], Language::French)
        .appointments()
        .unwrap();
    assert_eq!(view.empty, None);
    let rows: Vec<_> = view
        .rows
        .iter()
        .map(|row| (row.title.as_str(), row.description.as_str(), row.enabled))
        .collect();
    assert_eq!(
        rows,
        [
            ("Réunion d’équipe", "Chaque jeudi · 14:00–16:00", true),
            ("Rendez-vous du lundi", "Chaque lundi · 09:00–10:00", false),
        ]
    );
    assert_eq!(view.rows[0].edit_name, "Modifier Réunion d’équipe");
}

#[test]
fn the_section_is_only_on_accounting() {
    let mut form = accounting(vec![meeting()], Language::English);
    assert!(form.go_to(Page::Schedule));
    assert_eq!(form.appointments(), None);
}

#[test]
fn a_new_appointment_starts_on_thursday_afternoon_and_counts_from_today() {
    let mut form = accounting(Vec::new(), Language::French);
    form.open_appointment(None);
    let editor = form.appointments().unwrap().editor.unwrap();
    assert_eq!(editor.name_placeholder, "Rendez-vous du jeudi");
    assert_eq!(editor.save, "Créer le rendez-vous");
    assert_eq!(editor.delete, None);
    assert_eq!(form.time(Clock::AppointmentStart).text(), "14:00");
    assert_eq!(form.time(Clock::AppointmentEnd).text(), "16:00");

    form.choose_appointment_day(Weekday::Tuesday);
    let pending = form.save_appointment("  Formation  ");
    assert_eq!(form.appointment_draft(), None);

    assert_eq!(
        save(&mut form, pending),
        [RecurringAppointment {
            id: "tuesday-1400-20261022".to_owned(),
            name: "Formation".to_owned(),
            weekday: Weekday::Tuesday,
            start: time(14, 0, 0, 0),
            end: time(16, 0, 0, 0),
            enabled: true,
            since: Some(TODAY),
            alternate_weeks_from: None,
            automatic: false,
        }]
    );
}

#[test]
fn a_second_appointment_at_the_same_time_gets_its_own_id() {
    let mut form = accounting(vec![meeting()], Language::French);
    for _ in 0..2 {
        form.open_appointment(None);
        let pending = form.save_appointment("");
        save(&mut form, pending);
    }
    let ids: Vec<_> = form
        .current()
        .recurring_appointments
        .iter()
        .map(|appointment| appointment.id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "thursday-1400",
            "thursday-1400-20261022",
            "thursday-1400-20261022-2"
        ]
    );
}

#[test]
fn no_appointment_is_added_past_the_limit() {
    let many = (0..RecurringAppointment::MAXIMUM_COUNT)
        .map(|index| RecurringAppointment {
            id: format!("thursday-{index}"),
            ..meeting()
        })
        .collect();
    let mut form = accounting(many, Language::French);
    assert_eq!(form.appointments().unwrap().add, None);

    form.open_appointment(None);
    assert_eq!(form.save_appointment(""), None);
    assert_eq!(form.appointment_draft(), None);
    assert_eq!(
        form.appointments().unwrap().rows.len(),
        RecurringAppointment::MAXIMUM_COUNT
    );
}

#[test]
fn leaving_the_page_closes_the_editor() {
    let mut form = accounting(vec![meeting()], Language::French);
    form.open_appointment(Some("thursday-1400"));
    assert!(form.go_to(Page::Schedule));
    assert!(form.go_to(Page::Accounting));
    assert_eq!(form.appointments().unwrap().editor, None);
}

#[test]
fn hours_that_do_not_follow_each_other_are_refused_under_the_hours() {
    let mut form = accounting(Vec::new(), Language::English);
    form.open_appointment(None);
    form.time_mut(Clock::AppointmentEnd)
        .step(TimePart::Hours, -3);

    assert_eq!(form.save_appointment(""), None);
    let editor = form.appointments().unwrap().editor.unwrap();
    assert_eq!(
        editor.hours_error,
        Some("The end time must come after the start time.")
    );
    assert_eq!(
        form.appointment_draft().map(|draft| draft.invalid),
        Some(true)
    );
}

#[test]
fn renaming_keeps_the_day_the_rule_counts_from_but_new_hours_count_from_today() {
    let mut form = accounting(vec![meeting()], Language::French);
    form.open_appointment(Some("thursday-1400"));
    let editor = form.appointments().unwrap().editor.unwrap();
    assert_eq!(editor.save, "Enregistrer");
    assert_eq!(editor.delete, Some("Supprimer"));
    assert_eq!(form.appointments().unwrap().rows[0].edit, None);

    let pending = form.save_appointment("Comité");
    let saved = save(&mut form, pending);
    assert_eq!(saved[0].name, "Comité");
    assert_eq!(saved[0].since, Some(date(2026, 9, 3)));

    assert_eq!(
        form.open_appointment(Some("thursday-1400")).as_deref(),
        Some("Comité")
    );
    form.time_mut(Clock::AppointmentEnd)
        .step(TimePart::Hours, 1);
    let pending = form.save_appointment("Comité");
    let saved = save(&mut form, pending);
    assert_eq!(saved[0].id, "thursday-1400");
    assert_eq!(saved[0].end, time(17, 0, 0, 0));
    assert_eq!(saved[0].since, Some(TODAY));
}

#[test]
fn the_switch_and_delete_save_the_list() {
    let mut form = accounting(vec![meeting()], Language::French);
    let pending = form.toggle_appointment("thursday-1400");
    assert!(!save(&mut form, pending)[0].enabled);

    form.open_appointment(Some("thursday-1400"));
    assert_eq!(form.delete_appointment(), None);
    let editor = form.appointments().unwrap().editor.unwrap();
    assert_eq!(editor.delete, Some("Confirmer la suppression"));
    let pending = form.delete_appointment();
    assert_eq!(save(&mut form, pending), []);
    assert_eq!(
        form.appointments().unwrap().empty,
        Some("Aucun rendez-vous régulier pour l’instant.")
    );
}

#[test]
fn one_week_in_two_starts_on_the_next_day_chosen_and_reads_so_in_the_row() {
    let mut form = accounting(Vec::new(), Language::French);
    form.open_appointment(None);
    form.choose_appointment_repeat(true);
    let next = form.appointments().unwrap().editor.unwrap().next.unwrap();
    let weeks: Vec<_> = next.weeks.iter().map(|week| week.label.as_str()).collect();
    // TODAY is a Thursday whose appointment may be over: the dates offered come after it.
    assert_eq!(weeks, ["29\u{a0}octobre", "5\u{a0}novembre"]);
    assert_eq!(next.weeks[0].full, "jeudi\u{a0}29\u{a0}octobre");

    form.choose_appointment_week(true);
    let pending = form.save_appointment("");
    let saved = save(&mut form, pending);
    assert_eq!(saved[0].alternate_weeks_from, Some(date(2026, 11, 5)));
    assert_eq!(
        form.appointments().unwrap().rows[0].description,
        "Un jeudi sur deux · 14:00–16:00"
    );
}

#[test]
fn an_unchanged_week_in_two_keeps_its_days_and_a_switched_one_counts_from_today() {
    let alternate = RecurringAppointment {
        alternate_weeks_from: Some(date(2026, 9, 3)),
        ..meeting()
    };
    let mut form = accounting(vec![alternate], Language::English);
    form.open_appointment(Some("thursday-1400"));
    let editor = form.appointments().unwrap().editor.unwrap();
    assert!(editor.repeats[1].chosen);
    // 3 September and 29 October are eight weeks apart: 29 October is in its weeks.
    assert!(editor.next.unwrap().weeks[0].chosen);

    let pending = form.save_appointment("Comité");
    let saved = save(&mut form, pending);
    assert_eq!(saved[0].alternate_weeks_from, Some(date(2026, 9, 3)));
    assert_eq!(saved[0].since, Some(date(2026, 9, 3)));

    form.open_appointment(Some("thursday-1400"));
    form.choose_appointment_week(true);
    let pending = form.save_appointment("Comité");
    let saved = save(&mut form, pending);
    assert_eq!(saved[0].alternate_weeks_from, Some(date(2026, 11, 5)));
    assert_eq!(saved[0].since, Some(TODAY));

    form.open_appointment(Some("thursday-1400"));
    form.choose_appointment_repeat(false);
    let pending = form.save_appointment("Comité");
    assert_eq!(save(&mut form, pending)[0].alternate_weeks_from, None);
    assert_eq!(
        form.appointments().unwrap().rows[0].description,
        "Every Thursday · 14:00–16:00"
    );
}

/// « Proposer des rendez-vous d’après vos corrections » sits under the appointments, on, and
/// turning it off keeps the proposals turned down.
#[test]
fn the_proposals_switch_turns_proposals_off_and_keeps_the_refusals() {
    use super::super::{Control, Switch};
    use bientot_domain::configuration::DismissedProposal;

    let refusal = DismissedProposal {
        weekday: Weekday::Thursday,
        start: time(14, 0, 0, 0),
        end: time(16, 0, 0, 0),
        until: None,
    };
    let settings = Settings {
        activity_source: Some(ActivitySourceKind::BuiltIn),
        appointment_proposals: bientot_domain::configuration::AppointmentProposalSettings {
            enabled: true,
            dismissed: vec![refusal],
        },
        ..Settings::default()
    };
    let facts = UsagePingFacts {
        build: Build::new("1.0.0", Channel::Store).unwrap(),
        released: None,
        today: TODAY,
        last_sent: Some(TODAY),
    };
    let mut form = SettingsForm::new(settings, facts, Language::French);
    assert!(form.go_to(Page::Accounting));
    let view = form.appointments().unwrap();
    assert_eq!(
        view.proposals.title,
        "Proposer des rendez-vous d’après vos corrections"
    );
    assert!(matches!(
        view.proposals.control,
        Control::Switch {
            switch: Switch::AppointmentProposals,
            on: true,
            ..
        }
    ));

    let pending = form.toggle(Switch::AppointmentProposals);
    let mut port = Port::default();
    form.run(pending.expect("the change is accepted"), &mut port);
    let saved = port.saved.pop().unwrap().appointment_proposals;
    assert!(!saved.enabled);
    assert_eq!(saved.dismissed, [refusal]);
}

fn proposal(
    alternate_weeks_from: Option<Date>,
) -> bientot_domain::configuration::AppointmentProposal {
    bientot_domain::configuration::AppointmentProposal {
        weekday: Weekday::Thursday,
        start: time(14, 0, 0, 0),
        end: time(16, 0, 0, 0),
        alternate_weeks_from,
        occurrences: Vec::new(),
    }
}

/// « Compter chaque jeudi… » opens « Comptabilisation » on a new appointment with the hours of
/// the corrections, and creating it records no refusal.
#[test]
fn a_proposal_opens_the_editor_filled_in_and_creates_the_appointment() {
    let mut form = accounting(Vec::new(), Language::French);
    assert!(form.go_to(Page::General));
    assert!(form.open_proposal(&proposal(None)));
    let view = form.appointments().expect("on « Comptabilisation »");
    let editor = view.editor.expect("the editor is open");
    assert!(
        editor
            .days
            .iter()
            .any(|day| day.chosen && day.weekday == Weekday::Thursday)
    );
    assert_eq!(form.time(Clock::AppointmentStart).text(), "14:00");
    assert_eq!(form.time(Clock::AppointmentEnd).text(), "16:00");

    let pending = form.save_appointment("");
    let mut port = Port::default();
    form.run(pending.expect("the appointment is accepted"), &mut port);
    let saved = port.saved.pop().unwrap();
    assert_eq!(saved.recurring_appointments.len(), 1);
    assert_eq!(saved.recurring_appointments[0].since, Some(TODAY));
    assert!(saved.appointment_proposals.dismissed.is_empty());
}

/// One week in two from the day just corrected, a week before the first of the next two.
#[test]
fn a_proposal_one_week_in_two_keeps_its_weeks() {
    let mut form = accounting(Vec::new(), Language::French);
    assert!(form.open_proposal(&proposal(Some(TODAY))));
    let pending = form.save_appointment("");
    let mut port = Port::default();
    form.run(pending.unwrap(), &mut port);
    let from = port.saved.pop().unwrap().recurring_appointments[0]
        .alternate_weeks_from
        .unwrap();
    assert!(RecurringAppointment::same_alternate_weeks(from, TODAY));
}

/// « Annuler » on a proposal postpones it two weeks; on a new appointment it saves nothing.
#[test]
fn cancelling_a_proposal_postpones_it() {
    use bientot_domain::configuration::DismissedProposal;

    let mut form = accounting(Vec::new(), Language::French);
    form.open_appointment(None);
    assert!(form.close_appointment().is_none());

    assert!(form.open_proposal(&proposal(None)));
    let pending = form.close_appointment();
    let mut port = Port::default();
    form.run(pending.expect("the refusal is saved"), &mut port);
    assert!(form.appointment_draft().is_none());
    assert_eq!(
        port.saved.pop().unwrap().appointment_proposals.dismissed,
        [DismissedProposal {
            weekday: Weekday::Thursday,
            start: time(14, 0, 0, 0),
            end: time(16, 0, 0, 0),
            until: Some(date(2026, 11, 5)),
        }]
    );
}

/// An appointment added over the proposed hours since opens instead of a new one.
#[test]
fn a_proposal_already_added_opens_that_appointment() {
    let mut form = accounting(vec![meeting()], Language::French);
    assert!(form.open_proposal(&proposal(None)));
    let draft = form.appointment_draft().unwrap();
    assert_eq!(draft.editing.as_deref(), Some("thursday-1400"));
    assert_eq!(draft.proposed, None);
}

/// Invalid changes on another page ask first: the proposal waits for the answer.
#[test]
fn a_proposal_waits_while_the_page_has_unsaved_changes() {
    use super::super::Field;
    use bientot_core::settings::EditGroup;

    let mut form = accounting(Vec::new(), Language::French);
    assert!(form.go_to(Page::Schedule));
    form.set_text(Field::Target, "25:00");
    form.commit(EditGroup::Targets);
    assert!(!form.open_proposal(&proposal(None)));
    assert!(form.dialog().is_some());
    assert!(form.appointment_draft().is_none());
    assert!(
        form.dismiss_proposal(proposal(None).dismissal(None))
            .is_none()
    );
}

/// « À votre retour » chooses between asking and counting automatically, which reads so in
/// the row and keeps the day the rule counts from.
#[test]
fn counting_automatically_is_chosen_in_the_editor_and_reads_so_in_the_row() {
    let mut form = accounting(vec![meeting()], Language::French);
    form.open_appointment(Some("thursday-1400"));
    let editor = form.appointments().unwrap().editor.unwrap();
    assert_eq!(editor.mode_label, "À votre retour");
    assert!(editor.modes[0].chosen && !editor.modes[0].automatic);
    assert!(
        editor
            .mode_note
            .starts_with("Bientôt compte l’absence provisoirement")
    );

    form.choose_appointment_mode(true);
    let editor = form.appointments().unwrap().editor.unwrap();
    assert!(editor.modes[1].chosen);
    assert!(
        editor
            .mode_note
            .ends_with("Ce n’était pas du travail\u{a0}».")
    );
    let pending = form.save_appointment("Réunion d’équipe");
    let saved = save(&mut form, pending);
    assert!(saved[0].automatic);
    assert_eq!(saved[0].since, Some(date(2026, 9, 3)));
    assert_eq!(
        form.appointments().unwrap().rows[0].description,
        "Chaque jeudi · 14:00–16:00 · Compté automatiquement"
    );
}

#[test]
fn the_trust_offer_counts_an_appointment_automatically() {
    let mut form = accounting(vec![meeting()], Language::French);
    let pending = form.count_automatically("thursday-1400");
    let saved = save(&mut form, pending);
    assert!(saved[0].automatic);
    assert_eq!(saved[0].since, Some(date(2026, 9, 3)));
    assert!(form.count_automatically("unknown").is_none());
}

/// The editor open on the appointment the flyout counts automatically follows, so saving it
/// keeps the change.
#[test]
fn the_open_editor_keeps_the_flyout_counting_automatically() {
    let mut form = accounting(vec![meeting()], Language::French);
    form.open_appointment(Some("thursday-1400"));
    let pending = form.count_automatically("thursday-1400");
    assert!(save(&mut form, pending)[0].automatic);
    assert!(form.appointments().unwrap().editor.unwrap().modes[1].chosen);
    let pending = form.save_appointment("Comité");
    let saved = save(&mut form, pending);
    assert_eq!(saved[0].name, "Comité");
    assert!(saved[0].automatic);
}
