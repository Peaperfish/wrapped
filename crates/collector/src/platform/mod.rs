//! Platform layer: everything OS-specific lives below this module.
//!
//! Contract each platform implements (macOS now, Windows in Milestone 9):
//! * `data_dir()`: where `wrapped.db` lives.
//! * an event source that turns OS signals into
//!   [`wrapped_core::SystemEvent`]s and feeds them to the engine on one
//!   thread (lands in Phase B).
//!
//! Core logic never imports from here, and nothing here makes tracking
//! decisions: it only translates OS signals.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(not(target_os = "macos"))]
mod unsupported;
#[cfg(not(target_os = "macos"))]
pub use unsupported::*;
