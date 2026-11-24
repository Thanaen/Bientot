//! The last-input query behind the built-in activity source of `bientot-platform`.

#[cfg(windows)]
mod native;

#[cfg(windows)]
pub use native::SystemLastInput;
