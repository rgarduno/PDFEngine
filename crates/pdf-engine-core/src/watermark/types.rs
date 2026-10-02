//! Domain models and configurations for dynamic pagination and watermarks (ISO 32000-1 §8.4, §8.7, §11).

/// Screen/page placement position for dynamic headers, footers, and pagination numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaginationPosition {
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl PaginationPosition {
    /// Parses a string into a PaginationPosition enum.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().replace(['-', '_', ' '], "").as_str() {
            "topleft" => Some(Self::TopLeft),
            "topcenter" => Some(Self::TopCenter),
            "topright" => Some(Self::TopRight),
            "bottomleft" => Some(Self::BottomLeft),
            "bottomcenter" => Some(Self::BottomCenter),
            "bottomright" => Some(Self::BottomRight),
            _ => None,
        }
    }

    /// Serializes to canonical identifier string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TopLeft => "top_left",
            Self::TopCenter => "top_center",
            Self::TopRight => "top_right",
            Self::BottomLeft => "bottom_left",
            Self::BottomCenter => "bottom_center",
            Self::BottomRight => "bottom_right",
        }
    }
}

/// Configuration parameters for dynamic pagination and Bates numbering.
#[derive(Debug, Clone)]
pub struct PaginationConfig {
    /// Template string with placeholders: `{page}` and `{total}`.
    /// Example: `"Página {page} de {total}"`, `"Page {page}"`, or `"- {page} -"`
    pub format: String,
    /// Spatial placement on the page.
    pub position: PaginationPosition,
    /// Font size in points.
    pub font_size: f64,
    /// Text color in RGB `[r, g, b]` where components are in `[0.0, 1.0]`.
    pub color: [f64; 3],
    /// Margin from page edge in points.
    pub margin: f64,
    /// Starting page number value (e.g. 1).
    pub start_page_num: usize,
    /// If true, pagination will not be printed on the first page (cover page).
    pub skip_first_page: bool,
    /// Optional specific page indices (0-indexed) to paginate.
    /// If None, applies across all pages in the document.
    pub page_indices: Option<Vec<usize>>,
}

impl Default for PaginationConfig {
    fn default() -> Self {
        Self {
            format: "Página {page} de {total}".to_string(),
            position: PaginationPosition::BottomCenter,
            font_size: 9.0,
            color: [0.35, 0.35, 0.35],
            margin: 36.0,
            start_page_num: 1,
            skip_first_page: false,
            page_indices: None,
        }
    }
}

/// Placement layer for watermarks relative to page visual content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatermarkPlacement {
    /// Renders behind existing text, graphics, and images.
    Background,
    /// Renders on top of existing page contents.
    Foreground,
}

impl WatermarkPlacement {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().trim() {
            "background" | "back" | "bg" => Some(Self::Background),
            "foreground" | "front" | "fg" => Some(Self::Foreground),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Foreground => "foreground",
        }
    }
}

/// Configuration parameters for semi-transparent text watermarks.
#[derive(Debug, Clone)]
pub struct TextWatermarkConfig {
    /// Watermark text string (e.g. "CONFIDENCIAL", "BORRADOR", "DRAFT").
    pub text: String,
    /// Font size in points.
    pub font_size: f64,
    /// Color in RGB `[r, g, b]` in range `[0.0, 1.0]`.
    pub color: [f64; 3],
    /// Alpha opacity in range `[0.0, 1.0]` (e.g. 0.18 for subtle background).
    pub opacity: f64,
    /// Rotation angle in degrees (e.g. 45.0 for diagonal watermark).
    pub rotation_degrees: f64,
    /// Background or foreground placement.
    pub placement: WatermarkPlacement,
    /// Optional target page indices (0-indexed). If None, applies to all pages.
    pub page_indices: Option<Vec<usize>>,
}

impl Default for TextWatermarkConfig {
    fn default() -> Self {
        Self {
            text: "CONFIDENCIAL".to_string(),
            font_size: 52.0,
            color: [0.80, 0.20, 0.20],
            opacity: 0.22,
            rotation_degrees: 45.0,
            placement: WatermarkPlacement::Background,
            page_indices: None,
        }
    }
}

/// Configuration parameters for semi-transparent image watermarks.
#[derive(Debug, Clone)]
pub struct ImageWatermarkConfig {
    /// Raw binary bytes of the image (JPEG or PNG format).
    pub image_bytes: Vec<u8>,
    /// Optional target width in points. If None, derived from page width (e.g. 50%).
    pub width: Option<f64>,
    /// Optional target height in points. If None, preserves natural aspect ratio.
    pub height: Option<f64>,
    /// Alpha opacity in range `[0.0, 1.0]` (e.g. 0.25).
    pub opacity: f64,
    /// Rotation angle in degrees (e.g. 0.0 or 45.0).
    pub rotation_degrees: f64,
    /// Background or foreground placement.
    pub placement: WatermarkPlacement,
    /// Optional target page indices (0-indexed). If None, applies to all pages.
    pub page_indices: Option<Vec<usize>>,
}

impl Default for ImageWatermarkConfig {
    fn default() -> Self {
        Self {
            image_bytes: Vec::new(),
            width: None,
            height: None,
            opacity: 0.25,
            rotation_degrees: 0.0,
            placement: WatermarkPlacement::Background,
            page_indices: None,
        }
    }
}
