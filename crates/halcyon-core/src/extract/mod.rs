//! Pure, offline extractors for structured tokens in messages.
//!
//! Used by stage-1 classification (detecting one-time codes and shipments)
//! and to guard sensitive messages from leaving the device.

pub mod finance;
pub mod otp;
pub mod tracking;
