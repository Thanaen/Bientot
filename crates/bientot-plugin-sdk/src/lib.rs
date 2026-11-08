//! Writes Bientôt plugins in Rust (api 1.0). A plugin is a type that implements [`Plugin`],
//! exported with [`export!`]; it reads the day with [`day::current`] or from
//! [`Plugin::day_refreshed`], and shows a card with [`ui::show`]:
//!
//! ```
//! use bientot_plugin_sdk::prelude::*;
//!
//! #[derive(Default)]
//! struct Hello;
//!
//! impl Plugin for Hello {
//!     fn day_refreshed(&mut self, day: &DaySnapshot) {
//!         let _ = ui::show(card().line(row![text("counted"), duration(day.counted)]));
//!     }
//! }
//!
//! bientot_plugin_sdk::export!(Hello);
//! ```
//!
//! Built for `wasm32-unknown-unknown`, the crate talks to the host through the imports of
//! `contracts/host-abi.md`. Built natively, the same calls go to [`testing::TestHost`], which
//! checks them as Bientôt does, so a plugin is tested with `cargo test`.

pub mod card;
pub mod host;
#[cfg(not(target_arch = "wasm32"))]
pub mod testing;

#[doc(hidden)]
pub mod abi;

pub use bientot_plugin_api as api;
pub use bientot_plugin_api::{ActivateReason, CommandSource, DaySnapshot, Event, Language};
pub use host::{day, log, time, ui};

/// What a plugin needs, in one `use`.
pub mod prelude {
    pub use crate::card::{
        CardBuilder, TextNode, badge, button, card, countdown, duration, duration_since, progress,
        row, stack, text, time_at,
    };
    pub use crate::host::{day, log, time, ui};
    pub use crate::{ActivateReason, CommandSource, DaySnapshot, Event, Language, Plugin};
    pub use crate::{row, stack};
    pub use bientot_plugin_api::snapshot::{
        Forecast, OpenTail, Segment, SegmentKind, Source, Tracking, Week,
    };
    pub use bientot_plugin_api::view::{Role, Tone};
    pub use jiff::{SignedDuration, Timestamp};
}

/// A plugin. Bientôt creates one value with `Default` per instance and calls one method per
/// event; every method does nothing unless the plugin overrides it. An instance can be dropped
/// between two events (after five idle minutes, or when it grew too large) and a new one created
/// with [`Plugin::activate`], so a plugin keeps nothing in memory it cannot rebuild.
pub trait Plugin {
    /// The plugin was installed. Sent once, before the first `activate`.
    fn installed(&mut self, version: &str) {
        let _ = version;
    }

    /// The plugin was updated from `from`. Sent once, before the first `activate`.
    fn upgraded(&mut self, from: &str, version: &str) {
        let _ = (from, version);
    }

    /// A new instance starts. The first event every instance receives.
    fn activate(&mut self, language: Language, reason: ActivateReason) {
        let _ = (language, reason);
    }

    /// The day changed in a way worth a look: a segment settled, the tracking state, the target,
    /// or a departure moved by a minute or more. Needs `read:day` and the event in `events`.
    fn day_refreshed(&mut self, day: &DaySnapshot) {
        let _ = day;
    }

    /// The user ran one of the manifest's commands, from the palette or a card button.
    fn command(&mut self, command: &str, arg: Option<&str>, source: CommandSource) {
        let _ = (command, arg, source);
    }

    /// The app language changed. Cards are drawn in the new language by the host already; this
    /// is for a plugin that keeps texts of its own. Needs the event in `events`.
    fn language_changed(&mut self, language: Language) {
        let _ = language;
    }

    /// Dispatches one event to the methods above. An event of a later api is ignored.
    fn handle(&mut self, event: Event) {
        match event {
            Event::Installed { version } => self.installed(&version),
            Event::Upgraded { from, version } => self.upgraded(&from, &version),
            Event::Activate { language, reason } => self.activate(language, reason),
            Event::DayRefreshed { day } => self.day_refreshed(&day),
            Event::CommandInvoked {
                command,
                arg,
                source,
            } => self.command(&command, arg.as_deref(), source),
            Event::LanguageChanged { language } => self.language_changed(language),
            Event::Unknown => {}
        }
    }
}

/// A [`card::row`] of nodes of different types: `row![text("left"), duration(600)]`.
#[macro_export]
macro_rules! row {
    ($($node:expr),* $(,)?) => {
        $crate::card::row([$(::core::convert::Into::<$crate::api::Node>::into($node)),*])
    };
}

/// A [`card::stack`] of nodes of different types.
#[macro_export]
macro_rules! stack {
    ($($node:expr),* $(,)?) => {
        $crate::card::stack([$(::core::convert::Into::<$crate::api::Node>::into($node)),*])
    };
}

/// Exports a [`Plugin`] type, which must also implement `Default`, as the module's `bt_abi`,
/// `bt_alloc`, `bt_free` and `bt_handle`. Use it once, in the crate root. It does nothing in a
/// native build, where [`testing::TestHost`] drives the plugin instead.
#[macro_export]
macro_rules! export {
    ($plugin:ty) => {
        // A native build exports nothing; this keeps the plugin's code in use for the compiler.
        #[cfg(not(target_arch = "wasm32"))]
        #[doc(hidden)]
        pub fn __bientot_plugin(event: $crate::Event) {
            let mut plugin = <$plugin as ::core::default::Default>::default();
            $crate::Plugin::handle(&mut plugin, event);
        }

        #[cfg(target_arch = "wasm32")]
        const _: () = {
            ::std::thread_local! {
                static PLUGIN: ::core::cell::RefCell<::core::option::Option<$plugin>> =
                    const { ::core::cell::RefCell::new(::core::option::Option::None) };
            }

            #[unsafe(no_mangle)]
            extern "C" fn bt_abi() -> i32 {
                $crate::abi::ABI
            }

            #[unsafe(no_mangle)]
            extern "C" fn bt_alloc(len: i32) -> i32 {
                $crate::abi::alloc(len)
            }

            #[unsafe(no_mangle)]
            unsafe extern "C" fn bt_free(ptr: i32, len: i32) {
                // SAFETY: the host frees only buffers this module handed it.
                unsafe { $crate::abi::free(ptr, len) }
            }

            #[unsafe(no_mangle)]
            unsafe extern "C" fn bt_handle(ptr: i32, len: i32) -> i64 {
                // SAFETY: the host passes a buffer it allocated with `bt_alloc` and filled.
                let bytes = unsafe { $crate::abi::take(ptr, len) };
                PLUGIN.with_borrow_mut(|plugin| {
                    let plugin =
                        plugin.get_or_insert_with(<$plugin as ::core::default::Default>::default);
                    $crate::abi::handle(plugin, &bytes)
                })
            }
        };
    };
}
