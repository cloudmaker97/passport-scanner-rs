//! Protocol and device driver for Plustek SecureScan scanners.
//!
//! This crate knows about *this family of scanners* and nothing about the operating
//! system. It speaks to the hardware exclusively through
//! [`passport_scanner_core::VendorTransport`], so the same logic runs over a Windows kernel
//! streaming property or a Linux V4L2 extension unit without changes.
//!
//! Supporting another model means adding a [`models::Model`] entry; supporting another
//! operating system means adding a transport crate. Neither touches this code.

#![forbid(unsafe_code)]

pub mod eeprom;
pub mod models;
pub mod protocol;
pub mod scanner;
pub mod settings;
pub mod steady;

pub use eeprom::{RegisterTable, RegisterWrite, calibration_stamp, parse_tables};
pub use models::Model;
pub use scanner::{Calibration, CaptureProfile, Scanner};
