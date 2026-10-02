//! Dynamic pagination and watermark engine (ISO 32000-1 §8.4, §8.7, §8.9, §11).
//!
//! Provides Bates numbering, customizable headers and footers with `{page}` / `{total}` templating,
//! and semi-transparent rotated text and image watermarks with foreground/background placement.

pub mod helpers;
pub mod image;
pub mod pagination;
pub mod text;
pub mod types;

#[cfg(test)]
mod tests;

pub use image::apply_image_watermark;
pub use pagination::apply_pagination;
pub use text::apply_text_watermark;
pub use types::{
    ImageWatermarkConfig, PaginationConfig, PaginationPosition, TextWatermarkConfig,
    WatermarkPlacement,
};
