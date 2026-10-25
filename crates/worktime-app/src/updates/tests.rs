use std::cell::Cell;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use worktime_domain::time::FrozenClock;

use super::*;

fn now() -> Timestamp {
    "2026-10-14T07:30:00Z".parse().unwrap()
}

/// Polls a future that the fakes complete at once.
fn ready<T>(future: impl Future<Output = T>) -> T {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the future still waits"),
    }
}

/// The settings of the check, in memory.
struct Settings(Cell<UpdateSettings>);

impl Settings {
    fn new(check_for_updates: bool) -> Self {
        Self(Cell::new(UpdateSettings {
            check_for_updates,
            last_checked_at: None,
        }))
    }
}

impl UpdateCheckSettings for &Settings {
    type Error = Infallible;

    fn updates(&self) -> UpdateSettings {
        self.0.get()
    }

    fn record_check(&mut self, at: Timestamp) -> Result<(), Infallible> {
        self.0.set(UpdateSettings {
            last_checked_at: Some(at),
            ..self.0.get()
        });
        Ok(())
    }
}

/// A Store whose updates answer `install` and count the installs.
struct FakeStore {
    install: Result<(), &'static str>,
    installs: Rc<Cell<usize>>,
}

impl FakeStore {
    fn answering(install: Result<(), &'static str>) -> Self {
        Self {
            install,
            installs: Rc::default(),
        }
    }
}

struct FakeOffer {
    install: Result<(), &'static str>,
    installs: Rc<Cell<usize>>,
}

impl UpdateStore for &FakeStore {
    type Offer = FakeOffer;
    type Error = Infallible;

    fn find_update(&self) -> impl Future<Output = Result<Option<FakeOffer>, Infallible>> {
        std::future::ready(Ok(Some(FakeOffer {
            install: self.install,
            installs: Rc::clone(&self.installs),
        })))
    }
}

impl UpdateOffer for FakeOffer {
    type Error = &'static str;

    fn download_and_install(&self) -> impl Future<Output = Result<(), &'static str>> {
        self.installs.set(self.installs.get() + 1);
        std::future::ready(self.install)
    }
}

/// The offer of a daily check of `store`, as the host makes it.
fn checked<U>(store: U, settings: &Settings) -> Option<Offer>
where
    U: UpdateStore<Error = Infallible>,
    U::Offer: UpdateOffer + 'static,
    <U::Offer as UpdateOffer>::Error: Display,
{
    let mut service = StoreUpdateService::new(Some(store), settings, FrozenClock::new(now()));
    let found = ready(service.find_due_update());
    found.unwrap().map(Offer::new)
}

fn offered(store: &FakeStore) -> Updates {
    let mut updates = Updates::default();
    updates.found(checked(store, &Settings::new(true)).expect("the store has an update"));
    updates
}

fn refused() -> Result<(), InstallFailure> {
    Err(InstallFailure {
        error_type: "&str",
        message: "the Store refused".to_owned(),
    })
}

#[test]
fn the_store_and_installed_builds_and_a_demo_asked_for_it_check_for_updates() {
    let launch = |arguments: &[&str]| {
        Launch::parse(arguments.iter().map(|argument| argument.to_string())).unwrap()
    };
    for (arguments, channel, expected) in [
        (&[][..], Channel::Store, Some(UpdateSource::Store)),
        (&[], Channel::Direct, Some(UpdateSource::Direct)),
        (&[], Channel::Zip, None),
        (&["--demo"], Channel::Direct, None),
        (&["--demo"], Channel::Store, None),
        (
            &["--demo", "--demo-update"],
            Channel::Zip,
            Some(UpdateSource::Demo),
        ),
        (
            &["--demo-update", "--demo", "late"],
            Channel::Store,
            Some(UpdateSource::Demo),
        ),
        (&["--demo-update"], Channel::Zip, None),
    ] {
        assert_eq!(
            UpdateSource::of(&launch(arguments), channel),
            expected,
            "{arguments:?} on {channel:?}"
        );
    }
}

#[test]
fn nothing_is_offered_before_a_check_finds_an_update() {
    let mut updates = Updates::default();

    assert_eq!(updates.callout(), None);
    assert!(updates.install().is_none());
}

#[test]
fn an_update_the_daily_check_finds_is_offered_and_the_check_recorded() {
    let store = FakeStore::answering(Ok(()));
    let settings = Settings::new(true);
    let mut updates = Updates::default();

    updates.found(checked(&store, &settings).expect("the store has an update"));

    assert_eq!(updates.callout(), Some(UpdateCallout::Offered));
    assert_eq!(settings.0.get().last_checked_at, Some(now()));
    assert_eq!(store.installs.get(), 0);
}

#[test]
fn turning_update_checks_off_offers_nothing() {
    let store = FakeStore::answering(Ok(()));
    let settings = Settings::new(false);

    assert!(checked(&store, &settings).is_none());
    assert_eq!(settings.0.get().last_checked_at, None);
}

#[test]
fn update_hands_the_offer_to_the_store_once_until_it_answers() {
    let store = FakeStore::answering(Ok(()));
    let mut updates = offered(&store);

    let installing = updates.install().expect("an offer installs");

    assert_eq!(updates.callout(), Some(UpdateCallout::Installing));
    assert!(updates.install().is_none(), "a second click installs again");
    updates.found(checked(&store, &Settings::new(true)).unwrap());
    assert_eq!(updates.callout(), Some(UpdateCallout::Installing));
    assert_eq!(ready(installing), Ok(()));
    assert_eq!(store.installs.get(), 1);
}

#[test]
fn an_install_that_completes_or_that_the_user_declines_ends_the_offer() {
    let store = FakeStore::answering(Ok(()));
    let mut updates = offered(&store);

    let installed = ready(updates.install().unwrap());
    let quits = updates.installed(&installed);

    assert!(!quits, "the Store installs while Bientôt runs");
    assert_eq!(updates.callout(), None);
    assert!(updates.install().is_none());
}

#[test]
fn the_installed_builds_update_quits_worktime_once_its_updater_started() {
    let mut updates = Updates::default();
    updates.found(Offer::new(DemoOffer).restarting());
    assert!(updates.restarts());

    let installed = ready(updates.install().unwrap());

    assert!(updates.installed(&installed));
    assert_eq!(updates.callout(), None);
}

#[test]
fn the_installed_builds_failed_update_keeps_worktime_open_and_its_restart_button() {
    let store = FakeStore::answering(Err("the updater did not start"));
    let mut updates = Updates::default();
    updates.found(checked(&store, &Settings::new(true)).unwrap().restarting());

    let installed = ready(updates.install().unwrap());

    assert!(!updates.installed(&installed));
    assert_eq!(updates.callout(), Some(UpdateCallout::Failed));
    assert!(updates.restarts());
}

#[test]
fn the_callout_says_what_installing_does() {
    let cases = [
        (
            UpdateCallout::Offered,
            false,
            Key::UpdateNoticeMessage,
            Key::UpdateAvailableDialogPrimaryButtonText,
        ),
        (
            UpdateCallout::Installing,
            false,
            Key::UpdateNoticeMessage,
            Key::UpdateInstalling,
        ),
        (
            UpdateCallout::Failed,
            false,
            Key::UpdateFailedMessage,
            Key::UpdateRetry,
        ),
        (
            UpdateCallout::Offered,
            true,
            Key::UpdateReadyMessage,
            Key::UpdateRestartActionContent,
        ),
        (
            UpdateCallout::Installing,
            true,
            Key::UpdateRestartingMessage,
            Key::UpdateRestarting,
        ),
        (
            UpdateCallout::Failed,
            true,
            Key::UpdateRestartFailedMessage,
            Key::UpdateRestartActionContent,
        ),
    ];
    for (callout, restarts, message, label) in cases {
        assert_eq!(
            callout.text(restarts),
            (message, label),
            "{callout:?}, restarts: {restarts}"
        );
    }
}

#[test]
fn a_failed_install_says_so_and_update_tries_again() {
    let store = FakeStore::answering(Err("the Store refused"));
    let mut updates = offered(&store);

    let installed = ready(updates.install().unwrap());
    updates.installed(&installed);

    assert_eq!(installed, refused());
    assert_eq!(updates.callout(), Some(UpdateCallout::Failed));
    let retried = ready(updates.install().expect("a failed offer installs again"));
    assert_eq!(retried, refused());
    assert_eq!(store.installs.get(), 2);
}

#[test]
fn a_new_offer_replaces_a_failed_one() {
    let store = FakeStore::answering(Err("the Store refused"));
    let mut updates = offered(&store);
    let installed = ready(updates.install().unwrap());
    updates.installed(&installed);

    updates.found(checked(&store, &Settings::new(true)).unwrap());

    assert_eq!(updates.callout(), Some(UpdateCallout::Offered));
}

#[test]
fn an_answer_without_an_install_under_way_changes_nothing() {
    let store = FakeStore::answering(Ok(()));
    let mut updates = offered(&store);

    updates.installed(&refused());

    assert_eq!(updates.callout(), Some(UpdateCallout::Offered));
}

#[test]
fn the_demo_store_offers_an_update_that_installs_nothing() {
    let mut updates = Updates::default();

    updates.found(checked(DemoStore, &Settings::new(true)).expect("the demo has an update"));
    assert_eq!(updates.callout(), Some(UpdateCallout::Offered));
    let installed = ready(updates.install().unwrap());
    updates.installed(&installed);

    assert_eq!(installed, Ok(()));
    assert_eq!(updates.callout(), None);
}
