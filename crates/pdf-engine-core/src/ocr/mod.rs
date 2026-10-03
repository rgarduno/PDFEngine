//! Searchable text for a page whose content is a single scanned image.
//!
//! A scan is a page that paints exactly one Image XObject and shows no text.
//! Recognition reads that image with the local `tesseract` binary. The engine
//! then appends a text block to the page content stream in rendering mode 3
//! (neither fill nor stroke) and attaches a WinAnsi `/ToUnicode` CMap, so the
//! words can be searched and copied without changing the scan.

mod layer;
mod recognize;
mod scan;

pub use layer::add_searchable_text_layer;
pub use recognize::OcrReport;

#[cfg(test)]
mod tests;
