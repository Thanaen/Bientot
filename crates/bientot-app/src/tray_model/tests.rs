use jiff::civil::date;

use super::*;

/// Each entry as the menu shows it: a greyed item in brackets, a separator as a rule, a
/// submenu as its label and an arrow, then its items indented.
fn shown(state: &MenuState, language: Language) -> Vec<String> {
    lines(menu(state, language), "")
}

fn lines(entries: Vec<MenuEntry>, indent: &str) -> Vec<String> {
    entries
        .into_iter()
        .flat_map(|entry| match entry {
            MenuEntry::Item {
                label,
                enabled: true,
                ..
            } => vec![format!("{indent}{label}")],
            MenuEntry::Item {
                label,
                enabled: false,
                ..
            } => vec![format!("{indent}[{label}]")],
            MenuEntry::Submenu { label, items } => {
                let mut shown = vec![format!("{indent}{label} >")];
                shown.extend(lines(items, &format!("{indent}  ")));
                shown
            }
            MenuEntry::Separator => vec![format!("{indent}---")],
        })
        .collect()
}

fn state(tracking: TrackingState) -> MenuState {
    MenuState {
        tracking: Tracking::new(tracking, true),
        reviews: Vec::new(),
        has_latest_adjustment: false,
    }
}

/// The lock `id` from `start` to `end`, hours and minutes of 8 October 2026 in UTC.
fn review(
    id: &str,
    (start_hour, start_minute): (i8, i8),
    (end_hour, end_minute): (i8, i8),
) -> ClosedReview {
    let at = |hour, minute| date(2026, 10, 8).at(hour, minute, 0, 0);
    let instant = |hour, minute| at(hour, minute).in_tz("UTC").unwrap().timestamp();
    ClosedReview {
        workday: LocalWorkday::new(date(2026, 10, 8), "UTC"),
        id: LongLockReviewId::new(id).unwrap(),
        locked_episode: UtcInterval::new(
            instant(start_hour, start_minute),
            instant(end_hour, end_minute),
        )
        .unwrap(),
        local_start: at(start_hour, start_minute).time(),
        local_end: at(end_hour, end_minute).time(),
        finish_at: at(start_hour, start_minute).time(),
        duration: instant(end_hour, end_minute).duration_since(instant(start_hour, start_minute)),
        appointment: None,
    }
}

/// A day with a lock in the afternoon listed before one in the morning.
fn two_locks() -> MenuState {
    MenuState {
        tracking: Tracking::new(TrackingState::Running, true),
        reviews: vec![
            review("session-lock:57", (14, 5), (16, 54)),
            review("session-lock:42", (9, 30), (11, 0)),
        ],
        has_latest_adjustment: true,
    }
}

#[test]
fn a_running_day_offers_the_pause() {
    assert_eq!(
        shown(&state(TrackingState::Running), Language::French),
        [
            "Ouvrir Bientôt",
            "---",
            "Mettre le suivi en pause",
            "Terminer la journée",
            "Corriger le temps…",
            "Exporter le diagnostic…",
            "---",
            "Quitter Bientôt",
        ]
    );
}

#[test]
fn a_paused_or_finished_day_offers_to_resume_and_a_finished_one_cannot_finish_again() {
    assert_eq!(
        shown(&state(TrackingState::Paused), Language::English)[2..4],
        ["Resume tracking", "Finish day"]
    );
    assert_eq!(
        shown(&state(TrackingState::Finished), Language::English)[2..4],
        ["Resume tracking", "[Finish day]"]
    );
}

/// Unlike the .NET app's tray, as its flyout: no pause, resume nor finish on a day off. Nor
/// « Corriger le temps… », as `CanAddTime` of the .NET app says.
#[test]
fn a_day_off_greys_the_pause_toggle_finish_and_the_correction() {
    let day_off = |tracking| MenuState {
        tracking: Tracking::new(tracking, false),
        ..state(tracking)
    };
    assert_eq!(
        shown(&day_off(TrackingState::Running), Language::English)[2..5],
        ["[Pause tracking]", "[Finish day]", "[Adjust time…]"]
    );
    assert_eq!(
        shown(&day_off(TrackingState::Paused), Language::French)[2..5],
        [
            "[Reprendre le suivi]",
            "[Terminer la journée]",
            "[Corriger le temps…]"
        ]
    );
}

#[test]
fn each_closed_lock_gets_a_submenu_named_by_its_time_range_in_chronological_order() {
    assert_eq!(
        shown(&two_locks(), Language::French),
        [
            "Ouvrir Bientôt",
            "---",
            "Mettre le suivi en pause",
            "Terminer la journée",
            "Corriger le temps…",
            "---",
            "Verrouillage à vérifier · 09:30–11:00 >",
            "  Oui, je travaillais",
            "  Ce n’était pas du travail",
            "  Terminer la journée à\u{a0}09:30",
            "Verrouillage à vérifier · 14:05–16:54 >",
            "  Oui, je travaillais",
            "  Ce n’était pas du travail",
            "  Terminer la journée à\u{a0}14:05",
            "Modifier le dernier ajustement (suppression indisponible)…",
            "---",
            "Exporter le diagnostic…",
            "---",
            "Quitter Bientôt",
        ]
    );
}

/// `EditTimeAsync` of the .NET app does nothing on a day off by the calendar, though the
/// published day still has its adjustment: the item stays, greyed, and a click runs nothing.
#[test]
fn the_latest_adjustment_is_edited_only_on_a_working_day() {
    let latest = |working_day| MenuState {
        tracking: Tracking::new(TrackingState::Running, working_day),
        reviews: Vec::new(),
        has_latest_adjustment: true,
    };
    let working = menu(&latest(true), Language::English);
    assert_eq!(
        lines(working.clone(), "")[5..8],
        ["---", "Edit last adjustment (removal unavailable)…", "---"]
    );
    assert_eq!(
        chosen(&working, 1, &item_id(1, &[6])),
        Some(MenuCommand::EditLatestAdjustment)
    );
    let day_off = menu(&latest(false), Language::English);
    assert_eq!(
        lines(day_off.clone(), "")[6],
        "[Edit last adjustment (removal unavailable)…]"
    );
    assert_eq!(chosen(&day_off, 1, &item_id(1, &[6])), None);
}

#[test]
fn the_review_submenus_read_in_english_with_the_am_pm_kept_together() {
    assert_eq!(
        shown(&two_locks(), Language::English)[6..14],
        [
            "Lock to review · 9:30\u{a0}AM–11:00\u{a0}AM >",
            "  Yes, I was working",
            "  It wasn’t work",
            "  Finish the day at\u{a0}9:30\u{a0}AM",
            "Lock to review · 2:05\u{a0}PM–4:54\u{a0}PM >",
            "  Yes, I was working",
            "  It wasn’t work",
            "  Finish the day at\u{a0}2:05\u{a0}PM",
        ]
    );
}

#[test]
fn a_click_on_a_submenu_item_runs_its_review_command_while_its_menu_is_shown() {
    let entries = menu(&two_locks(), Language::English);
    let command = |generation, path: &[usize]| {
        chosen(&entries, 7, &item_id(generation, path)).map(|command| match command {
            MenuCommand::Review(ReviewCommand { review, action }) => {
                format!("{} {action:?}", review.id)
            }
            other => format!("{other:?}"),
        })
    };

    assert_eq!(
        command(7, &[7, 2]).as_deref(),
        Some("session-lock:57 FinishAtLockStart")
    );
    assert_eq!(
        command(7, &[6, 0]).as_deref(),
        Some("session-lock:42 ConfirmWork")
    );
    assert_eq!(
        command(7, &[6, 1]).as_deref(),
        Some("session-lock:42 Exclude")
    );
    assert_eq!(command(7, &[0]).as_deref(), Some("Open"));
    assert_eq!(command(7, &[8]).as_deref(), Some("EditLatestAdjustment"));
    // A menu since replaced, the submenu itself, a separator, nothing there.
    assert_eq!(command(6, &[7, 2]), None);
    assert_eq!(command(7, &[7]), None);
    assert_eq!(command(7, &[5]), None);
    assert_eq!(command(7, &[7, 3]), None);
    assert_eq!(command(7, &[0, 0]), None);
    assert_eq!(chosen(&entries, 7, "7:x"), None);
}

#[test]
fn the_review_command_carries_the_lock_to_resolve() {
    let entries = menu(&two_locks(), Language::French);

    assert_eq!(
        chosen(&entries, 0, "0:6.1"),
        Some(MenuCommand::Review(ReviewCommand {
            review: review("session-lock:42", (9, 30), (11, 0)),
            action: ReviewAction::Exclude,
        }))
    );
}

#[test]
fn an_appointment_review_is_named_after_it_and_offers_no_finish() {
    let state = MenuState {
        tracking: Tracking::new(TrackingState::Running, true),
        reviews: vec![ClosedReview {
            appointment: Some(String::new()),
            ..review(
                "recurring-appointment:meeting:2026-10-08",
                (14, 2),
                (15, 47),
            )
        }],
        has_latest_adjustment: false,
    };

    let menu = menu(&state, Language::French);

    let submenu = menu
        .iter()
        .find_map(|entry| match entry {
            MenuEntry::Submenu { label, items } => Some((label.clone(), items.len())),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        submenu,
        (
            "Rendez-vous du jeudi à vérifier · 14:02–15:47".to_owned(),
            2
        )
    );
    assert_eq!(
        appointment_name(" Réunion d’équipe ", Weekday::Thursday, Language::English),
        "Réunion d’équipe"
    );
}
