//! The « Animation effects » setting of Windows (Accessibility, Visual effects), which the app
//! follows with GPUI's reduced motion.
//!
//! GPUI owns the window procedures and handles `WM_SETTINGCHANGE` without passing it on, so on
//! Windows [`SettingChanges`] watches the messages sent to the windows of the GPUI thread.

#[cfg(windows)]
mod native;

#[cfg(windows)]
pub use native::{SettingChanges, reduce_motion};

/// Whether to reduce motion for what the query of the setting answered, `None` when it failed.
/// Only a setting read as off reduces it, since GPUI animates by default.
pub fn reduces_motion(animation_effects: Option<bool>) -> bool {
    animation_effects == Some(false)
}

#[cfg(test)]
mod tests;
