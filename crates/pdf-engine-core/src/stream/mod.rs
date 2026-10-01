//! Content Streams and Graphics State evaluation according to ISO 32000-1 §7.8 & §8.
//!
//! Provides lossless Abstract Syntax Tree (AST) representations for page descriptions,
//! affine transformation tracking (Current Transformation Matrix CTM),
//! exact text matrix state, and bidirectional AST serialization.

pub mod ast;
pub mod graphics_state;
pub mod parser;

pub use ast::*;
pub use graphics_state::*;
pub use parser::*;
