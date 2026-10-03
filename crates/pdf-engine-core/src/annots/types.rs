//! Data models and enums for ISO 32000-1 §12.5 PDF Annotations.

use crate::cos::object::ObjectId;
use crate::layout::geometry::Rect;

/// Annotation subtype defined by ISO 32000-1 §12.5.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationSubtype {
    /// Text highlight annotation (`/Highlight`).
    Highlight,
    /// Text underline annotation (`/Underline`).
    Underline,
    /// Text strikethrough annotation (`/StrikeOut`).
    StrikeOut,
    /// Interactive link annotation (`/Link`).
    Link,
    /// Rubber stamp annotation (`/Stamp`).
    Stamp,
    /// Freehand stroke (`/Ink`).
    Ink,
    /// Rectangle (`/Square`).
    Square,
    /// Ellipse (`/Circle`).
    Circle,
    /// Straight line, optionally with an arrow ending (`/Line`).
    Line,
    /// Closed polygon (`/Polygon`).
    Polygon,
    /// Other unhandled or custom annotation subtype.
    Other,
}

impl AnnotationSubtype {
    /// Returns the ISO 32000 subtype name.
    pub fn as_pdf_name(&self) -> &'static str {
        match self {
            AnnotationSubtype::Highlight => "Highlight",
            AnnotationSubtype::Underline => "Underline",
            AnnotationSubtype::StrikeOut => "StrikeOut",
            AnnotationSubtype::Link => "Link",
            AnnotationSubtype::Stamp => "Stamp",
            AnnotationSubtype::Ink => "Ink",
            AnnotationSubtype::Square => "Square",
            AnnotationSubtype::Circle => "Circle",
            AnnotationSubtype::Line => "Line",
            AnnotationSubtype::Polygon => "Polygon",
            AnnotationSubtype::Other => "Unknown",
        }
    }

    /// Parses from a PDF subtype name.
    pub fn from_pdf_name(name: &str) -> Self {
        match name {
            "Highlight" => AnnotationSubtype::Highlight,
            "Underline" => AnnotationSubtype::Underline,
            "StrikeOut" => AnnotationSubtype::StrikeOut,
            "Link" => AnnotationSubtype::Link,
            "Stamp" => AnnotationSubtype::Stamp,
            "Ink" => AnnotationSubtype::Ink,
            "Square" => AnnotationSubtype::Square,
            "Circle" => AnnotationSubtype::Circle,
            "Line" => AnnotationSubtype::Line,
            "Polygon" => AnnotationSubtype::Polygon,
            _ => AnnotationSubtype::Other,
        }
    }
}

/// Link action target.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkAction {
    /// External web URI (e.g. `https://example.com` or `mailto:...`).
    Uri(String),
    /// Internal 0-based page index.
    GoTo(usize),
}

/// Standard or custom rubber stamp rubrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StampType {
    Approved,
    Confidential,
    Draft,
    Rejected,
    Final,
    TopSecret,
    Custom(String),
}

impl StampType {
    /// Returns the human-readable text label of the stamp.
    pub fn text(&self) -> String {
        match self {
            StampType::Approved => "APPROVED".to_string(),
            StampType::Confidential => "CONFIDENTIAL".to_string(),
            StampType::Draft => "DRAFT".to_string(),
            StampType::Rejected => "REJECTED".to_string(),
            StampType::Final => "FINAL".to_string(),
            StampType::TopSecret => "TOP SECRET".to_string(),
            StampType::Custom(s) => s.clone(),
        }
    }

    /// Standard RGB color associated with the stamp type.
    pub fn default_color(&self) -> [f64; 3] {
        match self {
            StampType::Approved => [0.15, 0.68, 0.38],      // Emerald Green
            StampType::Confidential => [0.85, 0.15, 0.15],  // Red
            StampType::Draft => [0.20, 0.45, 0.85],         // Royal Blue
            StampType::Rejected => [0.75, 0.10, 0.10],      // Crimson Red
            StampType::Final => [0.40, 0.20, 0.70],         // Purple
            StampType::TopSecret => [0.90, 0.30, 0.10],     // Dark Orange
            StampType::Custom(_) => [0.20, 0.45, 0.85],     // Blue
        }
    }

    /// Parses from a string name or text.
    pub fn from_name_or_text(s: &str) -> Self {
        match s.trim().to_ascii_uppercase().as_str() {
            "APPROVED" => StampType::Approved,
            "CONFIDENTIAL" => StampType::Confidential,
            "DRAFT" => StampType::Draft,
            "REJECTED" => StampType::Rejected,
            "FINAL" => StampType::Final,
            "TOPSECRET" | "TOP SECRET" => StampType::TopSecret,
            other => StampType::Custom(other.to_string()),
        }
    }
}

/// Structured representation of a PDF annotation.
#[derive(Debug, Clone)]
pub struct Annotation {
    /// Indirect object ID of the annotation dictionary.
    pub id: ObjectId,
    /// 0-based page index.
    pub page_index: usize,
    /// Indirect object ID of the containing page.
    pub page_id: ObjectId,
    /// Annotation subtype.
    pub subtype: AnnotationSubtype,
    /// Spatial bounding box in user space coordinates.
    pub rect: Rect,
    /// Color array in RGB [0.0..1.0].
    pub color: Option<[f64; 3]>,
    /// Opacity [0.0..1.0].
    pub opacity: f64,
    /// Optional text contents / tooltip (`/Contents`).
    pub contents: Option<String>,
    /// Link action (if subtype == Link).
    pub link_action: Option<LinkAction>,
    /// Stamp rubric details (if subtype == Stamp).
    pub stamp_type: Option<StampType>,
    /// Optional date stamp string.
    pub date_str: Option<String>,
    /// Border width from `/BS /W`. Defaults to 1 when the dictionary omits it.
    pub border_width: f64,
    /// Interior color `/IC` for square, circle, and polygon marks.
    pub fill_color: Option<[f64; 3]>,
    /// Stroke vertices in user space (`/InkList`, `/L`, or `/Vertices`).
    pub points: Vec<[f64; 2]>,
    /// Line ending name at the second point, such as `OpenArrow`.
    pub line_ending: Option<String>,
}
