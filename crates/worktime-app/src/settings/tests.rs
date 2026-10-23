use std::io;

use jiff::SignedDuration;
use jiff::civil::Date;
use worktime_core::settings::SettingsPort;
use worktime_domain::configuration::WorkTimeSettings;
use worktime_store::settings::SaveSettingsError;

use super::*;

/// Records the saves; fails them while `fail`, and the recalculations while `stale`.
#[derive(Default)]
struct Port {
    saved: Vec<WorkTimeSettings>,
    fail: bool,
    stale: bool,
}

impl SettingsPort for Port {
    fn save(&mut self, settings: &WorkTimeSettings) -> Result<(), SaveSettingsError> {
        if self.fail {
            return Err(SaveSettingsError::Io(io::Error::other("disk unavailable")));
        }
        self.saved.push(settings.clone());
        Ok(())
    }

    fn refresh(&mut self) -> bool {
        !self.stale
    }
}

const TODAY: Date = jiff::civil::date(2026, 10, 13);

fn usage_ping(last_sent: Option<Date>, channel: Channel) -> UsagePingFacts {
    UsagePingFacts {
        build: Build::new("1.0.0", channel).unwrap(),
        released: None,
        today: TODAY,
        last_sent,
    }
}

/// The settings an onboarding saves: the built-in detection selected.
fn built_in() -> WorkTimeSettings {
    WorkTimeSettings {
        activity_source: Some(ActivitySourceKind::BuiltIn),
        ..WorkTimeSettings::default()
    }
}

fn form(language: Language) -> SettingsForm {
    SettingsForm::new(
        built_in(),
        usage_ping(Some(TODAY), Channel::Store),
        language,
    )
}

fn on_page(page: Page, language: Language) -> SettingsForm {
    let mut form = form(language);
    assert!(form.go_to(page));
    form
}

fn run(form: &mut SettingsForm, pending: Option<Pending>, port: &mut Port) {
    form.run(pending.expect("the change is accepted"), port);
}

fn titles(view: &PageView) -> Vec<(Option<&'static str>, Vec<&'static str>)> {
    view.sections
        .iter()
        .map(|section| {
            (
                section.heading,
                section.rows.iter().map(|row| row.title).collect(),
            )
        })
        .collect()
}

fn row(view: &PageView, setting: Setting) -> &RowView {
    view.sections
        .iter()
        .flat_map(|section| &section.rows)
        .find(|row| row.setting == setting)
        .expect("the page shows the setting")
}

#[test]
fn the_pages_are_named_as_the_dotnet_navigation() {
    let labels = |language| Page::ALL.map(|page| page.label(language));
    assert_eq!(
        labels(Language::French),
        ["Général", "Horaires", "Comptabilisation", "Notifications"]
    );
    assert_eq!(
        labels(Language::English),
        ["General", "Schedule", "Accounting", "Notifications"]
    );
    assert_eq!(
        Page::ALL.map(Page::icon),
        ["house", "clock", "sigma", "bell"]
    );
}

#[test]
fn the_schedule_shows_the_saved_settings_in_two_groups() {
    let form = on_page(Page::Schedule, Language::French);
    let view = form.view();

    assert_eq!(view.title, "Horaires");
    assert_eq!(view.status, Some("Paramètres enregistrés."));
    assert!(!view.saving);
    assert_eq!(
        titles(&view),
        [
            (
                Some("Objectifs"),
                vec!["Objectif quotidien", "Objectif minimum"]
            ),
            (
                Some("Journée"),
                vec![
                    "Pause déjeuner",
                    "Heure limite de départ souhaitée",
                    "Marge d’alerte"
                ]
            ),
        ]
    );
    assert_eq!(
        row(&view, Setting::Departure).description,
        "WorkTime vous prévient quand votre départ prévu approche de cette heure ou la dépasse."
    );
    assert_eq!(
        row(&view, Setting::Margin).control,
        Control::Number {
            field: Field::Margin,
            unit: "min",
            invalid: false,
            enabled: true,
        }
    );
    assert_eq!(
        [Field::Target, Field::Minimum, Field::Margin].map(|field| form.text(field)),
        ["7:00", "6:00", "15"]
    );
    assert_eq!(
        [Clock::LunchStart, Clock::LunchEnd, Clock::Departure].map(|clock| form.time(clock).text()),
        ["12:00", "14:00", "18:30"]
    );
}

#[test]
fn the_pages_read_in_english() {
    let view = on_page(Page::Schedule, Language::English).view();
    assert_eq!(view.status, Some("Settings saved."));
    assert_eq!(
        titles(&view),
        [
            (Some("Targets"), vec!["Daily target", "Minimum target"]),
            (
                Some("Workday"),
                vec![
                    "Lunch window",
                    "Preferred latest departure",
                    "Warning margin"
                ]
            ),
        ]
    );
    assert_eq!(
        row(&view, Setting::Departure).description,
        "WorkTime warns you when your projected departure gets close to or passes this time."
    );

    let view = on_page(Page::Notifications, Language::English).view();
    assert_eq!(
        titles(&view),
        [(
            None,
            vec![
                "Daily target approaching",
                "Preferred departure approaching",
                "Daily target complete"
            ]
        )]
    );
    assert_eq!(
        row(&view, Setting::Completion).description,
        "Notify this many minutes before you reach the daily target."
    );
    assert_eq!(
        row(&view, Setting::DepartureNotification).description,
        "Notify this many minutes before your preferred latest departure."
    );
}

#[test]
fn general_has_the_application_the_detection_and_the_internet_connections() {
    let form = form(Language::French);
    let view = form.view();
    assert_eq!(view.title, "Général");
    assert_eq!(view.source, None);
    assert_eq!(
        titles(&view),
        [
            (Some("Application"), vec!["Langue", "Démarrage Windows"]),
            (Some("Détection de l’activité"), vec!["Seuil d’inactivité"]),
            (
                Some("Connexions à Internet"),
                vec![
                    "Rechercher les mises à jour",
                    "Envoyer un signal d’utilisation anonyme"
                ]
            ),
        ]
    );
    assert_eq!(
        row(&view, Setting::Language).description,
        "Langue de WorkTime, appliquée au prochain démarrage."
    );
    assert_eq!(
        row(&view, Setting::IdleThreshold).description,
        "Sans clavier ni souris pendant ce délai, en secondes, WorkTime compte une absence \
         depuis votre dernière saisie. De 30 secondes à 30 minutes."
    );
    assert_eq!(
        row(&view, Setting::IdleThreshold).control,
        Control::Number {
            field: Field::IdleThreshold,
            unit: "s",
            invalid: false,
            enabled: true,
        }
    );
    assert_eq!(form.text(Field::IdleThreshold), "180");
    assert_eq!(
        row(&view, Setting::UsagePing).control,
        Control::Switch {
            switch: Switch::UsagePing,
            name: "Signal d’utilisation anonyme",
            on: true,
            enabled: true,
        }
    );
    assert_eq!(
        LanguageChoice::ALL.map(|choice| choice.label(Language::French)),
        ["Système", "Français", "English"]
    );

    let view = self::form(Language::English).view();
    assert_eq!(
        titles(&view),
        [
            (Some("App"), vec!["Language", "Windows startup"]),
            (Some("Activity detection"), vec!["Idle threshold"]),
            (
                Some("Internet connections"),
                vec!["Check for updates", "Send an anonymous usage signal"]
            ),
        ]
    );
    assert_eq!(
        row(&view, Setting::Language).description,
        "WorkTime’s language, applied at next start."
    );
}

#[test]
fn general_ends_with_the_version_and_the_day_it_was_published() {
    let facts = UsagePingFacts {
        build: Build::new("0.1.0-alpha.242", Channel::Direct).unwrap(),
        released: Some(jiff::civil::date(2026, 10, 21)),
        ..usage_ping(None, Channel::Direct)
    };
    let form = SettingsForm::new(built_in(), facts, Language::French);
    assert_eq!(
        form.view().about,
        Some(AboutView {
            heading: "À propos",
            title: "Version",
            version: "0.1.0-alpha.242",
            label: "Version 0.1.0-alpha.242".to_owned(),
            released: Some("Publiée le 21\u{a0}octobre\u{a0}2026".to_owned()),
        })
    );

    let english = on_page(Page::General, Language::English)
        .view()
        .about
        .unwrap();
    assert_eq!(
        (english.heading, english.version, english.released),
        ("About", "1.0.0", None)
    );

    for page in [Page::Schedule, Page::Accounting, Page::Notifications] {
        assert_eq!(on_page(page, Language::French).view().about, None);
    }
}

fn startup_switch(form: &SettingsForm) -> (Control, Option<&'static str>) {
    let view = form.view();
    let row = row(&view, Setting::Startup);
    (row.control.clone(), row.status)
}

fn startup_control(on: bool, enabled: bool) -> Control {
    Control::Switch {
        switch: Switch::Startup,
        name: "Démarrer WorkTime avec Windows",
        on,
        enabled,
    }
}

#[test]
fn the_startup_switch_waits_for_the_registration() {
    let mut form = form(Language::French);
    assert_eq!(
        row(&form.view(), Setting::Startup).description,
        "Démarrer WorkTime à l’ouverture de session."
    );
    assert_eq!(startup_switch(&form), (startup_control(false, false), None));
    assert_eq!(form.toggle(Switch::Startup), None);

    form.show_startup(StartupRegistrationState::DisabledByUser);
    assert_eq!(
        startup_switch(&form),
        (
            startup_control(false, false),
            Some(
                "Les applications de démarrage Windows ont désactivé WorkTime. Réactivez-le \
                 dans Paramètres > Applications > Démarrage."
            )
        )
    );
    assert_eq!(form.toggle(Switch::Startup), None);
}

#[test]
fn turning_startup_off_saves_once_windows_settled_on_it() {
    let mut form = form(Language::French);
    let mut port = Port::default();
    form.show_startup(StartupRegistrationState::Enabled);
    assert_eq!(
        startup_switch(&form),
        (
            startup_control(true, true),
            Some("WorkTime démarrera à la prochaine ouverture de session.")
        )
    );

    let pending = form.toggle(Switch::Startup).expect("the switch can change");
    assert_eq!(pending.startup(), Some(false));
    assert_eq!(form.view().status, Some("Enregistrement des paramètres…"));
    assert_eq!(startup_switch(&form).0, startup_control(false, false));
    assert!(port.saved.is_empty());

    form.finish_startup(pending, StartupRegistrationState::Disabled, &mut port);
    assert_eq!(
        startup_switch(&form),
        (
            startup_control(false, true),
            Some("WorkTime ne démarrera pas à l’ouverture de session.")
        )
    );
    assert!(!port.saved[0].start_with_windows);
    assert_eq!(form.view().status, Some("Paramètres enregistrés."));
}

#[test]
fn a_startup_change_windows_refused_shows_a_callout_and_retries() {
    for (language, message, retry) in [
        (
            Language::French,
            "Windows n’a pas modifié le démarrage. Vérifiez l’état ci-dessus et réessayez.",
            "Réessayer",
        ),
        (
            Language::English,
            "Windows did not apply the startup change. Check the status above and try again.",
            "Retry",
        ),
    ] {
        let mut form = form(language);
        let mut port = Port::default();
        form.show_startup(StartupRegistrationState::Disabled);
        let pending = form.toggle(Switch::Startup).expect("the switch can change");
        assert_eq!(pending.startup(), Some(true));
        // Windows answered without enabling the task.
        form.finish_startup(pending, StartupRegistrationState::Disabled, &mut port);

        let view = form.view();
        assert_eq!(view.status, None);
        assert_eq!(
            view.sections[0].callouts,
            [CalloutView {
                group: EditGroup::Startup,
                tone: Tone::Error,
                message,
                retry,
            }]
        );
        let row = row(&view, Setting::Startup);
        assert_eq!(
            (row.control.clone(), row.status),
            (
                Control::Switch {
                    switch: Switch::Startup,
                    name: Switch::Startup.name(language),
                    on: false,
                    enabled: true,
                },
                Some(language.text(Key::StartupDisabled))
            )
        );
        assert!(port.saved.is_empty());
        assert!(!form.may_close());
        form.keep_editing();

        let pending = form.retry(EditGroup::Startup).expect("the change failed");
        assert_eq!(pending.startup(), Some(true));
        form.finish_startup(pending, StartupRegistrationState::Enabled, &mut port);
        assert!(form.view().sections[0].callouts.is_empty());
        assert!(
            port.saved.is_empty(),
            "the settings already start with Windows"
        );
        assert!(form.current().start_with_windows);
    }
}

#[test]
fn the_idle_threshold_is_saved_in_seconds_within_30_seconds_and_30_minutes() {
    let mut form = form(Language::French);
    let mut port = Port::default();

    let pending = form.spin(Field::IdleThreshold, 1);
    run(&mut form, pending, &mut port);
    assert_eq!(form.text(Field::IdleThreshold), "210");
    assert_eq!(
        port.saved[0].built_in_activity.idle_threshold,
        SignedDuration::from_secs(210)
    );
    form.set_text(Field::IdleThreshold, "45");
    let pending = form.spin(Field::IdleThreshold, -1);
    run(&mut form, pending, &mut port);
    assert_eq!(form.text(Field::IdleThreshold), "30");

    form.set_text(Field::IdleThreshold, "1801");
    assert_eq!(form.commit(EditGroup::IdleThreshold), None);
    assert_eq!(
        row(&form.view(), Setting::IdleThreshold).error,
        Some("Saisissez un nombre entier compris dans la plage affichée.")
    );
    form.set_text(Field::IdleThreshold, "2 min");
    assert_eq!(form.commit(EditGroup::IdleThreshold), None);
    assert_eq!(
        row(&form.view(), Setting::IdleThreshold).error,
        Some("Saisissez un nombre entier de secondes.")
    );
    assert_eq!(
        form.current().built_in_activity.idle_threshold,
        SignedDuration::from_secs(30)
    );
}

#[test]
fn the_updates_and_the_usage_signal_switch_off_on_their_own() {
    let mut form = form(Language::French);
    let mut port = Port::default();

    let pending = form.toggle(Switch::Updates);
    run(&mut form, pending, &mut port);
    let pending = form.toggle(Switch::UsagePing);
    run(&mut form, pending, &mut port);

    assert!(!port.saved[0].updates.check_for_updates);
    assert!(port.saved[0].usage_ping);
    assert!(!port.saved[1].updates.check_for_updates);
    assert!(!port.saved[1].usage_ping);
}

#[test]
fn the_zip_build_greys_the_updates_and_links_to_github() {
    let mut form = SettingsForm::new(
        built_in(),
        usage_ping(None, Channel::Zip),
        Language::English,
    );

    assert!(form.toggle(Switch::Updates).is_none());
    assert_eq!(
        row(&form.view(), Setting::Updates),
        &RowView {
            setting: Setting::Updates,
            title: "Check for updates",
            description: "The ZIP version doesn’t check for updates. Get new versions from GitHub.",
            tone: RowTone::Unavailable,
            link: Some(Link {
                label: "GitHub",
                url: "https://github.com/Thanaen/WorkTime/releases",
            }),
            status: None,
            error: None,
            control: Control::Switch {
                switch: Switch::Updates,
                name: "Check for updates",
                on: false,
                enabled: false,
            },
            details: None,
        }
    );
    assert!(form.current().updates.check_for_updates);
}

#[test]
fn the_zip_build_says_so_in_french() {
    let form = SettingsForm::new(built_in(), usage_ping(None, Channel::Zip), Language::French);

    assert_eq!(
        row(&form.view(), Setting::Updates).description,
        "La version ZIP ne recherche pas les mises à jour. Téléchargez les nouvelles versions \
         sur GitHub."
    );
}

#[test]
fn the_installed_build_checks_downloads_and_says_so() {
    let mut form = SettingsForm::new(
        built_in(),
        usage_ping(None, Channel::Direct),
        Language::English,
    );

    let updates = row(&form.view(), Setting::Updates).clone();
    assert_eq!(updates.tone, RowTone::Normal);
    assert_eq!(
        updates.description,
        "Downloads any new version once a day. It installs the next time WorkTime starts."
    );
    assert!(form.toggle(Switch::Updates).is_some());
}

fn details(form: &SettingsForm) -> Details {
    row(&form.view(), Setting::UsagePing)
        .details
        .clone()
        .expect("the usage signal has details")
}

#[test]
fn see_what_is_sent_shows_the_next_signal_and_the_last_send() {
    let mut form = form(Language::French);
    assert_eq!(
        details(&form),
        Details {
            label: "Voir ce qui est envoyé",
            shown: None,
        }
    );

    form.toggle_details();

    assert_eq!(
        details(&form).shown,
        Some(PingDetails::On {
            body: "{\n  \"version\": \"1.0.0\",\n  \"channel\": \"store\",\n  \
                   \"first_this_month\": false\n}"
                .to_owned(),
            fields: "C’est tout ce qui est envoyé. «\u{a0}channel\u{a0}» indique où vous avez \
                     installé WorkTime (ici, le Microsoft\u{a0}Store) et «\u{a0}first_this_month\u{a0}» \
                     vaut true pour le premier envoi du mois."
                .to_owned(),
            server: "Le serveur ne garde rien et transmet ce signal à PostHog, hébergé dans \
                     l’Union européenne, sans votre adresse IP ni rien qui permette de vous \
                     reconnaître.",
            last_sent: "Dernier envoi\u{a0}: aujourd’hui".to_owned(),
        })
    );
    let mut port = Port::default();
    let pending = form.toggle(Switch::UsagePing);
    run(&mut form, pending, &mut port);
    assert_eq!(
        details(&form).shown,
        Some(PingDetails::Off(
            "Désactivé\u{a0}: WorkTime n’envoie aucun signal."
        ))
    );
    form.toggle_details();
    assert_eq!(details(&form).shown, None);
}

#[test]
fn the_last_send_reads_never_yesterday_or_its_date() {
    let shown = |last_sent: Option<Date>, channel: Channel, language: Language| {
        let mut form = SettingsForm::new(built_in(), usage_ping(last_sent, channel), language);
        form.toggle_details();
        match details(&form).shown {
            Some(PingDetails::On {
                body, last_sent, ..
            }) => (body.contains("\"first_this_month\": true"), last_sent),
            shown => panic!("the signal is on: {shown:?}"),
        }
    };

    assert_eq!(
        shown(None, Channel::Store, Language::French),
        (true, "Dernier envoi\u{a0}: jamais".to_owned())
    );
    assert_eq!(
        shown(
            Some(jiff::civil::date(2026, 10, 12)),
            Channel::Store,
            Language::French
        ),
        (false, "Dernier envoi\u{a0}: hier".to_owned())
    );
    assert_eq!(
        shown(
            Some(jiff::civil::date(2026, 9, 30)),
            Channel::Store,
            Language::French
        ),
        (true, "Dernier envoi\u{a0}: 30\u{a0}septembre".to_owned())
    );
    assert_eq!(
        shown(
            Some(jiff::civil::date(2026, 9, 30)),
            Channel::Zip,
            Language::English
        ),
        (true, "Last sent: September\u{a0}30".to_owned())
    );

    let mut form = SettingsForm::new(
        built_in(),
        usage_ping(None, Channel::Zip),
        Language::English,
    );
    form.toggle_details();
    let Some(PingDetails::On { body, fields, .. }) = details(&form).shown else {
        panic!("the signal is on");
    };
    assert!(body.contains("\"channel\": \"zip\""), "{body}");
    assert_eq!(
        fields,
        "That’s everything that gets sent. “channel” says where you installed WorkTime (here, the ZIP \
         download) and “first_this_month” is true for the first signal of the month."
    );
}

#[test]
fn settings_naming_activitywatch_offer_the_built_in_detection_on_general() {
    let settings = WorkTimeSettings {
        activity_source: Some(ActivitySourceKind::ActivityWatch),
        ..WorkTimeSettings::default()
    };
    let mut form = SettingsForm::new(settings, usage_ping(None, Channel::Store), Language::French);
    let mut port = Port::default();

    assert_eq!(
        form.view().source,
        Some(SourceCallout {
            title: "ActivityWatch n’est plus pris en charge",
            message: "WorkTime détecte maintenant l’activité lui-même. Passez à la détection \
                      intégrée pour que votre temps continue d’être compté.",
            action: "Utiliser la détection intégrée",
            failure: None,
        })
    );
    assert_eq!(
        Page::ALL.map(|page| form.warns(page)),
        [true, false, false, false]
    );
    assert!(form.go_to(Page::Schedule));
    assert_eq!(form.view().source, None);
    assert!(form.go_to(Page::General));

    port.fail = true;
    let pending = form.use_built_in();
    run(&mut form, pending, &mut port);
    let failure = form.view().source.unwrap().failure.unwrap();
    assert_eq!(
        failure.message,
        "Impossible de passer à la détection intégrée. Vérifiez l’accès au disque et réessayez."
    );

    port.fail = false;
    let pending = form.retry(failure.group);
    run(&mut form, pending, &mut port);
    assert_eq!(
        port.saved[0].activity_source,
        Some(ActivitySourceKind::BuiltIn)
    );
    assert_eq!(form.view().source, None);
    assert!(!form.warns(Page::General));
}

#[test]
fn settings_without_a_source_name_activitywatch() {
    let form = SettingsForm::new(
        WorkTimeSettings::default(),
        usage_ping(None, Channel::Store),
        Language::English,
    );
    assert_eq!(
        form.view().source.map(|callout| callout.title),
        Some("ActivityWatch is no longer supported")
    );
}

#[test]
fn the_file_is_the_last_navigation_item() {
    let mut form = form(Language::French);

    assert!(form.go_to(Page::Json));

    assert_eq!(Page::Json.label(Language::French), "settings.json");
    assert_eq!(Page::Json.icon(), "braces");
    let view = form.view();
    assert_eq!(view.title, "settings.json");
    assert!(view.sections.is_empty());
    assert_eq!(view.source, None);
}

#[test]
fn a_language_is_saved_for_the_next_start() {
    let mut form = form(Language::French);
    let mut port = Port::default();

    let pending = form.choose_language(LanguageChoice::English);
    run(&mut form, pending, &mut port);

    assert_eq!(port.saved[0].language_override.as_deref(), Some("en-US"));
    assert_eq!(form.choose_language(LanguageChoice::English), None);
    let pending = form.choose_language(LanguageChoice::System);
    run(&mut form, pending, &mut port);
    assert_eq!(port.saved[1].language_override, None);
    assert_eq!(
        row(&form.view(), Setting::Language).control,
        Control::Language {
            name: "Langue de l’application",
            value: LanguageChoice::System,
        }
    );
}

#[test]
fn a_target_out_of_range_shows_its_message_under_its_field() {
    for (language, message) in [
        (
            Language::French,
            "Saisissez une durée comprise entre 0:01 et 24:00.",
        ),
        (Language::English, "Enter a duration from 0:01 to 24:00."),
    ] {
        let mut form = on_page(Page::Schedule, language);
        form.set_text(Field::Target, "25:00");

        assert_eq!(form.commit(EditGroup::Targets), None);

        let view = form.view();
        assert_eq!(view.status, None);
        let target = row(&view, Setting::Target);
        assert_eq!(target.error, Some(message));
        assert_eq!(
            target.control,
            Control::Text {
                field: Field::Target,
                invalid: true,
                enabled: true,
            }
        );
        assert_eq!(row(&view, Setting::Minimum).error, None);
    }
}

#[test]
fn a_minimum_above_the_target_is_refused_under_the_minimum() {
    let mut form = on_page(Page::Schedule, Language::French);
    let mut port = Port::default();
    form.set_text(Field::Minimum, "8:00");

    assert_eq!(form.commit(EditGroup::Targets), None);
    assert_eq!(
        row(&form.view(), Setting::Minimum).error,
        Some("L’objectif minimum doit être positif et ne pas dépasser l’objectif quotidien.")
    );
    assert_eq!(row(&form.view(), Setting::Target).error, None);

    form.set_text(Field::Target, "8:30");
    let pending = form.commit(EditGroup::Targets);
    run(&mut form, pending, &mut port);
    assert_eq!(
        port.saved[0].nominal_daily_target,
        SignedDuration::from_mins(510)
    );
    assert_eq!(
        port.saved[0].minimum_effective_target,
        SignedDuration::from_mins(480)
    );
    assert_eq!(form.view().status, Some("Paramètres enregistrés."));
}

#[test]
fn a_lunch_ending_before_it_starts_is_refused_on_the_lunch_row() {
    let mut form = on_page(Page::Schedule, Language::English);
    form.time_mut(Clock::LunchEnd)
        .step(crate::adjust::TimePart::Hours, -3);

    assert_eq!(form.commit(EditGroup::DayTimes), None);

    let view = form.view();
    assert_eq!(
        row(&view, Setting::Lunch).error,
        Some("Lunch must end after it starts.")
    );
    assert_eq!(
        row(&view, Setting::Lunch).control,
        Control::Range {
            start: Clock::LunchStart,
            end: Clock::LunchEnd,
            invalid: true,
        }
    );
}

#[test]
fn a_save_shows_saving_until_it_runs() {
    let mut form = on_page(Page::Schedule, Language::French);
    form.set_text(Field::Target, "7:24");

    let pending = form.commit(EditGroup::Targets);

    let view = form.view();
    assert_eq!(view.status, Some("Enregistrement des paramètres…"));
    assert!(view.saving);
    assert!(form.is_saving());
    run(&mut form, pending, &mut Port::default());
    assert_eq!(form.view().status, Some("Paramètres enregistrés."));
    assert_eq!(
        form.current().nominal_daily_target,
        SignedDuration::from_mins(444)
    );
}

#[test]
fn a_failed_save_shows_a_callout_under_its_group_until_retried() {
    for (language, message, retry) in [
        (
            Language::French,
            "Impossible d’enregistrer les paramètres. Vérifiez l’accès au disque et réessayez.",
            "Réessayer",
        ),
        (
            Language::English,
            "Settings could not be saved. Check disk access and try again.",
            "Retry",
        ),
    ] {
        let mut form = on_page(Page::Schedule, language);
        let mut port = Port {
            fail: true,
            ..Port::default()
        };
        form.time_mut(Clock::Departure)
            .step(crate::adjust::TimePart::Hours, 1);
        let pending = form.commit(EditGroup::DayTimes);
        run(&mut form, pending, &mut port);

        let view = form.view();
        assert_eq!(view.status, None);
        assert!(view.sections[0].callouts.is_empty());
        assert_eq!(
            view.sections[1].callouts,
            [CalloutView {
                group: EditGroup::DayTimes,
                tone: Tone::Error,
                message,
                retry,
            }]
        );

        port.fail = false;
        let pending = form.retry(EditGroup::DayTimes);
        run(&mut form, pending, &mut port);
        assert!(form.view().sections[1].callouts.is_empty());
        assert_eq!(
            port.saved[0].preferred_latest_departure,
            jiff::civil::time(19, 30, 0, 0)
        );
    }
}

#[test]
fn a_day_left_to_recalculate_shows_a_warning_with_retry() {
    let mut form = on_page(Page::Accounting, Language::French);
    let mut port = Port {
        stale: true,
        ..Port::default()
    };

    let pending = form.toggle(Switch::Credit);
    run(&mut form, pending, &mut port);

    assert_eq!(
        form.view().sections[0].callouts,
        [CalloutView {
            group: EditGroup::Credit,
            tone: Tone::Warning,
            message: "Réglages enregistrés. Actualisez pour mettre à jour votre journée.",
            retry: "Réessayer",
        }]
    );
    port.stale = false;
    let pending = form.retry(EditGroup::Credit);
    run(&mut form, pending, &mut port);
    assert!(form.view().sections[0].callouts.is_empty());
    assert_eq!(port.saved.len(), 1);
}

#[test]
fn debt_follows_weekly_credit_and_its_limit_follows_debt() {
    let mut form = on_page(Page::Accounting, Language::French);
    let mut port = Port::default();
    let view = form.view();
    assert_eq!(
        titles(&view),
        [(
            None,
            vec![
                "Crédit hebdomadaire",
                "Dette hebdomadaire",
                "Limite de dette",
                "Vérification des verrouillages longs"
            ]
        )]
    );
    assert_eq!(row(&view, Setting::Debt).tone, RowTone::Normal);
    assert_eq!(row(&view, Setting::DebtLimit).tone, RowTone::Disabled);
    assert_eq!(
        Field::DebtLimit.placeholder(Language::French),
        "Aucune limite"
    );
    assert_eq!(Field::DebtLimit.placeholder(Language::English), "No limit");

    let pending = form.toggle(Switch::Debt);
    run(&mut form, pending, &mut port);
    assert_eq!(row(&form.view(), Setting::DebtLimit).tone, RowTone::Normal);
    form.set_text(Field::DebtLimit, "2:30");
    let pending = form.commit(EditGroup::Debt);
    run(&mut form, pending, &mut port);
    assert_eq!(
        port.saved[1].maximum_weekly_debt,
        Some(SignedDuration::from_mins(150))
    );

    let pending = form.toggle(Switch::Credit);
    run(&mut form, pending, &mut port);
    let view = form.view();
    assert_eq!(row(&view, Setting::Debt).tone, RowTone::Disabled);
    assert_eq!(row(&view, Setting::DebtLimit).tone, RowTone::Disabled);
    assert_eq!(
        row(&view, Setting::Debt).control,
        Control::Switch {
            switch: Switch::Debt,
            name: "Dette hebdomadaire",
            on: true,
            enabled: false,
        }
    );
    assert_eq!(form.toggle(Switch::Debt), None);
}

#[test]
fn a_debt_limit_that_is_not_a_duration_is_refused() {
    let mut form = on_page(Page::Accounting, Language::English);
    form.toggle(Switch::Debt);
    form.set_text(Field::DebtLimit, "2h");

    assert_eq!(form.commit(EditGroup::Debt), None);
    assert_eq!(
        row(&form.view(), Setting::DebtLimit).error,
        Some("Use the H:mm format.")
    );
}

#[test]
fn a_lead_time_is_enabled_with_its_rule() {
    let mut form = on_page(Page::Notifications, Language::French);
    let mut port = Port::default();
    let view = form.view();
    assert_eq!(
        row(&view, Setting::Completion).description,
        "Prévenir ce nombre de minutes avant d’atteindre l’objectif."
    );
    assert_eq!(
        row(&view, Setting::DepartureNotification).description,
        "Prévenir ce nombre de minutes avant l’heure limite de départ."
    );
    assert_eq!(
        row(&view, Setting::DayComplete).description,
        "Prévenir lorsque l’objectif quotidien est atteint."
    );
    assert_eq!(
        row(&view, Setting::Completion).control,
        Control::Rule {
            field: Field::CompletionLead,
            unit: "min",
            invalid: false,
            switch: Switch::Completion,
            name: "Notification d’approche de l’objectif quotidien",
            on: true,
        }
    );
    assert!(form.is_field_enabled(Field::CompletionLead));

    let pending = form.toggle(Switch::Completion);
    run(&mut form, pending, &mut port);
    assert!(!form.is_field_enabled(Field::CompletionLead));
    assert_eq!(form.spin(Field::CompletionLead, 1), None);
    assert_eq!(
        port.saved[0].notifications.completion_approach,
        NotificationRule::new(false, SignedDuration::from_mins(15))
    );
}

#[test]
fn spin_buttons_step_by_the_dotnet_change_within_the_range() {
    let mut form = form(Language::French);
    let mut port = Port::default();

    let pending = form.spin(Field::Margin, 1);
    run(&mut form, pending, &mut port);
    assert_eq!(form.text(Field::Margin), "20");
    let pending = form.spin(Field::LongLock, -1);
    run(&mut form, pending, &mut port);
    assert_eq!(form.text(Field::LongLock), "75");
    form.set_text(Field::DepartureLead, "3");
    let pending = form.spin(Field::DepartureLead, -1);
    run(&mut form, pending, &mut port);
    assert_eq!(form.text(Field::DepartureLead), "0");
    form.set_text(Field::Margin, "abc");
    let pending = form.spin(Field::Margin, 1);
    run(&mut form, pending, &mut port);
    assert_eq!(form.text(Field::Margin), "0");
    assert_eq!(form.spin(Field::Target, 1), None);

    assert_eq!(form.current().orange_warning_margin, SignedDuration::ZERO);
    assert_eq!(
        form.current().long_lock_review_threshold,
        SignedDuration::from_mins(75)
    );
}

#[test]
fn a_number_that_is_not_whole_is_refused() {
    let mut form = on_page(Page::Schedule, Language::French);
    form.set_text(Field::Margin, "7,5");

    assert_eq!(form.commit(EditGroup::Margin), None);
    assert_eq!(
        row(&form.view(), Setting::Margin).error,
        Some("Saisissez un nombre entier de minutes.")
    );
    form.set_text(Field::Margin, "2000");
    assert_eq!(form.commit(EditGroup::Margin), None);
    assert_eq!(
        row(&form.view(), Setting::Margin).error,
        Some("Saisissez un nombre entier compris dans la plage affichée.")
    );
}

#[test]
fn leaving_with_an_invalid_draft_asks_first() {
    let mut form = on_page(Page::Schedule, Language::French);
    form.set_text(Field::Target, "25:00");
    form.commit(EditGroup::Targets);

    assert!(!form.may_close());
    assert_eq!(
        form.dialog(),
        Some(DialogView {
            title: "Paramètres non enregistrés",
            message: "Certains paramètres sont invalides ou n’ont pas pu être enregistrés. Continuez la modification ou ignorez ces changements.",
            discard: "Ignorer les changements",
            keep_editing: "Continuer la modification",
        })
    );
    form.keep_editing();
    assert_eq!(form.dialog(), None);

    assert!(!form.go_to(Page::Notifications));
    assert_eq!(form.page(), Page::Schedule);
    assert_eq!(form.discard(), Some(Leaving::Page(Page::Notifications)));
    assert_eq!(form.page(), Page::Notifications);
    assert_eq!(form.text(Field::Target), "7:00");
    assert_eq!(form.dialog(), None);
    assert!(form.may_close());
}

#[test]
fn the_dialog_holds_its_page_while_the_window_behind_it_is_ignored() {
    let mut form = on_page(Page::Schedule, Language::French);
    form.set_text(Field::Target, "25:00");
    form.commit(EditGroup::Targets);
    assert!(!form.go_to(Page::Notifications));
    let credit = form.is_on(Switch::Credit);

    assert!(!form.go_to(Page::General));
    assert_eq!(form.toggle(Switch::Credit), None);
    assert_eq!(form.spin(Field::Margin, 1), None);
    assert_eq!(form.choose_language(LanguageChoice::English), None);
    assert_eq!(form.commit(EditGroup::Margin), None);
    assert_eq!(form.retry(EditGroup::Targets), None);

    assert_eq!(form.page(), Page::Schedule);
    assert_eq!(form.is_on(Switch::Credit), credit);
    assert_eq!(form.language(), LanguageChoice::System);
    assert_eq!(form.current(), &built_in());
    assert_eq!(form.use_built_in(), None);
    assert_eq!(form.discard(), Some(Leaving::Page(Page::Notifications)));
    assert_eq!(form.page(), Page::Notifications);
}

#[test]
fn closing_while_the_dialog_holds_a_page_closes_after_discarding() {
    let mut form = on_page(Page::Schedule, Language::French);
    form.set_text(Field::Target, "25:00");
    form.commit(EditGroup::Targets);
    assert!(!form.go_to(Page::Notifications));

    assert!(!form.may_close());
    assert_eq!(form.discard(), Some(Leaving::Window));
}

#[test]
fn the_dialog_reads_in_english_and_discards_only_the_unsaved_groups() {
    let mut form = on_page(Page::Schedule, Language::English);
    let mut port = Port::default();
    form.set_text(Field::Margin, "30");
    let pending = form.commit(EditGroup::Margin);
    form.set_text(Field::Target, "0:00");
    form.commit(EditGroup::Targets);

    assert!(!form.may_close());
    assert_eq!(
        form.dialog(),
        Some(DialogView {
            title: "Unsaved settings",
            message: "Some settings are invalid or could not be saved. Keep editing, or discard those changes.",
            discard: "Discard unsaved changes",
            keep_editing: "Keep editing",
        })
    );
    assert_eq!(form.discard(), Some(Leaving::Window));
    assert_eq!(form.text(Field::Target), "7:00");
    assert_eq!(form.text(Field::Margin), "30");
    run(&mut form, pending, &mut port);
    assert_eq!(
        form.current().orange_warning_margin,
        SignedDuration::from_mins(30)
    );
}
