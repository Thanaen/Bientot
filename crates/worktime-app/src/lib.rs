//! WorkTime's GPUI shell: the notification-area icon and the windows it opens.

pub mod adjust;
pub mod adjust_view;
pub mod assets;
pub mod automatic;
pub mod demo;
pub mod diagnostics;
pub mod file_dialogs;
pub mod flyout;
pub mod host;
pub mod launch;
pub mod ledger;
pub mod live;
pub mod notify;
pub mod onboarding;
pub mod onboarding_view;
pub mod palette;
pub mod palette_view;
pub mod settings;
pub mod settings_view;
pub mod shell;
pub mod startup_failure;
pub mod startup_failure_view;
pub mod strings;
pub mod text_input;
pub mod theme;
pub mod time_text;
pub mod transfer;
#[cfg(windows)]
pub mod tray;
pub mod tray_model;
pub mod updates;
pub mod usage_ping;
pub mod week;
pub mod week_view;
pub mod widgets;
pub mod workday;
pub mod zones;

use std::hash::{BuildHasher as _, RandomState};

/// 128 random bits, as the .NET app takes them from a new GUID.
pub(crate) fn random_id() -> u128 {
    let random = RandomState::new();
    (u128::from(random.hash_one(1_u8)) << 64) | u128::from(random.hash_one(2_u8))
}
