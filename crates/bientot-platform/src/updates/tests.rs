use std::cell::{Cell, RefCell};
use std::fs;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use bientot_domain::configuration::Settings;
use bientot_domain::time::FrozenClock;
use bientot_store::settings::{SaveSettingsError, SettingsSource, SettingsStore};
use tempfile::TempDir;

use super::*;

fn now() -> Timestamp {
    "2026-10-12T07:30:00Z".parse().unwrap()
}

fn hours(hours: i64) -> SignedDuration {
    SignedDuration::from_hours(hours)
}

fn updates(check_for_updates: bool, last_checked_at: Option<Timestamp>) -> UpdateSettings {
    UpdateSettings {
        check_for_updates,
        last_checked_at,
    }
}

#[test]
fn a_store_never_asked_is_due() {
    assert!(is_due(&UpdateSettings::default(), now()));
}

#[test]
fn a_check_is_due_once_a_day_has_passed_since_the_last_one() {
    for (since_last_check, expected) in [
        (SignedDuration::ZERO, false),
        (hours(24) - SignedDuration::from_secs(1), false),
        (hours(24), true),
        (hours(12 * 24 + 3), true),
    ] {
        assert_eq!(
            is_due(&updates(true, Some(now() - since_last_check)), now()),
            expected,
            "{since_last_check:?} since the last check"
        );
    }
}

#[test]
fn a_disabled_check_is_never_due() {
    assert!(!is_due(&updates(false, None), now()));
    assert!(!is_due(
        &updates(false, Some(now() - hours(30 * 24))),
        now()
    ));
}

#[test]
fn a_last_check_after_now_does_not_block_checks_when_the_clock_moved_back() {
    for ahead_of_now in [SignedDuration::from_secs(1), hours(48)] {
        assert!(is_due(&updates(true, Some(now() + ahead_of_now)), now()));
    }
}

/// The host's settings: what is current, saved to a real `settings.json`.
struct SavedSettings {
    _directory: TempDir,
    store: SettingsStore,
    current: RefCell<Settings>,
}

impl SavedSettings {
    fn new(updates: UpdateSettings) -> Self {
        let directory = TempDir::new().unwrap();
        let store = SettingsStore::new(directory.path().join("settings.json")).unwrap();
        Self::at(directory, store, updates)
    }

    fn at(directory: TempDir, store: SettingsStore, updates: UpdateSettings) -> Self {
        Self {
            _directory: directory,
            store,
            current: RefCell::new(Settings {
                updates,
                ..Settings::default()
            }),
        }
    }

    /// What the settings page submits.
    fn edit(&self, edit: impl FnOnce(&mut UpdateSettings)) {
        let mut next = self.current.borrow().clone();
        edit(&mut next.updates);
        self.store.save(&next).unwrap();
        self.current.replace(next);
    }

    /// `None` while nothing was saved.
    fn saved(&self) -> Option<UpdateSettings> {
        let loaded = self.store.load().unwrap();
        (loaded.source == SettingsSource::File).then_some(loaded.settings.updates)
    }
}

impl UpdateCheckSettings for &SavedSettings {
    type Error = SaveSettingsError;

    fn updates(&self) -> UpdateSettings {
        self.current.borrow().updates
    }

    fn record_check(&mut self, at: Timestamp) -> Result<(), SaveSettingsError> {
        let mut next = self.current.borrow().clone();
        next.updates.last_checked_at = Some(at);
        self.store.save(&next)?;
        self.current.replace(next);
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Offer;

#[derive(Debug, PartialEq, Eq)]
struct StoreFailure;

struct FakeStore {
    answer: Result<bool, ()>,
    never_answers: bool,
    calls: Cell<usize>,
}

impl FakeStore {
    fn offering() -> Self {
        Self::answering(Ok(true))
    }

    fn answering(answer: Result<bool, ()>) -> Self {
        Self {
            answer,
            never_answers: false,
            calls: Cell::new(0),
        }
    }
}

impl UpdateStore for &FakeStore {
    type Offer = Offer;
    type Error = StoreFailure;

    fn find_update(&self) -> impl Future<Output = Result<Option<Offer>, StoreFailure>> {
        self.calls.set(self.calls.get() + 1);
        let (answer, never_answers) = (self.answer, self.never_answers);
        async move {
            if never_answers {
                std::future::pending::<()>().await;
            }
            answer
                .map(|found| found.then_some(Offer))
                .map_err(|()| StoreFailure)
        }
    }
}

type Service<'a> = StoreUpdateService<&'a FakeStore, &'a SavedSettings, FrozenClock>;

fn service<'a>(store: Option<&'a FakeStore>, settings: &'a SavedSettings) -> Service<'a> {
    StoreUpdateService::new(store, settings, FrozenClock::new(now()))
}

type Checked = Result<Option<Offer>, UpdateCheckError<SaveSettingsError, StoreFailure>>;

/// Runs a check whose store answers at once, as the fakes do.
fn check(service: &mut Service<'_>) -> Checked {
    match pin!(service.find_due_update()).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(checked) => checked,
        Poll::Pending => panic!("the check is still waiting for the store"),
    }
}

#[test]
fn an_update_found_on_the_first_check_is_offered_and_the_check_is_recorded() {
    let store = FakeStore::offering();
    let settings = SavedSettings::new(UpdateSettings::default());

    let offer = check(&mut service(Some(&store), &settings));

    assert_eq!(offer.unwrap(), Some(Offer));
    assert_eq!(store.calls.get(), 1);
    assert_eq!(settings.saved(), Some(updates(true, Some(now()))));
}

#[test]
fn no_update_offers_nothing_and_records_the_check() {
    let store = FakeStore::answering(Ok(false));
    let settings = SavedSettings::new(UpdateSettings::default());

    let offer = check(&mut service(Some(&store), &settings));

    assert_eq!(offer.unwrap(), None);
    assert_eq!(store.calls.get(), 1);
    assert_eq!(settings.saved(), Some(updates(true, Some(now()))));
}

#[test]
fn a_store_failure_is_returned_and_still_counts_as_the_daily_check() {
    let store = FakeStore::answering(Err(()));
    let settings = SavedSettings::new(UpdateSettings::default());
    let mut service = service(Some(&store), &settings);

    let first = check(&mut service);
    let retry = check(&mut service);

    assert!(matches!(first, Err(UpdateCheckError::Store(StoreFailure))));
    assert_eq!(retry.unwrap(), None);
    assert_eq!(store.calls.get(), 1);
    assert_eq!(settings.saved(), Some(updates(true, Some(now()))));
}

#[test]
fn a_second_check_on_the_same_day_does_not_ask_the_store_again() {
    let store = FakeStore::offering();
    let settings = SavedSettings::new(UpdateSettings::default());
    let mut service = service(Some(&store), &settings);

    check(&mut service).unwrap();
    let second = check(&mut service);

    assert_eq!(second.unwrap(), None);
    assert_eq!(store.calls.get(), 1);
}

#[test]
fn a_check_twenty_three_hours_ago_is_not_repeated() {
    let store = FakeStore::offering();
    let settings = SavedSettings::new(updates(true, Some(now() - hours(23))));

    let offer = check(&mut service(Some(&store), &settings));

    assert_eq!(offer.unwrap(), None);
    assert_eq!(store.calls.get(), 0);
    assert_eq!(settings.saved(), None);
}

#[test]
fn a_check_a_day_ago_asks_the_store_again() {
    let store = FakeStore::offering();
    let settings = SavedSettings::new(updates(true, Some(now() - hours(24))));

    let offer = check(&mut service(Some(&store), &settings));

    assert_eq!(offer.unwrap(), Some(Offer));
    assert_eq!(store.calls.get(), 1);
    assert_eq!(settings.saved(), Some(updates(true, Some(now()))));
}

#[test]
fn disabled_checks_never_ask_the_store() {
    let store = FakeStore::offering();
    let settings = SavedSettings::new(updates(false, None));

    let offer = check(&mut service(Some(&store), &settings));

    assert_eq!(offer.unwrap(), None);
    assert_eq!(store.calls.get(), 0);
    assert_eq!(settings.saved(), None);
}

#[test]
fn turning_checks_off_in_settings_stops_the_next_check_without_a_restart() {
    let last_checked = now() - hours(48);
    let store = FakeStore::offering();
    let settings = SavedSettings::new(updates(true, Some(last_checked)));
    let mut service = service(Some(&store), &settings);

    settings.edit(|updates| updates.check_for_updates = false);
    let while_off = check(&mut service);

    assert_eq!(while_off.unwrap(), None);
    assert_eq!(store.calls.get(), 0);
    assert_eq!(settings.saved(), Some(updates(false, Some(last_checked))));

    settings.edit(|updates| updates.check_for_updates = true);
    let after_turning_on = check(&mut service);

    assert_eq!(after_turning_on.unwrap(), Some(Offer));
    assert_eq!(store.calls.get(), 1);
    assert_eq!(settings.saved(), Some(updates(true, Some(now()))));
}

#[test]
fn the_portable_build_without_package_identity_never_checks_nor_records_a_check() {
    let settings = SavedSettings::new(UpdateSettings::default());

    let offer = check(&mut service(None, &settings));

    assert_eq!(offer.unwrap(), None);
    assert_eq!(settings.saved(), None);
}

#[test]
fn an_abandoned_check_is_no_failure_and_still_counts_as_the_daily_check() {
    let store = FakeStore {
        never_answers: true,
        ..FakeStore::offering()
    };
    let settings = SavedSettings::new(UpdateSettings::default());
    let mut service = service(Some(&store), &settings);

    let poll = pin!(service.find_due_update()).poll(&mut Context::from_waker(Waker::noop()));

    assert!(poll.is_pending());
    assert_eq!(store.calls.get(), 1);
    assert_eq!(settings.saved(), Some(updates(true, Some(now()))));
}

#[test]
fn settings_that_cannot_be_saved_are_returned_and_the_store_is_not_asked() {
    let store = FakeStore::offering();
    let directory = TempDir::new().unwrap();
    fs::write(directory.path().join("blocked"), "").unwrap();
    let blocked =
        SettingsStore::new(directory.path().join("blocked").join("settings.json")).unwrap();
    let settings = SavedSettings::at(directory, blocked, UpdateSettings::default());

    let offer = check(&mut service(Some(&store), &settings));

    assert!(matches!(
        offer,
        Err(UpdateCheckError::Settings(SaveSettingsError::Io(_)))
    ));
    assert_eq!(store.calls.get(), 0);
    assert_eq!(settings.current.borrow().updates, UpdateSettings::default());
}
