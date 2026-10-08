//! PDF/A-1b and PDF/A-2b archive conversion.
//!
//! The engine checks structure, embeds an original bitmap face for unembedded
//! simple fonts, writes an RGB output intent, and removes features the part
//! forbids. This is the engine's own check. It is not a veraPDF certificate,
//! an Acrobat preflight, or an acceptance by a court.

mod convert;
mod face;
mod icc;

#[cfg(test)]
mod tests;

pub use convert::{convert_to_pdfa, validate_pdfa, PdfALevel};
