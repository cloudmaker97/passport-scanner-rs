//! Device-agnostic abstractions for ID document scanners.
//!
//! This crate deliberately contains no I/O, no platform code and no knowledge of any
//! particular scanner. It defines the seams along which the rest of the system is
//! extended:
//!
//! | Seam | Trait | Add a new … |
//! |------|-------|-------------|
//! | vendor control channel | [`VendorTransport`] | OS backend (KS, V4L2, …) |
//! | image stream | [`FrameSource`] | capture backend |
//! | scanner hardware | [`ScannerDevice`] | model (X-Mini, X50, X-Cube, …) |
//! | pixel processing | [`ImageProcessor`] | correction stage |
//! | data extraction | [`DocumentRecognizer`] | MRZ, OCR, barcode, NFC chip |
//!
//! Every concrete implementation lives in its own crate and depends only on this one,
//! so adding a backend never requires touching existing code.

#![forbid(unsafe_code)]

pub mod device;
pub mod error;
pub mod frame;
pub mod geometry;
pub mod imaging;
pub mod light;
pub mod recognize;
pub mod transport;

pub use device::{DeviceDescriptor, DeviceId, DocumentPresence, ScannerDevice, SourceCapabilities};
pub use error::{Error, Result};
pub use frame::{Control, Frame, FrameSource, PixelFormat};
pub use geometry::Rect;
pub use imaging::{ImageProcessor, ShadingMap};
pub use light::LightSource;
pub use recognize::{Document, DocumentRecognizer, Field, FieldKind};
pub use transport::{Selector, VendorTransport};
