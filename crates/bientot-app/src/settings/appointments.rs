//! « Rendez-vous réguliers » of « Comptabilisation » (issue #321): the recurring appointments,
//! each a row with its switch, and the editor that adds one or changes one. Every change saves
//! the whole list at once.

use bientot_core::settings::{EditGroup, Pending};
use bientot_domain::configuration::{AppointmentProposal, DismissedProposal, RecurringAppointment};
use jiff::ToSpan as _;
use jiff::civil::{Date, Weekday};

use super::{CalloutView, Page, RowView, Setting, SettingsForm, time_field};
use crate::strings::{Key, Language};
use crate::tray_model::appointment_name;

/// The weekdays an appointment can fall on.
pub const WEEKDAYS: [Weekday; 5] = [
    Weekday::Monday,
    Weekday::Tuesday,
    Weekday::Wednesday,
    Weekday::Thursday,
    Weekday::Friday,
];

/// The hours a new appointment starts with.
pub const NEW_START: jiff::civil::Time = jiff::civil::time(14, 0, 0, 0);
pub const NEW_END: jiff::civil::Time = jiff::civil::time(16, 0, 0, 0);

/// What the editor holds besides its hours, which are the drafts of
/// [`Clock::AppointmentStart`] and [`Clock::AppointmentEnd`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppointmentDraft {
    /// The appointment changed, `None` for a new one.
    pub editing: Option<String>,
    pub name: String,
    pub weekday: Weekday,
    /// « Créer » or « Enregistrer » was pressed on hours that do not follow each other.
    pub invalid: bool,
    /// « Supprimer » was pressed once and now reads « Confirmer la suppression ».
    pub confirming_delete: bool,
    /// « Une semaine sur deux ».
    pub alternate: bool,
    /// One week in two, from the second of the next two days it falls on rather than the
    /// first. Kept while « Chaque semaine » is chosen, so that switching back restores it.
    pub later_week: bool,
    /// The proposal it opened on, which « Annuler » postpones.
    pub proposed: Option<AppointmentProposal>,
    /// « Compter automatiquement » rather than « Me demander ».
    pub automatic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppointmentsView {
    pub heading: &'static str,
    pub description: &'static str,
    pub rows: Vec<AppointmentRow>,
    /// Instead of the rows when there are none.
    pub empty: Option<&'static str>,
    /// « Ajouter un rendez-vous », until the list holds as many as the settings allow.
    pub add: Option<&'static str>,
    pub editor: Option<EditorView>,
    pub callout: Option<CalloutView>,
    /// « Proposer des rendez-vous d’après vos corrections », below, with the callout of its
    /// save.
    pub proposals: RowView,
    pub proposals_callout: Option<CalloutView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppointmentRow {
    pub id: String,
    /// The name, or « Rendez-vous du jeudi ».
    pub title: String,
    /// « Chaque jeudi · 14:00–16:00 ».
    pub description: String,
    pub enabled: bool,
    /// The edit button, named after the appointment for screen readers; `None` on the row the
    /// editor is open on.
    pub edit: Option<&'static str>,
    pub edit_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorView {
    pub title: &'static str,
    pub name_label: &'static str,
    /// The name the appointment takes when left empty.
    pub name_placeholder: String,
    pub day_label: &'static str,
    pub days: Vec<DayOption>,
    pub repeat_label: &'static str,
    /// « Chaque semaine », then « Une semaine sur deux ».
    pub repeats: [RepeatOption; 2],
    /// One week in two: which of the next two days it falls on comes first.
    pub next: Option<NextView>,
    pub hours_label: &'static str,
    pub mode_label: &'static str,
    /// « Me demander », then « Compter automatiquement ».
    pub modes: [ModeOption; 2],
    /// What the mode chosen does.
    pub mode_note: &'static str,
    pub hours_error: Option<&'static str>,
    pub note: &'static str,
    pub save: &'static str,
    pub cancel: &'static str,
    /// « Supprimer », or « Confirmer la suppression » once pressed, when an appointment is
    /// changed.
    pub delete: Option<&'static str>,
}

/// A weekday of the editor, « Jeu. » for « jeudi ».
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DayOption {
    pub weekday: Weekday,
    pub short: &'static str,
    pub full: &'static str,
    pub chosen: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepeatOption {
    pub alternate: bool,
    pub label: &'static str,
    pub chosen: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeOption {
    pub automatic: bool,
    pub label: &'static str,
    pub chosen: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NextView {
    pub label: &'static str,
    pub weeks: [WeekOption; 2],
}

/// « 29 octobre », named « jeudi 29 octobre » for screen readers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeekOption {
    pub later: bool,
    pub label: String,
    pub full: String,
    pub chosen: bool,
}

impl SettingsForm {
    /// « Rendez-vous réguliers », under the groups of « Comptabilisation ».
    pub fn appointments(&self) -> Option<AppointmentsView> {
        if self.page != Page::Accounting {
            return None;
        }
        let language = self.language;
        let rows: Vec<_> = self
            .drafts
            .appointments
            .iter()
            .map(|appointment| {
                let title = appointment_name(&appointment.name, appointment.weekday, language);
                AppointmentRow {
                    id: appointment.id.clone(),
                    description: describe(appointment, language),
                    enabled: appointment.enabled,
                    edit: (self
                        .appointment
                        .as_ref()
                        .and_then(|draft| draft.editing.as_ref())
                        != Some(&appointment.id))
                    .then(|| language.text(Key::SettingsAppointmentEdit)),
                    edit_name: language.format(Key::SettingsAppointmentEditNameFormat, &[&title]),
                    title,
                }
            })
            .collect();
        Some(AppointmentsView {
            heading: language.text(Key::SettingsAppointmentsGroup),
            description: language.text(Key::SettingsAppointmentsDescription),
            empty: rows
                .is_empty()
                .then(|| language.text(Key::SettingsAppointmentsEmpty)),
            add: (rows.len() < RecurringAppointment::MAXIMUM_COUNT)
                .then(|| language.text(Key::SettingsAppointmentsAdd)),
            rows,
            editor: self.appointment.as_ref().map(|draft| self.editor(draft)),
            callout: self.callout(EditGroup::Appointments),
            proposals: self.row(Setting::AppointmentProposals),
            proposals_callout: self.callout(EditGroup::AppointmentProposals),
        })
    }

    fn editor(&self, draft: &AppointmentDraft) -> EditorView {
        let language = self.language;
        let editing = draft.editing.is_some();
        EditorView {
            title: language.text(if editing {
                Key::SettingsAppointmentEditTitle
            } else {
                Key::SettingsAppointmentsAdd
            }),
            name_label: language.text(Key::SettingsAppointmentNameLabel),
            name_placeholder: appointment_name("", draft.weekday, language),
            day_label: language.text(Key::SettingsAppointmentDayLabel),
            days: WEEKDAYS
                .into_iter()
                .map(|weekday| DayOption {
                    weekday,
                    short: language.weekday_abbreviation(weekday),
                    full: language.weekday_name(weekday),
                    chosen: weekday == draft.weekday,
                })
                .collect(),
            repeat_label: language.text(Key::SettingsAppointmentRepeatLabel),
            repeats: [
                (false, Key::SettingsAppointmentEveryWeek),
                (true, Key::SettingsAppointmentEveryOtherWeek),
            ]
            .map(|(alternate, key)| RepeatOption {
                alternate,
                label: language.text(key),
                chosen: alternate == draft.alternate,
            }),
            next: self.next_view(draft),
            hours_label: language.text(Key::SettingsAppointmentHoursLabel),
            mode_label: language.text(Key::SettingsAppointmentModeLabel),
            modes: [
                (false, Key::SettingsAppointmentModeAsk),
                (true, Key::SettingsAppointmentModeAutomatic),
            ]
            .map(|(automatic, key)| ModeOption {
                automatic,
                label: language.text(key),
                chosen: automatic == draft.automatic,
            }),
            mode_note: language.text(if draft.automatic {
                Key::SettingsAppointmentModeAutomaticNote
            } else {
                Key::SettingsAppointmentModeAskNote
            }),
            hours_error: draft
                .invalid
                .then(|| language.text(Key::SettingsAppointmentHoursError)),
            note: language.text(Key::SettingsAppointmentNote),
            save: language.text(if editing {
                Key::SettingsAppointmentSave
            } else {
                Key::SettingsAppointmentCreate
            }),
            cancel: language.text(Key::SettingsAppointmentCancel),
            delete: editing.then(|| {
                language.text(if draft.confirming_delete {
                    Key::SettingsAppointmentDeleteConfirm
                } else {
                    Key::SettingsAppointmentDelete
                })
            }),
        }
    }

    /// « Prochaine date », for an appointment one week in two.
    fn next_view(&self, draft: &AppointmentDraft) -> Option<NextView> {
        let language = self.language;
        draft.alternate.then(|| {
            let dates = next_two(self.usage_ping.today, draft.weekday);
            NextView {
                label: language.text(Key::SettingsAppointmentNextLabel),
                weeks: [false, true].map(|later| {
                    let date = [dates[usize::from(later)]];
                    WeekOption {
                        later,
                        label: language.format_dates(Key::SettingsAppointmentWeekFormat, &date),
                        full: language.format_dates(Key::SettingsAppointmentWeekFullFormat, &date),
                        chosen: later == draft.later_week,
                    }
                }),
            }
        })
    }

    /// Opens the editor on the appointment `id`, or on a new one, and returns the name it
    /// starts with; `None` while the dialog holds the page.
    pub fn open_appointment(&mut self, id: Option<&str>) -> Option<String> {
        if self.leaving.is_some() {
            return None;
        }
        let existing = id.and_then(|id| {
            self.drafts
                .appointments
                .iter()
                .find(|appointment| appointment.id == id)
        });
        let today = self.usage_ping.today;
        let (draft, start, end) = match existing {
            Some(appointment) => (
                AppointmentDraft {
                    editing: Some(appointment.id.clone()),
                    name: appointment.name.clone(),
                    weekday: appointment.weekday,
                    invalid: false,
                    confirming_delete: false,
                    alternate: appointment.alternate_weeks_from.is_some(),
                    later_week: later_week(
                        today,
                        appointment.weekday,
                        appointment.alternate_weeks_from,
                    ),
                    proposed: None,
                    automatic: appointment.automatic,
                },
                appointment.start,
                appointment.end,
            ),
            None => (
                AppointmentDraft {
                    editing: None,
                    name: String::new(),
                    weekday: Weekday::Thursday,
                    invalid: false,
                    confirming_delete: false,
                    alternate: false,
                    later_week: false,
                    proposed: None,
                    automatic: false,
                },
                NEW_START,
                NEW_END,
            ),
        };
        self.drafts.appointment_start = time_field(start);
        self.drafts.appointment_end = time_field(end);
        let name = draft.name.clone();
        self.appointment = Some(draft);
        Some(name)
    }

    /// « Compter chaque jeudi… »: « Comptabilisation » with the editor on a new appointment
    /// with the day, hours and weeks of `proposal`, or on the appointment added over its hours
    /// since; only the list once it holds as many as the settings allow. `false` while the
    /// dialog holds the page, or opens to leave the one shown.
    pub fn open_proposal(&mut self, proposal: &AppointmentProposal) -> bool {
        if !self.go_to(Page::Accounting) {
            return false;
        }
        let added = self.drafts.appointments.iter().find(|appointment| {
            appointment.weekday == proposal.weekday
                && appointment.start < proposal.end
                && proposal.start < appointment.end
        });
        if let Some(id) = added.map(|appointment| appointment.id.clone()) {
            self.open_appointment(Some(&id));
            return true;
        }
        if self.drafts.appointments.len() >= RecurringAppointment::MAXIMUM_COUNT {
            self.appointment = None;
            return true;
        }
        let today = self.usage_ping.today;
        self.drafts.appointment_start = time_field(proposal.start);
        self.drafts.appointment_end = time_field(proposal.end);
        self.appointment = Some(AppointmentDraft {
            editing: None,
            name: String::new(),
            weekday: proposal.weekday,
            invalid: false,
            confirming_delete: false,
            alternate: proposal.alternate_weeks_from.is_some(),
            later_week: later_week(today, proposal.weekday, proposal.alternate_weeks_from),
            proposed: Some(proposal.clone()),
            automatic: false,
        });
        true
    }

    pub fn appointment_draft(&self) -> Option<&AppointmentDraft> {
        self.appointment.as_ref()
    }

    /// « Annuler »: an editor opened on a proposal postpones it by
    /// [`DismissedProposal::POSTPONED_DAYS`].
    pub fn close_appointment(&mut self) -> Option<Pending> {
        let proposal = self.appointment.take()?.proposed?;
        let until = self
            .usage_ping
            .today
            .checked_add(DismissedProposal::POSTPONED_DAYS.days())
            .ok();
        self.dismiss_proposal(proposal.dismissal(until))
    }

    /// « Ne plus proposer pour les jeudis », or a proposal postponed; nothing while the dialog
    /// holds the page, see [`SettingsForm::dialog`].
    pub fn dismiss_proposal(&mut self, dismissal: DismissedProposal) -> Option<Pending> {
        if self.leaving.is_some() {
            return None;
        }
        let today = self.usage_ping.today;
        self.drafts.appointment_proposals.dismiss(dismissal, today);
        self.commit(EditGroup::AppointmentProposals)
    }

    pub fn choose_appointment_day(&mut self, weekday: Weekday) {
        if let Some(draft) = &mut self.appointment {
            draft.weekday = weekday;
        }
    }

    /// « Chaque semaine » or « Une semaine sur deux ».
    pub fn choose_appointment_repeat(&mut self, alternate: bool) {
        if let Some(draft) = &mut self.appointment {
            draft.alternate = alternate;
        }
    }

    /// « Me demander » or « Compter automatiquement ».
    pub fn choose_appointment_mode(&mut self, automatic: bool) {
        if let Some(draft) = &mut self.appointment {
            draft.automatic = automatic;
        }
    }

    /// The first of the next two days an appointment one week in two falls on, or the second.
    pub fn choose_appointment_week(&mut self, later: bool) {
        if let Some(draft) = &mut self.appointment {
            draft.later_week = later;
        }
    }

    /// « Créer le rendez-vous » or « Enregistrer » with the name typed. Changed hours or day
    /// count from today, so that the days before keep what they were accounted with.
    pub fn save_appointment(&mut self, name: &str) -> Option<Pending> {
        let draft = self.appointment.as_mut()?;
        name.clone_into(&mut draft.name);
        let (start, end) = (
            self.drafts.appointment_start.time(),
            self.drafts.appointment_end.time(),
        );
        if end <= start {
            draft.invalid = true;
            return None;
        }
        let name: String = draft
            .name
            .trim()
            .chars()
            .take(RecurringAppointment::MAXIMUM_NAME_LENGTH)
            .collect();
        let today = self.usage_ping.today;
        let alternate_weeks_from = draft
            .alternate
            .then(|| next_two(today, draft.weekday)[usize::from(draft.later_week)]);
        let appointments = &mut self.drafts.appointments;
        match &draft.editing {
            Some(id) => {
                let Some(appointment) = appointments
                    .iter_mut()
                    .find(|appointment| appointment.id == *id)
                else {
                    self.appointment = None;
                    return None;
                };
                let moved = (appointment.weekday, appointment.start, appointment.end)
                    != (draft.weekday, start, end)
                    || !RecurringAppointment::same_repeat(
                        appointment.alternate_weeks_from,
                        alternate_weeks_from,
                    );
                appointment.name = name;
                appointment.weekday = draft.weekday;
                appointment.start = start;
                appointment.end = end;
                appointment.automatic = draft.automatic;
                if moved {
                    appointment.since = Some(today);
                    appointment.alternate_weeks_from = alternate_weeks_from;
                }
            }
            None if appointments.len() >= RecurringAppointment::MAXIMUM_COUNT => {
                self.appointment = None;
                return None;
            }
            None => {
                let id = new_id(appointments, draft.weekday, start, today);
                appointments.push(RecurringAppointment {
                    id,
                    name,
                    weekday: draft.weekday,
                    start,
                    end,
                    enabled: true,
                    since: Some(today),
                    alternate_weeks_from,
                    automatic: draft.automatic,
                });
            }
        }
        self.appointment = None;
        self.commit(EditGroup::Appointments)
    }

    /// « Supprimer » in the editor, which deletes on its second press.
    pub fn delete_appointment(&mut self) -> Option<Pending> {
        let draft = self.appointment.as_mut()?;
        let id = draft.editing.clone()?;
        if !draft.confirming_delete {
            draft.confirming_delete = true;
            return None;
        }
        self.drafts
            .appointments
            .retain(|appointment| appointment.id != id);
        self.appointment = None;
        self.commit(EditGroup::Appointments)
    }

    /// « Compter automatiquement à l’avenir » of the flyout on the appointment `id`.
    pub fn count_automatically(&mut self, id: &str) -> Option<Pending> {
        if self.leaving.is_some() {
            return None;
        }
        let appointment = self
            .drafts
            .appointments
            .iter_mut()
            .find(|appointment| appointment.id == id)?;
        appointment.automatic = true;
        // The editor open on it saves its own choice: it follows, rather than undoing it.
        if let Some(draft) = self
            .appointment
            .as_mut()
            .filter(|draft| draft.editing.as_deref() == Some(id))
        {
            draft.automatic = true;
        }
        self.commit(EditGroup::Appointments)
    }

    /// The switch of the appointment `id`.
    pub fn toggle_appointment(&mut self, id: &str) -> Option<Pending> {
        if self.leaving.is_some() {
            return None;
        }
        let appointment = self
            .drafts
            .appointments
            .iter_mut()
            .find(|appointment| appointment.id == id)?;
        appointment.enabled = !appointment.enabled;
        self.commit(EditGroup::Appointments)
    }
}

/// Whether an appointment one week in two from `from` falls on the second of the next two days
/// on `weekday` rather than the first.
fn later_week(today: Date, weekday: Weekday, from: Option<Date>) -> bool {
    from.is_some_and(|from| {
        !RecurringAppointment::same_alternate_weeks(next_two(today, weekday)[0], from)
    })
}

/// The next two days on `weekday` after today, whose appointment may be over already.
fn next_two(today: Date, weekday: Weekday) -> [Date; 2] {
    let tomorrow = today
        .tomorrow()
        .expect("today is not the last day jiff knows");
    let ahead = i64::from(weekday.since(tomorrow.weekday()));
    let first = tomorrow + jiff::Span::new().days(ahead);
    [first, first + jiff::Span::new().days(7)]
}

/// « Chaque jeudi · 14:00–16:00 », or « Un jeudi sur deux · … », in the 24-hour times of the
/// editor's fields, then « · Compté automatiquement » for an appointment counted without asking.
fn describe(appointment: &RecurringAppointment, language: Language) -> String {
    let clock = |time| time_field(time).text();
    let hours = language.format(
        if appointment.alternate_weeks_from.is_some() {
            Key::SettingsAppointmentAlternateRowFormat
        } else {
            Key::SettingsAppointmentRowFormat
        },
        &[
            &language.weekday_name(appointment.weekday),
            &clock(appointment.start),
            &clock(appointment.end),
        ],
    );
    if appointment.automatic {
        language.format(Key::SettingsAppointmentAutomaticRowFormat, &[&hours])
    } else {
        hours
    }
}

/// `thursday-1400-20261022`: the day and hours it starts with and the day it was created, so
/// that an appointment deleted then created again on another day does not take the reviews of
/// the first; `-2` and on while taken.
fn new_id(
    appointments: &[RecurringAppointment],
    weekday: Weekday,
    start: jiff::civil::Time,
    created: jiff::civil::Date,
) -> String {
    let base = format!(
        "{}-{:02}{:02}-{}",
        format!("{weekday:?}").to_ascii_lowercase(),
        start.hour(),
        start.minute(),
        created.strftime("%Y%m%d"),
    );
    let taken = |id: &str| appointments.iter().any(|appointment| appointment.id == id);
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|suffix| format!("{base}-{suffix}"))
        .find(|id| !taken(id))
        .expect("an unused suffix exists")
}

#[cfg(test)]
mod tests;
