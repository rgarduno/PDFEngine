//! Surgical in-place content stream editor and reflow engine.
//!
//! Provides atomic mutations on content ASTs without altering non-targeted
//! graphics state, images, or vector paths. A paragraph edit can keep the
//! current font or switch to a standard 14 face. A text box appends new
//! content inside a rectangle. It is not a `/FreeText` annotation.
//! A fill-and-sign mark appends typed text, a drawn path, or one image.
//! It is not a PKCS#7 signature.

pub mod mutator;
pub mod reflow;
pub mod sign;
pub mod style;
pub mod textbox;

pub use mutator::SurgicalEditor;
pub use reflow::{ReflowEngine, ReflowLine};
pub use sign::{append_fill_sign, AppendedFillSign, FillSignKind, FillSignMark, FillSignSpec};
pub use style::{ParagraphStyle, PreparedParagraphStyle};
pub use textbox::{append_text_box, AppendedTextBox, TextBoxSpec};
