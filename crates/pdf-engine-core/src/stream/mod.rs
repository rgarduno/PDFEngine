//! Content Streams and Graphics State evaluation according to ISO 32000-1 §7.8 & §8.
//!
//! Provides lossless Abstract Syntax Tree (AST) representations for page descriptions,
//! affine transformation tracking (Current Transformation Matrix CTM),
//! and exact text matrix state.

pub mod ast;

pub use ast::*;
