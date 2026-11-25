//! Halcyon core: everything that is independent of the user interface.
//!
//! The core owns the local mail store (SQLite with full-text search), the
//! account discovery and the IMAP synchronisation. Front ends (the desktop
//! app, the development CLI and later mobile builds) only talk to this crate.

pub mod assignments;
pub mod autoconfig;
pub mod brand;
pub mod bundles;
pub mod cases;
pub mod content;
pub mod error;
pub mod extract;
pub mod filing;
pub mod imap;
pub mod invoices;
pub mod kinds;
pub mod linker;
pub mod mask;
pub mod model;
pub mod notify;
pub mod phishing;
pub mod phishing_store;
pub mod reply;
pub mod secrets;
pub mod send;
pub mod sources;
pub mod store;
pub mod sync;
pub mod turn;
pub mod utf7;

pub use error::{Error, Result};
