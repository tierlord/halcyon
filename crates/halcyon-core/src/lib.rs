//! Halcyon core: everything that is independent of the user interface.
//!
//! The core owns the local mail store (SQLite with full-text search), the
//! account discovery and the IMAP synchronisation. Front ends (the desktop
//! app, the development CLI and later mobile builds) only talk to this crate.

pub mod autoconfig;
pub mod brand;
pub mod content;
pub mod error;
pub mod extract;
pub mod imap;
pub mod model;
pub mod reply;
pub mod secrets;
pub mod send;
pub mod store;
pub mod sync;
pub mod utf7;

pub use error::{Error, Result};
