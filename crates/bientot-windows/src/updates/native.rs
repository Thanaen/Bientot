//! The WinRT calls of the .NET `StoreContextUpdatePort`. A desktop app's `StoreContext` needs an
//! owner window and must be called on that window's thread; otherwise the Store calls fail with
//! `ERROR_INVALID_WINDOW_HANDLE`. Each call starts its operation when it is made, so call it on
//! the UI thread and await the future there.

#![allow(unsafe_code)]

use std::future::Future;
use std::num::NonZeroIsize;
use std::pin::Pin;

use windows::Services::Store::{StoreContext, StorePackageUpdateState};
use windows::Win32::Foundation::{E_FAIL, HWND};
use windows::Win32::UI::Shell::IInitializeWithWindow;
use windows::core::{Error, Interface};

use super::{UpdateOffer, UpdateStore};

/// The Store, for the MSIX build.
pub struct StoreUpdates {
    context: StoreContext,
}

impl StoreUpdates {
    /// The Store with `owner` as the window of its prompts, or `None` without package identity.
    pub fn for_window(owner: NonZeroIsize) -> windows::core::Result<Option<Self>> {
        if !crate::has_package_identity() {
            return Ok(None);
        }
        let context = StoreContext::GetDefault()?;
        let initialize = context.cast::<IInitializeWithWindow>()?;
        // SAFETY: `initialize` is a live interface of `context`. `Initialize` only stores the
        // handle as the owner of the Store's prompts; a handle that is not a window makes the
        // Store calls fail with an error, it is never dereferenced here.
        unsafe { initialize.Initialize(HWND(owner.get() as *mut _)) }?;
        Ok(Some(Self { context }))
    }
}

impl UpdateStore for StoreUpdates {
    type Offer = StoreUpdateOffer;
    type Error = Error;

    fn find_update(&self) -> impl Future<Output = Result<Option<StoreUpdateOffer>, Error>> {
        let operation = self.context.GetAppAndOptionalStorePackageUpdatesAsync();
        let context = self.context.clone();
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
}

impl UpdateOffer for StoreUpdateOffer {
    type Error = Error;

    fn download_and_install(&self) -> impl Future<Output = Result<(), Error>> {
        let installing = (self.install)();
        async move {
            let state = installing.await?;
            // Canceled means the user declined Windows' permission prompt.
            if state == StorePackageUpdateState::Completed
                || state == StorePackageUpdateState::Canceled
            {
                Ok(())
            } else {
                Err(Error::new(
                    E_FAIL,
                    format!("The Store update ended as {}.", state.0),
                ))
            }
        }
    }
}
