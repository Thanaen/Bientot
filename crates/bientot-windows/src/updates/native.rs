//! The WinRT calls of the .NET `StoreContextUpdatePort`. A desktop app's `StoreContext` needs an
//! owner window and must be called on that window's thread; otherwise the Store calls fail with
//! `ERROR_INVALID_WINDOW_HANDLE`. Each call starts its operation when it is made, so call it on
//! the UI thread and await the future there.
//!
//! The Store closes the app to install an update. A packaged desktop app comes back only if it
//! called `RegisterApplicationRestart` before: the offer registers right before it hands the
//! update to the Store, and takes the registration back when nothing was installed, so that a
//! sign-out with Windows' « restart my apps » setting does not restart it after a declined update.

#![allow(unsafe_code)]

use std::future::Future;
use std::num::NonZeroIsize;
use std::pin::Pin;

use bientot_platform::updates::{UpdateOffer, UpdateSource};
use windows::Services::Store::{StoreContext, StorePackageUpdateState};
use windows::Win32::Foundation::{E_FAIL, HWND};
use windows::Win32::UI::Shell::IInitializeWithWindow;
use windows::core::{Error, HSTRING, Interface};
use windows_sys::Win32::System::Recovery::{
    RESTART_NO_CRASH, RESTART_NO_HANG, RESTART_NO_REBOOT, RegisterApplicationRestart,
    UnregisterApplicationRestart,
};

/// The Store, for the MSIX build.
pub struct StoreUpdates {
    context: StoreContext,
    /// The command line of the restart after an install, see [`super::restart_command_line`].
    restart: HSTRING,
}

impl StoreUpdates {
    /// The Store with `owner` as the window of its prompts, or `None` without package identity.
    /// Once an update installs, the app starts again with `restart_arguments`.
    pub fn for_window(
        owner: NonZeroIsize,
        restart_arguments: &[String],
    ) -> windows::core::Result<Option<Self>> {
        if !crate::has_package_identity() {
            return Ok(None);
        }
        let context = StoreContext::GetDefault()?;
        let initialize = context.cast::<IInitializeWithWindow>()?;
        // SAFETY: `initialize` is a live interface of `context`. `Initialize` only stores the
        // handle as the owner of the Store's prompts; a handle that is not a window makes the
        // Store calls fail with an error, it is never dereferenced here.
        unsafe { initialize.Initialize(HWND(owner.get() as *mut _)) }?;
        Ok(Some(Self {
            context,
            restart: HSTRING::from(super::restart_command_line(restart_arguments)),
        }))
    }
}

impl UpdateSource for StoreUpdates {
    type Offer = StoreUpdateOffer;
    type Error = Error;

    fn find_update(&self) -> impl Future<Output = Result<Option<StoreUpdateOffer>, Error>> {
        let operation = self.context.GetAppAndOptionalStorePackageUpdatesAsync();
        let context = self.context.clone();
        let restart = self.restart.clone();
        async move {
            let updates = operation?.await?;
            if updates.Size()? == 0 {
                return Ok(None);
            }
            let install = move || -> Installing {
                let operation = context.RequestDownloadAndInstallStorePackageUpdatesAsync(&updates);
                Box::pin(async move { operation?.await?.OverallState() })
            };
            Ok(Some(StoreUpdateOffer {
                install: Box::new(install),
                restart,
            }))
        }
    }
}

type Installing = Pin<Box<dyn Future<Output = Result<StorePackageUpdateState, Error>>>>;

/// Updates the Store found for this package.
pub struct StoreUpdateOffer {
    /// Hands the updates found to the Store. A closure, because the `windows` crate does not
    /// export the collection type that holds them.
    install: Box<dyn Fn() -> Installing>,
    restart: HSTRING,
}

impl UpdateOffer for StoreUpdateOffer {
    type Error = Error;

    fn download_and_install(&self) -> impl Future<Output = Result<(), Error>> {
        // Before the Store starts: the install may close the app at any moment after.
        register_restart(&self.restart);
        let installing = (self.install)();
        async move {
            let state = installing.await.inspect_err(|_| unregister_restart())?;
            match state {
                // The deployment usually closes the app before this. Should it still run, the
                // registration stays for the rest of the session: it restarts the app only if
                // Windows closes it to apply the update, not if the user quits it, and a
                // sign-out with « restart my apps » turned on would start it again too.
                StorePackageUpdateState::Completed => Ok(()),
                // The user declined Windows' permission prompt.
                StorePackageUpdateState::Canceled => {
                    unregister_restart();
                    Ok(())
                }
                _ => {
                    unregister_restart();
                    Err(Error::new(
                        E_FAIL,
                        format!("The Store update ended as {}.", state.0),
                    ))
                }
            }
        }
    }
}

/// Asks Windows to start the app again with `command_line` when an install closes it. Not after
/// a crash, a hang or a reboot: the startup task already starts the app at sign-in. A failed
/// registration only leaves the app closed after the install, as before.
fn register_restart(command_line: &HSTRING) {
    // SAFETY: `command_line` is a live NUL-terminated wide string that Windows copies.
    let _ = unsafe {
        RegisterApplicationRestart(
            command_line.as_ptr(),
            RESTART_NO_CRASH | RESTART_NO_HANG | RESTART_NO_REBOOT,
        )
    };
}

fn unregister_restart() {
    // SAFETY: takes no argument; fails harmlessly when nothing is registered.
    let _ = unsafe { UnregisterApplicationRestart() };
}
