//! ISO 32000-1 §12.5 Annotations module.
//!
//! Provides inspection, creation, deletion, and flattening of PDF annotations
//! including highlights, underlines, strikeouts, interactive web links, and rubber stamps.

pub mod delete;
pub mod flatten;
pub mod link;
pub mod markup;
pub mod reader;
pub mod stamp;
pub mod types;

#[cfg(test)]
mod tests;

pub use delete::delete_annotation;
pub use flatten::flatten_annotations;
pub use link::{add_link_goto, add_link_uri};
pub use markup::add_text_markup;
pub use reader::{extract_all_annotations, extract_page_annotations};
pub use stamp::add_stamp;
pub use types::{Annotation, AnnotationSubtype, LinkAction, StampType};
