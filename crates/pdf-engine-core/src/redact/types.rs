//! Data models and configurations for PDF Redaction and Content Sanitization according to ISO 32000-1 §14.11.

use crate::layout::geometry::Rect;

/// Target region to be permanently redacted and visually blacked out.
#[derive(Debug, Clone, PartialEq)]
pub struct RedactionRect {
    /// Bounding box on the target page in device points.
    pub rect: Rect,
    /// Solid fill color for the blackout box in normalized RGB [0.0..1.0]. Default is black [0.0, 0.0, 0.0].
    pub fill_color: [f64; 3],
    /// Optional stroke / border outline color in normalized RGB [0.0..1.0].
    pub border_color: Option<[f64; 3]>,
    /// Optional overlay label displayed centered inside the blackout box (e.g. "[REDACTADO]", "CONFIDENTIAL").
    pub overlay_text: Option<String>,
    /// Text color for the overlay label in normalized RGB [0.0..1.0]. Default is white [1.0, 1.0, 1.0].
    pub text_color: [f64; 3],
    /// Font size for the overlay label. If None, auto-calculated based on rectangle height.
    pub font_size: Option<f64>,
}

impl RedactionRect {
    /// Creates a standard black redaction rectangle without overlay text.
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            fill_color: [0.0, 0.0, 0.0],
            border_color: None,
            overlay_text: None,
            text_color: [1.0, 1.0, 1.0],
            font_size: None,
        }
    }

    /// Sets the fill color.
    pub fn with_fill_color(mut self, r: f64, g: f64, b: f64) -> Self {
        self.fill_color = [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)];
        self
    }

    /// Sets the overlay text and its color.
    pub fn with_overlay_text(mut self, text: impl Into<String>, text_color: Option<[f64; 3]>) -> Self {
        self.overlay_text = Some(text.into());
        if let Some(c) = text_color {
            self.text_color = [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)];
        }
        self
    }

    /// Sets the font size of the overlay text.
    pub fn with_font_size(mut self, size: f64) -> Self {
        self.font_size = Some(size.max(4.0));
        self
    }
}

/// Built-in PII (Personally Identifiable Information) pattern targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedactionPattern {
    /// Email addresses (e.g. user@domain.com).
    Email,
    /// Phone numbers (domestic, international, formatted or unformatted).
    Phone,
    /// Social Security Numbers (e.g. 123-45-6789 or 9 consecutive digits).
    Ssn,
    /// Credit Card / Debit Card PAN numbers (16 digits formatted or continuous).
    CreditCard,
    /// Mexican RFC (Taxpayer ID for Persona Física or Moral).
    Rfc,
    /// Mexican CURP (Unique Population Registry Code, 18 chars).
    Curp,
    /// Exact substring match with optional case sensitivity.
    Text {
        query: String,
        case_sensitive: bool,
    },
}

impl RedactionPattern {
    /// Parses a pattern name string into an enum variant.
    pub fn from_name(name: &str, custom_query: Option<&str>, case_sensitive: bool) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "email" | "correo" => Some(Self::Email),
            "phone" | "telefono" | "tel" => Some(Self::Phone),
            "ssn" | "social_security" => Some(Self::Ssn),
            "credit_card" | "tarjeta" | "card" => Some(Self::CreditCard),
            "rfc" => Some(Self::Rfc),
            "curp" => Some(Self::Curp),
            "text" | "string" | "query" => {
                custom_query.map(|q| Self::Text {
                    query: q.to_string(),
                    case_sensitive,
                })
            }
            _ => None,
        }
    }
}

/// Global configuration options for a redaction job.
#[derive(Debug, Clone)]
pub struct RedactionConfig {
    /// Default fill color for blackout boxes in normalized RGB [0.0..1.0]. Default is black [0.0, 0.0, 0.0].
    pub fill_color: [f64; 3],
    /// Default overlay label (e.g. "[REDACTADO]").
    pub overlay_text: Option<String>,
    /// Default text color for overlay label in normalized RGB [0.0..1.0].
    pub text_color: [f64; 3],
    /// Default font size for overlay label.
    pub font_size: Option<f64>,
    /// Whether to prune interactive annotations (links, comments, highlights) intersecting redaction boxes.
    pub prune_annotations: bool,
    /// Whether to scrub document metadata (Author, Title, Keywords, Creator) and XMP metadata stream.
    pub scrub_metadata: bool,
    /// Extra horizontal and vertical padding around matched text bounding boxes in points.
    pub padding: f64,
}

impl Default for RedactionConfig {
    fn default() -> Self {
        Self {
            fill_color: [0.0, 0.0, 0.0],
            overlay_text: Some("[REDACTADO]".to_string()),
            text_color: [1.0, 1.0, 1.0],
            font_size: None,
            prune_annotations: true,
            scrub_metadata: false,
            padding: 1.5,
        }
    }
}

/// Statistical summary of an applied redaction pass.
#[derive(Debug, Clone, Default)]
pub struct RedactionSummary {
    /// Target 0-based page index.
    pub page_index: usize,
    /// Total number of individual text glyphs physically purged from the content stream.
    pub purged_glyphs_count: usize,
    /// Total number of AST TextBlocks modified or cleared.
    pub modified_blocks_count: usize,
    /// Total number of blackout boxes rendered.
    pub blackout_boxes_count: usize,
    /// Total number of annotations deleted (links, markup, redact annotations).
    pub pruned_annotations_count: usize,
    /// List of spatial rectangles where redactions were applied.
    pub applied_rects: Vec<Rect>,
}
