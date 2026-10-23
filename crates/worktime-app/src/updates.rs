//! The update the flyout offers, ported from `MainWindow.OfferUpdate`, the update checks of
//! `App.xaml.cs` and `Services/Demo/DemoStoreUpdatePort.cs`. The checks are
//! `worktime_windows::updates::StoreUpdateService`'s: at startup, then every
//! [`RECHECK_INTERVAL`], asking the Store or the feed of the installed build at most once a day
//! and only while « Rechercher les mises à jour » is on. « Mettre à jour » hands the update to the
//! Store at once, without the .NET dialog; the Callout says what the dialog said. The installed
//! build offers an update it already downloaded, which « Redémarrer WorkTime » installs.

#[cfg(all(windows, feature = "updates"))]
pub mod direct;

use std::convert::Infallible;
use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use gpui::{App, AsyncApp, Global};
use jiff::Timestamp;
use worktime_core::diagnostics::{DiagnosticLogLevel, DiagnosticValue};
use worktime_domain::configuration::UpdateSettings;
use worktime_domain::time::SystemClock;
use worktime_domain::usage_ping::Channel;
use worktime_store::settings::SaveSettingsError;
use worktime_windows::updates::{
    RECHECK_INTERVAL, StoreUpdateService, UpdateCheckError, UpdateCheckSettings, UpdateOffer,
    UpdateStore,
};

use crate::flyout::InstallUpdate;
use crate::host::Today;
use crate::launch::Launch;
use crate::strings::Key;

/// Has a demo offer an update, as the .NET app's `--demo-update`.
pub const DEMO_ARGUMENT: &str = "--demo-update";

/// The store a launch asks for updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateSource {
    /// The Microsoft Store, for the MSIX build on the user's data.
    Store,
    /// The feed of the build Velopack's `Setup.exe` installed.
    Direct,
    /// [`DemoStore`], for a demo launched with `--demo-update`.
    Demo,
}

impl UpdateSource {
    /// `None` for the ZIP build, which has no store, and for a demo without `--demo-update`.
    pub fn of(launch: &Launch, channel: Channel) -> Option<Self> {
        match (launch.demo, channel) {
            (Some(_), _) => launch.demo_update.then_some(Self::Demo),
            (None, Channel::Store) => Some(Self::Store),
            (None, Channel::Direct) => Some(Self::Direct),
            (None, Channel::Zip) => None,
        }
    }
}

/// What the flyout's update Callout shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateCallout {
    Offered,
    /// Handed to the Store, or to the updater of the installed build: the button waits for it.
    Installing,
    /// The install failed; the button tries again.
    Failed,
}

impl UpdateCallout {
    /// The Callout's message and button label. The installed build installs while WorkTime is
    /// closed, and at the next start when that failed.
    pub fn text(self, restarts: bool) -> (Key, Key) {
        match (self, restarts) {
            (Self::Offered, false) => (
                Key::UpdateNoticeMessage,
                Key::UpdateAvailableDialogPrimaryButtonText,
            ),
            (Self::Installing, false) => (Key::UpdateNoticeMessage, Key::UpdateInstalling),
            (Self::Failed, false) => (Key::UpdateFailedMessage, Key::UpdateRetry),
            (Self::Offered, true) => (Key::UpdateReadyMessage, Key::UpdateRestartActionContent),
            (Self::Installing, true) => (Key::UpdateRestartingMessage, Key::UpdateRestarting),
            (Self::Failed, true) => (
                Key::UpdateRestartFailedMessage,
                Key::UpdateRestartActionContent,
            ),
        }
    }
}

/// Why an install failed, as the diagnostic log records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallFailure {
    pub error_type: &'static str,
    pub message: String,
}

impl InstallFailure {
    fn of<E: Display>(error: &E) -> Self {
        Self {
            error_type: std::any::type_name::<E>(),
            message: error.to_string(),
        }
    }
}

type Installing = Pin<Box<dyn Future<Output = Result<(), InstallFailure>>>>;

/// An update of any store, installed as its own [`UpdateOffer`] does.
pub struct Offer {
    install: Box<dyn Fn() -> Installing>,
    /// [`UpdateOffer::RESTARTS`].
    restarts: bool,
}

impl Offer {
    pub fn new<O>(offer: O) -> Self
    where
        O: UpdateOffer + 'static,
        O::Error: Display,
    {
        let offer = Rc::new(offer);
        let install = Box::new(move || -> Installing {
            let offer = Rc::clone(&offer);
            Box::pin(async move {
                offer
                    .download_and_install()
                    .await
                    .map_err(|error| InstallFailure::of(&error))
            })
        });
        Self {
            install,
            restarts: O::RESTARTS,
        }
    }

    /// The same offer, installed as the installed build's is: for demos and screenshots.
    pub fn restarting(self) -> Self {
        Self {
            restarts: true,
            ..self
        }
    }
}

/// The update on offer, from its discovery until the Store installed it.
#[derive(Default)]
pub enum Updates {
    /// Nothing found yet, or the last offer was installed or declined in Windows' prompt: the
    /// next daily check offers it again if it is still there.
    #[default]
    None,
    Offered(Rc<Offer>),
    Installing(Rc<Offer>),
    Failed(Rc<Offer>),
}

impl Global for Updates {}

impl Updates {
    /// Offers what a check `found`. An install under way goes on.
    pub fn found(&mut self, found: Offer) {
        if !matches!(self, Self::Installing(_)) {
            *self = Self::Offered(Rc::new(found));
        }
    }

    /// « Mettre à jour » or « Réessayer »: hands the offer to the Store. `None` while nothing can
    /// be installed: no offer, or an install already under way.
    pub fn install(&mut self) -> Option<impl Future<Output = Result<(), InstallFailure>> + use<>> {
        let (Self::Offered(offer) | Self::Failed(offer)) = self else {
            return None;
        };
        let offer = Rc::clone(offer);
        let installing = (offer.install)();
        *self = Self::Installing(offer);
        Some(installing)
    }

    /// What the Store or the updater answered. A success, which includes the user declining
    /// Windows' prompt, ends the offer. `true` when WorkTime must now quit for the updater.
    pub fn installed(&mut self, result: &Result<(), InstallFailure>) -> bool {
        let Self::Installing(offer) = self else {
            return false;
        };
        let restarts = offer.restarts;
        *self = match result {
            Ok(()) => Self::None,
            Err(_) => Self::Failed(Rc::clone(offer)),
        };
        result.is_ok() && restarts
    }

    /// Whether the offer installs as the installed build's does: see [`UpdateCallout::text`].
    pub fn restarts(&self) -> bool {
        match self {
            Self::None => false,
            Self::Offered(offer) | Self::Installing(offer) | Self::Failed(offer) => offer.restarts,
        }
    }

    pub fn callout(&self) -> Option<UpdateCallout> {
        match self {
            Self::None => None,
            Self::Offered(_) => Some(UpdateCallout::Offered),
            Self::Installing(_) => Some(UpdateCallout::Installing),
            Self::Failed(_) => Some(UpdateCallout::Failed),
        }
    }
}

/// The Store of a demo: it always offers an update, which installs nothing.
pub struct DemoStore;

pub struct DemoOffer;

impl UpdateStore for DemoStore {
    type Offer = DemoOffer;
    type Error = Infallible;

    fn find_update(&self) -> impl Future<Output = Result<Option<DemoOffer>, Infallible>> {
        std::future::ready(Ok(Some(DemoOffer)))
    }
}

impl UpdateOffer for DemoOffer {
    type Error = Infallible;

    fn download_and_install(&self) -> impl Future<Output = Result<(), Infallible>> {
        std::future::ready(Ok(()))
    }
}

/// What the diagnostic log keeps of a Store failure: its HRESULT, the only detail that tells
/// Store failures apart.
pub trait StoreError: Display {
    fn hresult(&self) -> Option<i32>;
}

impl StoreError for Infallible {
    fn hresult(&self) -> Option<i32> {
        match *self {}
    }
}

#[cfg(windows)]
impl StoreError for windows::core::Error {
    fn hresult(&self) -> Option<i32> {
        Some(self.code().0)
    }
}

/// Checks `source` for updates, if any: see [`install`].
pub fn start(source: Option<UpdateSource>, cx: &mut App) {
    let Some(source) = source else {
        return;
    };
    match source {
        UpdateSource::Demo => install(DemoStore, cx),
        #[cfg(windows)]
        UpdateSource::Store => install_store(cx),
        #[cfg(all(windows, feature = "updates"))]
        UpdateSource::Direct => {
            if let Some(feed) = direct::DirectUpdates::installed() {
                install(feed, cx);
            }
        }
        #[cfg(not(windows))]
        UpdateSource::Store => {}
        #[cfg(not(all(windows, feature = "updates")))]
        UpdateSource::Direct => {}
    }
}

/// Checks the Microsoft Store, whose prompts the notification-area icon's window owns: the
/// flyout closes when Windows' prompt takes the focus. A build without package identity has
/// no Store to ask.
#[cfg(windows)]
fn install_store(cx: &mut App) {
    let Some(owner) = crate::tray::window_handle(cx) else {
        return;
    };
    match worktime_windows::updates::StoreUpdates::for_window(owner) {
        Ok(Some(store)) => install(store, cx),
        Ok(None) => {}
        Err(error) => {
            let message = error.to_string();
            log::warn!("update_check.unavailable: {message}");
            if let Some(host) = cx.global_mut::<Today>().live() {
                host.log(
                    DiagnosticLogLevel::Warning,
                    "update_check.unavailable",
                    &[
                        ("exceptionType", std::any::type_name_of_val(&error).into()),
                        ("message", message.as_str().into()),
                    ],
                );
            }
        }
    }
}

/// The day's settings, read and saved by the checks between their awaits.
struct TodaySettings(AsyncApp);

impl UpdateCheckSettings for TodaySettings {
    type Error = SaveSettingsError;

    fn updates(&self) -> UpdateSettings {
        self.0.update(|cx| cx.global::<Today>().settings().updates)
    }

    fn record_check(&mut self, at: Timestamp) -> Result<(), SaveSettingsError> {
        self.0
            .update(|cx| cx.global_mut::<Today>().record_update_check(at))
    }
}

/// Checks `store` for updates at startup, then every [`RECHECK_INTERVAL`], and offers what it
/// finds in the flyout. Call it once, after the day is installed.
fn install<U>(store: U, cx: &mut App)
where
    U: UpdateStore + 'static,
    U::Offer: UpdateOffer + 'static,
    U::Error: StoreError,
    <U::Offer as UpdateOffer>::Error: Display,
{
    cx.set_global(Updates::default());
    cx.on_action(|_: &InstallUpdate, cx| install_offer(cx));
    cx.spawn(async move |cx| {
        let settings = TodaySettings(cx.clone());
        let mut service = StoreUpdateService::new(Some(store), settings, SystemClock);
        loop {
            match service.find_due_update().await {
                Ok(Some(offer)) => {
                    cx.update_global::<Updates, _>(|updates, _| updates.found(Offer::new(offer)))
                }
                Ok(None) => {}
                Err(error) => cx.update(|cx| log_check_failure(&error, cx)),
            }
            cx.background_executor().timer(RECHECK_INTERVAL).await;
        }
    })
    .detach();
}

/// « Mettre à jour » or « Redémarrer WorkTime »: the flyout shows the install under way, then
/// its failure if any. WorkTime quits for the updater of the installed build.
fn install_offer(cx: &mut App) {
    let Some(installing) = cx.global_mut::<Updates>().install() else {
        return;
    };
    cx.spawn(async move |cx| {
        let installed = installing.await;
        cx.update(|cx| {
            if let Err(failure) = &installed {
                log_install_failure(failure, cx);
            }
            if cx.global_mut::<Updates>().installed(&installed) {
                cx.quit();
            }
        });
    })
    .detach();
}

/// Logs `update_check.failed`, as `StoreUpdateService.cs` reports it, with the HRESULT.
fn log_check_failure<E: StoreError>(error: &UpdateCheckError<SaveSettingsError, E>, cx: &mut App) {
    let (error_type, hresult) = match error {
        UpdateCheckError::Settings(_) => (std::any::type_name::<SaveSettingsError>(), None),
        UpdateCheckError::Store(store) => (std::any::type_name::<E>(), store.hresult()),
    };
    let message = error.to_string();
    log::warn!("update_check.failed: {message}");
    if let Some(host) = cx.global_mut::<Today>().live() {
        host.log(
            DiagnosticLogLevel::Warning,
            "update_check.failed",
            &[
                ("exceptionType", error_type.into()),
                ("message", message.as_str().into()),
                ("hresult", hresult.map_or(DiagnosticValue::Null, Into::into)),
            ],
        );
    }
}

/// Logs `window_command.failed`, as `MainWindow.OnUpdateNoticeClick` reports a failed install.
fn log_install_failure(failure: &InstallFailure, cx: &mut App) {
    log::error!("window_command.failed: {}", failure.message);
    if let Some(host) = cx.global_mut::<Today>().live() {
        host.log(
            DiagnosticLogLevel::Error,
            "window_command.failed",
            &[
                ("exceptionType", failure.error_type.into()),
                ("message", failure.message.as_str().into()),
            ],
        );
    }
}

#[cfg(test)]
mod tests;
