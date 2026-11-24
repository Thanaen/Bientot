//! Microsoft Store updates, ported from `Updates/StoreContextUpdatePort.cs`. Only the MSIX build
//! has a Store to ask; on Windows [`StoreUpdates`] asks it through WinRT's `StoreContext`.

#[cfg(windows)]
mod native;

#[cfg(windows)]
pub use native::{StoreUpdateOffer, StoreUpdates};
