//! Data types and models for ISO 32000 AcroForm interactive form fields.

use crate::cos::object::ObjectId;
use crate::layout::geometry::Rect;

/// Field types defined by ISO 32000-1 §12.7.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormFieldType {
    /// Text field (`/Tx`): single-line or multi-line text input.
    Text,
    /// Button field (`/Btn`): Checkbox.
    Checkbox,
    /// Button field (`/Btn`): Radio button option.
    RadioButton,
    /// Button field (`/Btn`): Pushbutton action.
    PushButton,
    /// Choice field (`/Ch`): Combo box / Dropdown.
    Choice,
    /// Signature field (`/Sig`): Cryptographic digital signature.
    Signature,
}

impl FormFieldType {
    /// Returns the standard PDF type code name.
    pub fn as_str(&self) -> &'static str {
        match self {
            FormFieldType::Text => "Text",
            FormFieldType::Checkbox => "Checkbox",
            FormFieldType::RadioButton => "RadioButton",
            FormFieldType::PushButton => "PushButton",
            FormFieldType::Choice => "Choice",
            FormFieldType::Signature => "Signature",
        }
    }
}

/// Represents an interactive form field in a PDF document.
#[derive(Debug, Clone)]
pub struct FormField {
    /// Indirect object ID of the field dictionary.
    pub id: ObjectId,
    /// Fully-qualified field name (e.g. `Client.FirstName` or `AgreeTerms`).
    pub name: String,
    /// User-friendly alternative field name / tooltip (`/TU`).
    pub alt_name: Option<String>,
    /// Field type.
    pub field_type: FormFieldType,
    /// Current field value (`/V`).
    pub value: String,
    /// Default field value (`/DV`), if present.
    pub default_value: Option<String>,
    /// Bounding rectangle in PDF points on the target page.
    pub rect: Rect,
    /// 1-based page number where this field/widget is placed.
    pub page_number: usize,
    /// Object ID of the associated page.
    pub page_id: ObjectId,
    /// Available options for choice/dropdown fields (`/Opt`).
    pub options: Vec<String>,
    /// Field flags bitfield (`/Ff`).
    pub flags: u32,
    /// Whether the field is read-only.
    pub is_read_only: bool,
    /// Whether the field is required.
    pub is_required: bool,
    /// Whether a text field is multiline.
    pub is_multiline: bool,
    /// Maximum character length (`/MaxLen`), if set.
    pub max_length: Option<usize>,
}

impl FormField {
    /// Checks whether the field is currently checked (for checkboxes and radio buttons).
    pub fn is_checked(&self) -> bool {
        match self.field_type {
            FormFieldType::Checkbox | FormFieldType::RadioButton => {
                let v = self.value.trim().to_lowercase();
                v == "yes" || v == "true" || v == "1" || v == "on"
            }
            _ => false,
        }
    }
}
