use std::io;
use std::sync::Mutex;

use jiff::civil::Date;
use worktime_domain::usage_ping::{Build, Channel};

use super::*;
use crate::adjust::TimePart;
use crate::settings::RowTone;

const TODAY: Date = jiff::civil::date(2026, 10, 13);

fn onboarding(forced: Option<Language>) -> Onboarding {
    Onboarding::new(
        WorkTimeSettings::default(),
        UsagePingFacts {
            build: Build::new("1.0.0", Channel::Store).unwrap(),
            released: None,
            today: TODAY,
            last_sent: None,
        },
        forced,
    )
}

/// On the last step, the registration read as `state`.
fn on_connections(state: Option<StartupRegistrationState>) -> Onboarding {
    let mut onboarding = onboarding(Some(Language::French));
    onboarding.advance();
    onboarding.advance();
    if let Some(state) = state {
        onboarding.show_startup(state);
    }
    onboarding
}

fn rows(onboarding: &Onboarding) -> Vec<RowView> {
    match onboarding.view().body {
        Body::Rows { rows, .. } => rows,
        body => panic!("a page of rows: {body:?}"),
    }
}

fn row(onboarding: &Onboarding, setting: Setting) -> RowView {
    rows(onboarding)
        .into_iter()
        .find(|row| row.setting == setting)
        .expect("the page shows the setting")
}

/// The switch of `setting`: whether it is on, and whether it can be changed.
fn switch(onboarding: &Onboarding, setting: Setting) -> (bool, bool) {
    match row(onboarding, setting).control {
        Control::Switch { on, enabled, .. } => (on, enabled),
        control => panic!("a switch: {control:?}"),
    }
}

/// The error under each row of the schedule.
fn schedule_errors(onboarding: &Onboarding) -> Vec<Option<&'static str>> {
    rows(onboarding).iter().map(|row| row.error).collect()
}

#[test]
fn shows_on_a_first_run_or_when_asked() {
    let cases = [
        (false, false, false),
        (false, true, false),
        (true, false, false),
        (true, true, false),
        (false, false, true),
        (true, true, true),
    ];
    assert_eq!(
        cases.map(|(saved, demo, requested)| should_show(saved, demo, requested)),
        [true, false, false, false, true, true]
    );
}

#[test]
fn the_language_step_offers_windows_french_and_english() {
    let onboarding = onboarding(None);
    let page = onboarding.view();

    assert_eq!(page.counter, "Étape 1 sur 3");
    assert_eq!(page.heading, "Choisissez votre langue");
    assert_eq!(
        page.intro,
        "WorkTime suit la langue de Windows. Vous pourrez la changer dans Paramètres."
    );
    assert_eq!(
        page.body,
        Body::Language {
            name: "Langue",
            options: vec![
                LanguageOption {
                    choice: LanguageChoice::System,
                    label: "Langue de Windows",
                    selected: true,
                },
                LanguageOption {
                    choice: LanguageChoice::French,
                    label: "Français",
                    selected: false,
                },
                LanguageOption {
                    choice: LanguageChoice::English,
                    label: "English",
                    selected: false,
                },
            ],
        }
    );
    assert_eq!((page.previous, page.next), (None, "Suivant"));
}

#[test]
fn a_chosen_language_shows_at_once_unless_the_launch_fixed_it() {
    let mut onboarding = onboarding(None);
    onboarding.choose_language(LanguageChoice::English);

    let page = onboarding.view();
    assert_eq!(
        (page.counter.as_str(), page.heading, page.next),
        ("Step 1 of 3", "Choose your language", "Next")
    );
    let Body::Language { options, .. } = page.body else {
        panic!("the language step");
    };
    assert_eq!(
        options
            .iter()
            .map(|option| (option.label, option.selected))
            .collect::<Vec<_>>(),
        [
            ("Windows language", false),
            ("Français", false),
            ("English", true)
        ]
    );

    let mut forced = self::onboarding(Some(Language::French));
    forced.choose_language(LanguageChoice::English);
    assert_eq!(forced.view().heading, "Choisissez votre langue");
}

#[test]
fn the_schedule_step_shows_the_settings_cards() {
    let mut onboarding = onboarding(Some(Language::English));
    onboarding.advance();
    let page = onboarding.view();

    assert_eq!(page.counter, "Step 2 of 3");
    assert_eq!(page.heading, "Your typical day");
    assert_eq!(
        page.intro,
        "WorkTime works out when you can leave from these values."
    );
    assert_eq!((page.previous, page.next), (Some("Previous"), "Next"));
    assert_eq!(
        rows(&onboarding)
            .iter()
            .map(|row| (row.title, row.description))
            .collect::<Vec<_>>(),
        [
            (
                "Daily target",
                "Hours and minutes per standard workday, for example 7:24."
            ),
            ("Lunch window", "Start and end time."),
            (
                "Preferred latest departure",
                "WorkTime warns you when your projected departure gets close to or passes this \
                 time."
            ),
        ]
    );
    assert_eq!(onboarding.target(), "7:00");
    assert_eq!(onboarding.time(Clock::LunchEnd).text(), "14:00");
}

#[test]
fn an_invalid_schedule_keeps_the_step_and_names_the_row() {
    let mut onboarding = onboarding(Some(Language::French));
    onboarding.advance();

    onboarding.set_target("7h");
    assert_eq!(onboarding.advance(), None);
    assert_eq!(onboarding.step(), Step::Schedule);
    assert_eq!(
        schedule_errors(&onboarding),
        [Some("Utilisez le format H:mm."), None, None]
    );
    assert_eq!(
        row(&onboarding, Setting::Target).control,
        Control::Text {
            field: Field::Target,
            invalid: true,
            enabled: true,
        }
    );

    onboarding.set_target("25:00");
    assert!(!onboarding.commit_schedule());
    assert_eq!(
        schedule_errors(&onboarding)[0],
        Some("Saisissez une durée comprise entre 0:01 et 24:00.")
    );

    onboarding.set_target("7:24");
    onboarding
        .time_mut(Clock::LunchEnd)
        .step(TimePart::Hours, -3);
    assert!(!onboarding.commit_schedule());
    assert_eq!(
        schedule_errors(&onboarding),
        [
            None,
            Some("La fin du déjeuner doit être postérieure à son début."),
            None
        ]
    );

    onboarding
        .time_mut(Clock::LunchEnd)
        .step(TimePart::Hours, 3);
    *onboarding.time_mut(Clock::Departure) = TimeField::new(13, 0);
    onboarding.advance();
    assert_eq!(onboarding.step(), Step::Schedule);
    assert_eq!(
        schedule_errors(&onboarding),
        [
            None,
            None,
            Some("L’heure limite de départ doit être après le déjeuner.")
        ]
    );

    *onboarding.time_mut(Clock::Departure) = TimeField::new(17, 45);
    assert_eq!(onboarding.advance(), None);
    assert_eq!(onboarding.step(), Step::Connections);
}

#[test]
fn the_schedule_saves_its_values_and_a_minimum_no_longer_than_the_target() {
    let mut onboarding = onboarding(Some(Language::French));
    onboarding.advance();
    onboarding.set_target(" 5:30 ");
    onboarding
        .time_mut(Clock::LunchStart)
        .step(TimePart::Minutes, 15);
    *onboarding.time_mut(Clock::Departure) = TimeField::new(17, 45);
    onboarding.advance();
    onboarding.back();
    assert_eq!(onboarding.target(), "5:30");

    onboarding.set_target("8:00");
    onboarding.advance();
    let saved = onboarding.advance().expect("« Commencer » saves");

    assert_eq!(
        saved.nominal_daily_target,
        SignedDuration::from_mins(8 * 60)
    );
    assert_eq!(
        saved.minimum_effective_target,
        WorkTimeSettings::default().minimum_effective_target
    );
    assert_eq!(saved.lunch_start, jiff::civil::time(12, 15, 0, 0));
    assert_eq!(saved.lunch_end, jiff::civil::time(14, 0, 0, 0));
    assert_eq!(
        saved.preferred_latest_departure,
        jiff::civil::time(17, 45, 0, 0)
    );

    let mut short = self::onboarding(None);
    short.advance();
    short.set_target("4:00");
    short.advance();
    assert_eq!(
        short.advance().unwrap().minimum_effective_target,
        SignedDuration::from_mins(4 * 60)
    );
}

#[test]
fn previous_goes_back_with_the_drafts_even_invalid() {
    let mut onboarding = onboarding(Some(Language::French));
    onboarding.advance();
    onboarding.set_target("abc");
    onboarding.back();

    assert_eq!(onboarding.step(), Step::Language);
    onboarding.back();
    assert_eq!(onboarding.step(), Step::Language);
    assert_eq!(onboarding.advance(), None);
    assert_eq!(onboarding.step(), Step::Schedule);
    assert_eq!(onboarding.target(), "abc");
    assert_eq!(onboarding.advance(), None);
    assert_eq!(onboarding.step(), Step::Schedule);
}

#[test]
fn the_last_step_offers_startup_updates_and_the_usage_signal() {
    let onboarding = on_connections(Some(StartupRegistrationState::Disabled));
    let page = onboarding.view();

    assert_eq!(page.counter, "Étape 3 sur 3");
    assert_eq!(page.heading, "Démarrage et connexions");
    assert_eq!(
        page.intro,
        "Vous pourrez tout changer plus tard dans Paramètres\u{a0}› Général."
    );
    assert_eq!((page.previous, page.next), (Some("Précédent"), "Commencer"));
    assert_eq!(
        page.body,
        Body::Rows {
            rows: vec![
                RowView {
                    setting: Setting::Startup,
                    title: "Démarrage Windows",
                    description: "WorkTime compte votre temps seulement quand il tourne. Le \
                                  lancer à l’ouverture de session évite d’oublier une matinée.",
                    tone: RowTone::Normal,
                    link: None,
                    status: None,
                    error: None,
                    control: Control::Switch {
                        switch: Switch::Startup,
                        name: "Démarrer WorkTime avec Windows",
                        on: true,
                        enabled: true,
                    },
                    details: None,
                },
                RowView {
                    setting: Setting::Updates,
                    title: "Rechercher les mises à jour",
                    description: "Vérifie une fois par jour si une nouvelle version est disponible.",
                    tone: RowTone::Normal,
                    link: None,
                    status: None,
                    error: None,
                    control: Control::Switch {
                        switch: Switch::Updates,
                        name: "Rechercher les mises à jour",
                        on: true,
                        enabled: true,
                    },
                    details: None,
                },
                RowView {
                    setting: Setting::UsagePing,
                    title: "Envoyer un signal d’utilisation anonyme",
                    description: "Une fois par jour, sans identifiant ni donnée sur votre \
                                  temps de travail. Ce décompte aide le développeur à savoir \
                                  combien de personnes utilisent WorkTime.",
                    tone: RowTone::Normal,
                    link: None,
                    status: None,
                    error: None,
                    control: Control::Switch {
                        switch: Switch::UsagePing,
                        name: "Signal d’utilisation anonyme",
                        on: true,
                        enabled: true,
                    },
                    details: Some(Details {
                        label: "Voir ce qui est envoyé",
                        shown: None,
                    }),
                },
            ],
            footnote: Some("Vos heures et votre activité restent sur ce PC."),
        }
    );
}

#[test]
fn start_with_windows_shows_what_windows_decides() {
    let shown = |state| {
        let onboarding = on_connections(state);
        (
            switch(&onboarding, Setting::Startup),
            row(&onboarding, Setting::Startup).status,
        )
    };

    assert_eq!(shown(None), ((true, false), None));
    assert_eq!(
        shown(Some(StartupRegistrationState::Enabled)),
        ((true, true), None)
    );
    assert_eq!(
        shown(Some(StartupRegistrationState::DisabledByPolicy)),
        (
            (false, false),
            Some("Votre organisation empêche WorkTime de démarrer à l’ouverture de session.")
        )
    );
    assert_eq!(
        shown(Some(StartupRegistrationState::EnabledByPolicy)),
        (
            (true, false),
            Some("Votre organisation démarre WorkTime à l’ouverture de session.")
        )
    );
    assert_eq!(
        shown(Some(StartupRegistrationState::Unavailable)),
        (
            (false, false),
            Some("Le démarrage automatique n’est pas disponible pour cette copie de WorkTime.")
        )
    );
    assert_eq!(
        shown(Some(StartupRegistrationState::RegistrationMismatch)),
        (
            (true, true),
            Some(
                "Une autre copie utilise l’entrée de démarrage WorkTime. Activez ce réglage \
                 pour utiliser cette copie."
            )
        )
    );
}

#[test]
fn the_zip_build_leaves_the_updates_out() {
    let mut onboarding = Onboarding::new(
        WorkTimeSettings::default(),
        UsagePingFacts {
            build: Build::new("1.0.0", Channel::Zip).unwrap(),
            released: None,
            today: TODAY,
            last_sent: None,
        },
        Some(Language::French),
    );
    onboarding.advance();
    onboarding.advance();

    assert_eq!(
        rows(&onboarding)
            .iter()
            .map(|row| row.setting)
            .collect::<Vec<_>>(),
        [Setting::Startup, Setting::UsagePing]
    );
    let saved = onboarding.advance().expect("« Commencer » saves");
    assert!(saved.updates.check_for_updates);
}

#[test]
fn the_switches_change_what_commencer_saves() {
    let mut onboarding = on_connections(Some(StartupRegistrationState::Enabled));
    onboarding.toggle(Switch::Startup);
    onboarding.toggle(Switch::Updates);
    onboarding.toggle(Switch::UsagePing);

    assert_eq!(switch(&onboarding, Setting::UsagePing), (false, true));
    let saved = onboarding.advance().expect("« Commencer » saves");
    assert_eq!(
        (
            saved.start_with_windows,
            saved.updates.check_for_updates,
            saved.usage_ping,
            saved.activity_source,
        ),
        (false, false, false, Some(ActivitySourceKind::BuiltIn))
    );

    let mut held = on_connections(Some(StartupRegistrationState::DisabledByUser));
    held.toggle(Switch::Startup);
    assert!(held.advance().unwrap().start_with_windows);
}

#[test]
fn see_what_is_sent_shows_the_body_and_what_the_server_keeps() {
    let mut onboarding = on_connections(None);
    onboarding.toggle_details();

    assert_eq!(
        row(&onboarding, Setting::UsagePing).details,
        Some(Details {
            label: "Voir ce qui est envoyé",
            shown: Some(PingDetails::Brief {
                body: "{\n  \"version\": \"1.0.0\",\n  \"channel\": \"store\",\n  \
                       \"first_this_month\": true\n}"
                    .to_owned(),
                note: "C’est tout ce qui est envoyé. Le serveur ne conserve qu’un total par \
                       jour, par version et par canal. Il ne garde ni votre adresse IP ni rien \
                       qui permette de vous reconnaître."
                    .to_owned(),
            }),
        })
    );

    onboarding.toggle(Switch::UsagePing);
    assert_eq!(
        row(&onboarding, Setting::UsagePing).details.unwrap().shown,
        Some(PingDetails::Off(
            "Désactivé\u{a0}: WorkTime n’envoie aucun signal."
        ))
    );

    let mut english = self::onboarding(Some(Language::English));
    english.advance();
    english.advance();
    english.toggle_details();
    let Some(PingDetails::Brief { note, .. }) =
        row(&english, Setting::UsagePing).details.unwrap().shown
    else {
        panic!("the details of a signal sent");
    };
    assert_eq!(
        note,
        "That’s everything that gets sent. The server keeps only a daily total per version and \
         channel. It keeps neither your IP address nor anything that could identify you."
    );
}

#[test]
fn saving_freezes_the_page_and_a_failure_lets_commencer_try_again() {
    let mut onboarding = on_connections(Some(StartupRegistrationState::Enabled));
    assert!(onboarding.advance().is_some());

    let page = onboarding.view();
    assert!(page.saving);
    assert_eq!(switch(&onboarding, Setting::Updates), (true, false));
    assert_eq!(onboarding.advance(), None);
    onboarding.back();
    assert_eq!(onboarding.step(), Step::Connections);

    onboarding.finished(false);
    let page = onboarding.view();
    assert_eq!(
        (page.saving, page.failure),
        (
            false,
            Some("WorkTime n’a pas pu enregistrer votre configuration. Réessayez.")
        )
    );
    assert!(onboarding.advance().is_some());
    assert_eq!(onboarding.view().failure, None);
}

/// Answers `state`, and settles every change on `settled`, recording it.
struct Registration {
    state: StartupRegistrationState,
    settled: StartupRegistrationState,
    changes: Mutex<Vec<bool>>,
}

impl Registration {
    fn new(state: StartupRegistrationState, settled: StartupRegistrationState) -> Self {
        Self {
            state,
            settled,
            changes: Mutex::new(Vec::new()),
        }
    }
}

impl StartupPort for Registration {
    fn state(&self) -> StartupRegistrationState {
        self.state
    }

    fn set_enabled(&self, enabled: bool) -> StartupRegistrationState {
        self.changes.lock().unwrap().push(enabled);
        self.settled
    }
}

#[test]
fn complete_asks_windows_then_saves_what_it_settled_on() {
    use StartupRegistrationState::*;
    let saved = |requested: bool, port: Option<&Registration>| {
        let settings = WorkTimeSettings {
            start_with_windows: requested,
            ..WorkTimeSettings::default()
        };
        let mut written = None;
        let port = port.map(|port| port as &dyn StartupPort);
        let result = complete(settings, port, |settings| {
            written = Some(settings.start_with_windows);
            Ok(())
        });
        assert_eq!(result.unwrap().start_with_windows, written.unwrap());
        written.unwrap()
    };

    let taken = Registration::new(Disabled, Enabled);
    assert!(saved(true, Some(&taken)));
    assert_eq!(*taken.changes.lock().unwrap(), [true]);

    let refused = Registration::new(Disabled, DisabledByUser);
    assert!(!saved(true, Some(&refused)));

    let held = Registration::new(DisabledByPolicy, Enabled);
    assert!(!saved(true, Some(&held)));
    assert!(held.changes.lock().unwrap().is_empty());

    let already = Registration::new(Enabled, Disabled);
    assert!(saved(true, Some(&already)));
    assert!(already.changes.lock().unwrap().is_empty());

    assert!(saved(true, None));
    assert!(!saved(false, None));
}

#[test]
fn complete_reports_a_failed_save() {
    let result = complete(WorkTimeSettings::default(), None, |_| {
        Err(SaveSettingsError::Io(io::Error::other("disk unavailable")))
    });

    assert!(
        matches!(&result, Err(SaveSettingsError::Io(error)) if error.to_string() == "disk unavailable"),
        "{result:?}"
    );
}
