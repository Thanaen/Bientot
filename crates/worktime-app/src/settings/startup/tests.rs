use std::sync::Mutex;

use StartupRegistrationState::*;

use super::*;
use crate::strings::Language;

/// Answers `state`, and settles every change on `settled`, recording it.
struct Registration {
    windows: Mutex<(StartupRegistrationState, StartupRegistrationState)>,
    changes: Mutex<Vec<bool>>,
}

impl Registration {
    fn new(state: StartupRegistrationState, settled: StartupRegistrationState) -> Self {
        Self {
            windows: Mutex::new((state, settled)),
            changes: Mutex::new(Vec::new()),
        }
    }

    /// Windows changed outside WorkTime.
    fn now(&self, state: StartupRegistrationState, settled: StartupRegistrationState) {
        *self.windows.lock().unwrap() = (state, settled);
    }

    fn changes(&self) -> Vec<bool> {
        self.changes.lock().unwrap().clone()
    }
}

impl StartupPort for Registration {
    fn state(&self) -> StartupRegistrationState {
        self.windows.lock().unwrap().0
    }

    fn set_enabled(&self, enabled: bool) -> StartupRegistrationState {
        self.changes.lock().unwrap().push(enabled);
        self.windows.lock().unwrap().1
    }
}

#[test]
fn each_state_shows_the_switch_and_the_dotnet_status() {
    let shown = |language: Language| {
        [
            Enabled,
            Disabled,
            DisabledByUser,
            DisabledByPolicy,
            EnabledByPolicy,
            RegistrationMismatch,
            Unavailable,
        ]
        .map(|state| {
            (
                is_on(state),
                can_change(state),
                language.text(status(state)),
            )
        })
    };
    assert_eq!(
        shown(Language::French),
        [
            (
                true,
                true,
                "Bientôt démarrera à la prochaine ouverture de session."
            ),
            (
                false,
                true,
                "Bientôt ne démarrera pas à l’ouverture de session."
            ),
            (
                false,
                false,
                "Les applications de démarrage Windows ont désactivé Bientôt. Réactivez-le dans \
                 Paramètres > Applications > Démarrage."
            ),
            (
                false,
                false,
                "Votre organisation empêche Bientôt de démarrer à l’ouverture de session."
            ),
            (
                true,
                false,
                "Votre organisation impose le démarrage de Bientôt à l’ouverture de session."
            ),
            (
                false,
                true,
                "Une autre copie utilise l’entrée de démarrage de Bientôt. Activez ce réglage pour \
                 utiliser cette copie."
            ),
            (
                false,
                false,
                "Le démarrage automatique n’est pas disponible pour cette copie de Bientôt."
            ),
        ]
    );
    assert_eq!(
        shown(Language::English).map(|(_, _, text)| text),
        [
            "Bientôt will start the next time you sign in.",
            "Bientôt will not start when you sign in.",
            "Windows Startup Apps has disabled Bientôt. Re-enable it in Settings > Apps > Startup.",
            "Your organization prevents Bientôt from starting when you sign in.",
            "Your organization starts Bientôt when you sign in.",
            "Another copy uses the Bientôt startup entry. Turn this setting on to use this copy.",
            "Startup registration is unavailable for this copy of Bientôt.",
        ]
    );
}

#[test]
fn a_change_is_applied_when_windows_settles_on_it() {
    let registration = Registration::new(Disabled, Enabled);
    let settled = apply(&registration, true);
    assert_eq!((settled, applied(settled, true)), (Enabled, true));
    assert_eq!(registration.changes(), [true]);

    let registration = Registration::new(RegistrationMismatch, Enabled);
    assert_eq!(apply(&registration, true), Enabled);
    assert_eq!(registration.changes(), [true]);

    let registration = Registration::new(Enabled, Disabled);
    let settled = apply(&registration, false);
    assert_eq!((settled, applied(settled, false)), (Disabled, true));
}

#[test]
fn a_refusal_by_windows_is_not_applied() {
    // The packaged task stays disabled by the user when Windows refuses to enable it.
    let registration = Registration::new(Disabled, DisabledByUser);
    let settled = apply(&registration, true);
    assert_eq!((settled, applied(settled, true)), (DisabledByUser, false));

    let registration = Registration::new(Enabled, Unavailable);
    let settled = apply(&registration, false);
    assert_eq!((settled, applied(settled, false)), (Unavailable, false));
}

#[test]
fn what_the_user_or_a_policy_holds_is_never_changed() {
    for state in [DisabledByUser, DisabledByPolicy, Unavailable] {
        let registration = Registration::new(state, Enabled);
        assert_eq!(apply(&registration, true), state);
        assert!(!applied(state, true));
        assert!(registration.changes().is_empty());
    }
    let registration = Registration::new(EnabledByPolicy, Disabled);
    assert_eq!(apply(&registration, false), EnabledByPolicy);
    assert!(registration.changes().is_empty());
}

#[test]
fn a_switch_already_there_is_left_as_is() {
    let registration = Registration::new(Enabled, Disabled);
    let settled = apply(&registration, true);
    assert_eq!((settled, applied(settled, true)), (Enabled, true));
    assert!(registration.changes().is_empty());
}

#[test]
fn a_retry_reads_windows_again() {
    // The first attempt left Startup Apps holding WorkTime; the user then re-enabled it there.
    let registration = Registration::new(Disabled, DisabledByUser);
    assert_eq!(apply(&registration, true), DisabledByUser);
    registration.now(DisabledByUser, DisabledByUser);
    assert_eq!(apply(&registration, true), DisabledByUser);

    registration.now(Disabled, Enabled);
    let settled = apply(&registration, true);
    assert_eq!((settled, applied(settled, true)), (Enabled, true));
    assert_eq!(registration.changes(), [true, true]);
}
