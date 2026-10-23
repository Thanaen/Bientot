use super::*;

fn situation(host: Host, tracking: TrackingState) -> Situation {
    Situation {
        host,
        tracking: Tracking::new(tracking, true),
        can_correct_time: true,
    }
}

fn labels(palette: &Palette) -> Vec<String> {
    palette.rows().into_iter().map(|row| row.label).collect()
}

/// Each row's matched part in brackets, « [Par]amètres : ouvrir ».
fn hits(palette: &Palette) -> Vec<String> {
    palette
        .rows()
        .into_iter()
        .map(|row| {
            let hit = row.hit.expect("a query marks what it matched");
            format!(
                "{}[{}]{}",
                &row.label[..hit.start],
                &row.label[hit.clone()],
                &row.label[hit.end..]
            )
        })
        .collect()
}

#[test]
fn the_flyout_lists_its_commands_under_one_heading() {
    let palette = Palette::new(
        situation(Host::Flyout, TrackingState::Running),
        Language::French,
    );
    assert_eq!(palette.heading(), Some("Commandes"));
    assert_eq!(
        labels(&palette),
        [
            "Suivi\u{a0}: mettre en pause",
            "Temps\u{a0}: corriger…",
            "Journée\u{a0}: terminer",
            "Semaine\u{a0}: ouvrir",
            "Paramètres\u{a0}: ouvrir",
            "Paramètres\u{a0}: ouvrir settings.json",
            "WorkTime\u{a0}: actualiser",
            "WorkTime\u{a0}: exporter le diagnostic…",
            "WorkTime\u{a0}: quitter",
        ]
    );
}

#[test]
fn the_week_adds_its_own_commands_after_the_correction_and_drops_its_opening() {
    let palette = Palette::new(
        situation(Host::Week, TrackingState::Paused),
        Language::English,
    );
    assert_eq!(palette.heading(), Some("Commands"));
    assert_eq!(
        labels(&palette),
        [
            "Tracking: resume",
            "Time: adjust…",
            "Week: previous day",
            "Week: next day",
            "Week: settle next slot",
            "Week: undo last action",
            "Day: finish",
            "Settings: open",
            "Settings: open settings.json",
            "WorkTime: refresh",
            "WorkTime: export diagnostics…",
            "WorkTime: quit",
        ]
    );
}

#[test]
fn a_finished_day_with_nothing_to_correct_hides_what_cannot_run() {
    let palette = Palette::new(
        Situation {
            host: Host::Flyout,
            tracking: Tracking::new(TrackingState::Finished, true),
            can_correct_time: false,
        },
        Language::English,
    );
    assert_eq!(
        labels(&palette)[..3],
        ["Tracking: resume", "Week: open", "Settings: open"]
    );
}

/// `MainWindow.UpdateCommands` of the .NET app: no pause, resume nor finish on a day off.
#[test]
fn a_day_off_hides_the_tracking_commands() {
    for tracking in [TrackingState::Running, TrackingState::Paused] {
        let palette = Palette::new(
            Situation {
                host: Host::Flyout,
                tracking: Tracking::new(tracking, false),
                can_correct_time: false,
            },
            Language::English,
        );
        assert_eq!(
            labels(&palette),
            [
                "Week: open",
                "Settings: open",
                "Settings: open settings.json",
                "WorkTime: refresh",
                "WorkTime: export diagnostics…",
                "WorkTime: quit",
            ],
            "{tracking:?}"
        );
    }
}

#[test]
fn rows_read_with_their_shortcut_when_they_have_one() {
    let names = |host, language| -> Vec<SharedString> {
        Palette::new(situation(host, TrackingState::Running), language)
            .rows()
            .into_iter()
            .map(|row| row.name)
            .collect()
    };
    let french = names(Host::Week, Language::French);
    assert_eq!(french[0], "Suivi\u{a0}: mettre en pause, Ctrl+Alt+P");
    assert_eq!(french[2], "Semaine\u{a0}: jour précédent, ↑");
    assert_eq!(french[7], "Paramètres\u{a0}: ouvrir, Ctrl+Virgule");
    assert_eq!(french[8], "Paramètres\u{a0}: ouvrir settings.json");
    let english = names(Host::Flyout, Language::English);
    assert_eq!(english[2], "Day: finish, Ctrl+Shift+F");
    assert_eq!(english[4], "Settings: open, Ctrl+Comma");
    assert_eq!(english[7], "WorkTime: export diagnostics…");
}

#[test]
fn a_query_ranks_the_matches_at_the_start_of_a_domain_or_verb_first() {
    let mut palette = Palette::new(
        situation(Host::Flyout, TrackingState::Running),
        Language::French,
    );
    palette.set_query("OU");
    assert_eq!(palette.heading(), None);
    assert_eq!(
        hits(&palette),
        [
            "Semaine\u{a0}: [ou]vrir",
            "Paramètres\u{a0}: [ou]vrir",
            "Paramètres\u{a0}: [ou]vrir settings.json",
            "J[ou]rnée\u{a0}: terminer",
        ]
    );
    palette.set_query("par");
    assert_eq!(
        hits(&palette),
        [
            "[Par]amètres\u{a0}: ouvrir",
            "[Par]amètres\u{a0}: ouvrir settings.json"
        ]
    );
}

#[test]
fn a_query_ignores_accents_and_the_no_break_space() {
    let mut palette = Palette::new(
        situation(Host::Week, TrackingState::Running),
        Language::French,
    );
    palette.set_query("precedent");
    assert_eq!(hits(&palette), ["Semaine\u{a0}: jour [précédent]"]);
    palette.set_query("journee : ter");
    assert_eq!(hits(&palette), ["[Journée\u{a0}: ter]miner"]);
}

#[test]
fn the_selection_moves_within_the_rows_and_a_new_query_resets_it() {
    let mut palette = Palette::new(
        situation(Host::Flyout, TrackingState::Running),
        Language::English,
    );
    palette.set_query("set");
    assert_eq!(palette.chosen(), Some(Command::OpenSettings));
    palette.select_next();
    palette.select_next();
    assert_eq!(palette.selected(), 1);
    assert_eq!(palette.chosen(), Some(Command::OpenSettingsJson));
    palette.select_previous();
    palette.select_previous();
    assert_eq!(palette.chosen(), Some(Command::OpenSettings));
    palette.select_next();
    palette.set_query("quit");
    assert_eq!(palette.chosen(), Some(Command::Quit));
    palette.set_query("zzz");
    assert_eq!(labels(&palette), Vec::<String>::new());
    assert_eq!(palette.chosen(), None);
}

#[test]
fn the_pause_row_follows_the_day() {
    let mut palette = Palette::new(
        situation(Host::Flyout, TrackingState::Running),
        Language::English,
    );
    palette.set_query("track");
    assert_eq!(labels(&palette), ["Tracking: pause"]);
    palette.set_situation(situation(Host::Flyout, TrackingState::Paused));
    assert_eq!(labels(&palette), ["Tracking: resume"]);
}
