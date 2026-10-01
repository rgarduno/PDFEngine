//! Surgical in-place content stream editor and reflow engine.
//!
//! Provides atomic mutations on content ASTs without altering non-targeted
//! graphics state, images, or vector paths.

pub mod mutator;
pub mod reflow;

pub use mutator::SurgicalEditor;
pub use reflow::{ReflowEngine, ReflowLine};
