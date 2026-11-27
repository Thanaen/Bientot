//! Updates of the build that Velopack installed (D-016, revised 2026-10-19): `Setup.exe`'s on
//! Windows, the AppImage on Linux. A check reads the feed at [`FEED_URL`] and downloads a newer
//! package at once; `VelopackApp`, the first thing `main` runs, installs it at the next start, and
//! « Redémarrer WorkTime » installs it now. The backends compile this module with `updates`.
//!
//! The feed is read with the system TLS and no query string: Velopack's own `HttpSource` would
//! send an identifier of the installation with every check.

use std::fs::File;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use bientot_platform::updates::{UpdateOffer, UpdateSource};
use futures::channel::oneshot;
use ureq::Agent;
use velopack::bundle::Manifest;
use velopack::{UpdateCheck, UpdateManager, VelopackAsset, VelopackAssetFeed};

use crate::platform;

/// Where `scripts/release-direct.sh` publishes the packages and their `releases.<channel>.json`:
/// `win`, `linux` or `linux-arm64`, which Velopack reads from the installed package.
pub const FEED_URL: &str = "https://worktime.thanaen.dev/updates/";

/// Long enough to download a full package on a slow connection.
const TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The feed of the installed build.
pub struct DirectUpdates(UpdateManager);

impl DirectUpdates {
    /// `None` when Velopack did not install the running exe, as for `cargo run`.
    pub fn installed() -> Option<Self> {
        match UpdateManager::new(Feed, None, None) {
            Ok(manager) => Some(Self(manager)),
            Err(error) => {
                log::info!("update_check.not_installed: {error}");
                None
            }
        }
    }
}

impl UpdateSource for DirectUpdates {
    type Offer = DirectOffer;
    type Error = velopack::Error;

    /// Downloads the update it finds, so that the offer is ready to install.
    async fn find_update(&self) -> Result<Option<DirectOffer>, velopack::Error> {
        let manager = self.0.clone();
        off_thread(move || {
            let UpdateCheck::UpdateAvailable(update) = manager.check_for_updates()? else {
                return Ok(None);
            };
            manager.download_updates(&update, None)?;
            Ok(Some(DirectOffer {
                manager,
                package: update.TargetFullRelease,
            }))
        })
        .await
    }
}

/// A downloaded package.
pub struct DirectOffer {
    manager: UpdateManager,
    package: VelopackAsset,
}

impl UpdateOffer for DirectOffer {
    type Error = velopack::Error;
    const RESTARTS: bool = true;

    /// Starts Velopack's updater, which waits for WorkTime to quit, installs the package and
    /// starts WorkTime again with the flyout open: the caller quits once this succeeds.
    async fn download_and_install(&self) -> Result<(), velopack::Error> {
        self.manager.wait_exit_then_apply_updates(
            &self.package,
            true,
            true,
            crate::launch::restart_arguments(false),
        )
    }
}

/// Runs the blocking `work` on a thread of its own, so that neither the UI nor GPUI's executor
/// waits for the network.
async fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, velopack::Error> + Send + 'static,
) -> Result<T, velopack::Error> {
    let (sender, receiver) = oneshot::channel();
    thread::spawn(move || {
        let _ = sender.send(work());
    });
    receiver
        .await
        .unwrap_or_else(|_| Err(velopack::Error::Other("the update check stopped".into())))
}

/// The feed at [`FEED_URL`], read without a query string.
struct Feed;

impl velopack::sources::UpdateSource for Feed {
    fn get_release_feed(
        &self,
        channel: &str,
        _app: &Manifest,
        _staged_user_id: &str,
    ) -> Result<VelopackAssetFeed, velopack::Error> {
        let json = agent()
            .get(format!("{FEED_URL}releases.{channel}.json"))
            .call()?
            .body_mut()
            .read_to_string()?;
        Ok(serde_json::from_str(&json)?)
    }

    fn download_release_entry(
        &self,
        asset: &VelopackAsset,
        local_file: &Path,
        _progress: Option<Sender<i16>>,
    ) -> Result<(), velopack::Error> {
        let mut response = agent()
            .get(format!("{FEED_URL}{}", asset.FileName))
            .call()?;
        let mut file = File::create(local_file)?;
        std::io::copy(&mut response.body_mut().as_reader(), &mut file)?;
        Ok(())
    }
}

/// An agent that trusts the system's root certificates and fails on an error status.
fn agent() -> Agent {
    Agent::config_builder()
        .tls_config(platform::system_tls())
        .https_only(true)
        .timeout_global(Some(TIMEOUT))
        .build()
        .into()
}

/// Runs `hooks`, Velopack's startup with the backend's callbacks, before anything else. A
/// downloaded package waits for [`install_pending`]: Velopack's own install at startup would
/// have its updater kill the WorkTime already running. `false` when Velopack just started
/// WorkTime again after an install, which must not install again whatever happened.
pub fn run_hooks(hooks: velopack::VelopackApp<'_>) -> bool {
    let restarted = std::env::var_os(RESTARTED).is_some();
    hooks.set_auto_apply_on_startup(false).run();
    !restarted
}

/// What Velopack's updater sets on the WorkTime it starts after an install.
const RESTARTED: &str = "VELOPACK_RESTART";

/// Installs the package a check downloaded, if any, and ends the process: the updater starts
/// WorkTime again as this launch, in the notification area when the system started it. Call it
/// once this launch is the only WorkTime, so that nothing is lost when the updater stops the
/// processes of the installed build.
pub fn install_pending() {
    let Some(DirectUpdates(manager)) = DirectUpdates::installed() else {
        return;
    };
    let Some(package) = manager.get_update_pending_restart() else {
        return;
    };
    match manager.wait_exit_then_apply_updates(
        &package,
        true,
        true,
        crate::launch::restart_arguments(true),
    ) {
        Ok(()) => std::process::exit(0),
        Err(error) => log::warn!("update_install.failed: {error}"),
    }
}
