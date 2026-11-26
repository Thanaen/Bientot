//! Bientôt's Linux integration (specs/005-linux-support). The rules stay platform independent and
//! tested on any OS; only the system calls behind them, in the `native` modules, are Linux only.

#![deny(unsafe_code)]

pub mod logind;
pub mod placement;
