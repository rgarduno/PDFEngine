//! Surgical in-place content stream editor and reflow engine.
//!
//! Provides atomic mutations on content ASTs without altering non-targeted
//! graphics state, images, or vector paths. A paragraph edit can keep the
//! current font or switch to a standard 14 face.

pub mod mutator;
pub mod reflow;
pub mod style;

pub use mutator::SurgicalEditor;
pub use reflow::{ReflowEngine, ReflowLine};
pub use style::{ParagraphStyle, PreparedParagraphStyle};
