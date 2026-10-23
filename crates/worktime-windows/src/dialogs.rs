//! The common item dialogs that pick where to save a file and which file to open, as the .NET
//! app's `FileSavePicker` did for « Exporter le diagnostic… ». Each blocks until closed, so it
//! runs on a thread of its own.

#[cfg(windows)]
mod native;

#[cfg(windows)]
pub use native::{pick_open_file, pick_save_file};
