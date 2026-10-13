//! Paragraph style for a surgical text edit.
//!
//! A face change is limited to the standard 14 names. Size, color, alignment,
//! and leading can stay on the font resource the paragraph already uses, so an
//! embedded subset keeps its character codes. Leading is a distance in points.

use crate::error::{PdfError, PdfResult};
use crate::fonts::css_face;
use crate::layout::TextAlignment;

/// CSS family a caller may name. Bold and italic select the matching standard face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StandardFamily {
    Helvetica,
    Times,
    Courier,
}

/// Optional style applied while replacing a paragraph.
///
/// Every field is independent. An empty value keeps the paragraph's current
/// font, size, alignment, and leading, and writes no color operator.
#[derive(Debug, Clone, PartialEq)]
pub struct ParagraphStyle {
    /// Helvetica, Times, or Courier, including the Arial, Times New Roman, and Courier New aliases.
    pub family: Option<String>,
    /// Font size in points.
    pub size: Option<f64>,
    /// `Some(true)` selects the bold standard face.
    pub bold: Option<bool>,
    /// `Some(true)` selects the italic or oblique standard face.
    pub italic: Option<bool>,
    /// Fill color as device RGB components in `0..=1`.
    pub color: Option<[f64; 3]>,
    pub alignment: Option<TextAlignment>,
    /// Baseline distance in points.
    pub leading: Option<f64>,
}

impl Default for ParagraphStyle {
    fn default() -> Self {
        Self {
            family: None,
            size: None,
            bold: None,
            italic: None,
            color: None,
            alignment: None,
            leading: None,
        }
    }
}

impl ParagraphStyle {
    pub fn is_empty(&self) -> bool {
        self.family.is_none()
            && self.size.is_none()
            && self.bold.is_none()
            && self.italic.is_none()
            && self.color.is_none()
            && self.alignment.is_none()
            && self.leading.is_none()
    }

    /// Parses `left`, `center`, `right`, or `justified`.
    pub fn parse_alignment(raw: &str) -> PdfResult<TextAlignment> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "left" => Ok(TextAlignment::Left),
            "center" => Ok(TextAlignment::Center),
            "right" => Ok(TextAlignment::Right),
            "justified" => Ok(TextAlignment::Justified),
            _ => Err(style_error(
                "Alignment must be left, center, right, or justified.",
            )),
        }
    }

    /// Accepts three finite device-RGB components in `0..=1`.
    pub fn color_from_components(values: &[f64]) -> PdfResult<[f64; 3]> {
        if values.len() != 3
            || values
                .iter()
                .any(|component| !component.is_finite() || *component < 0.0 || *component > 1.0)
        {
            return Err(style_error(
                "Color components must be finite numbers from 0 to 1.",
            ));
        }
        Ok([values[0], values[1], values[2]])
    }

    /// Resolves the operators a surgical edit will emit.
    ///
    /// `current_family` is the CSS family of the font resource already in use.
    /// It is empty when that resource has no resolved face. A named family does
    /// not inherit the old weight. Bold or italic without a family does.
    pub fn prepare(
        &self,
        current_resource: &str,
        current_size: f64,
        current_leading: f64,
        current_alignment: TextAlignment,
        current_family: &str,
        current_weight: u16,
        current_css_style: &str,
    ) -> PdfResult<PreparedParagraphStyle> {
        let size = match self.size {
            Some(size) => finite_in(size, 1.0, 144.0, "Font size must be a finite number from 1 to 144.")?,
            None => current_size,
        };
        let leading = match self.leading {
            Some(leading) => {
                finite_in(leading, 1.0, 400.0, "Leading must be a finite number from 1 to 400.")?
            }
            None if self.size.is_some() => scaled_leading(current_leading, current_size, size),
            None => current_leading,
        };
        if let Some(color) = self.color {
            Self::color_from_components(&color)?;
        }

        let resource = current_resource.trim_start_matches('/').to_string();
        let alignment = self.alignment.unwrap_or(current_alignment);
        if !self.requests_face() {
            return Ok(PreparedParagraphStyle {
                font_resource: resource,
                base_font: None,
                registers_font: false,
                css_family: current_family.to_string(),
                weight: current_weight,
                css_style: current_css_style.to_string(),
                size,
                leading,
                alignment,
                color: self.color,
            });
        }

        let (family, bold, italic) = if let Some(name) = &self.family {
            let family = parse_family(name)?;
            (
                family,
                self.bold.unwrap_or(false),
                self.italic.unwrap_or(false),
            )
        } else {
            let family = family_from_css(current_family).ok_or_else(|| {
                style_error(
                    "The current font cannot change weight or style. Name Helvetica, Times, or Courier.",
                )
            })?;
            (
                family,
                self.bold.unwrap_or(current_weight >= 700),
                self.italic.unwrap_or(current_css_style == "italic"),
            )
        };
        let (alias, base_font) = standard_face(family, bold, italic);
        let (css_family, weight, css_style) = css_face(base_font);
        Ok(PreparedParagraphStyle {
            font_resource: alias.to_string(),
            base_font: Some(base_font.to_string()),
            registers_font: true,
            css_family,
            weight,
            css_style,
            size,
            leading,
            alignment,
            color: self.color,
        })
    }

    fn requests_face(&self) -> bool {
        self.family.is_some() || self.bold.is_some() || self.italic.is_some()
    }
}

/// Concrete font, size, color, alignment, and leading for one edit.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedParagraphStyle {
    /// Resource name used by `Tf`, without a leading slash.
    pub font_resource: String,
    /// Standard 14 `/BaseFont` when this edit registers a new face.
    pub base_font: Option<String>,
    /// True when the page font dictionary must gain `font_resource`.
    pub registers_font: bool,
    pub css_family: String,
    pub weight: u16,
    pub css_style: String,
    pub size: f64,
    pub leading: f64,
    pub alignment: TextAlignment,
    pub color: Option<[f64; 3]>,
}

fn style_error(message: &str) -> PdfError {
    PdfError::OperationError(format!("Style: {message}"))
}

fn finite_in(value: f64, min: f64, max: f64, message: &str) -> PdfResult<f64> {
    if value.is_finite() && value >= min && value <= max {
        Ok(value)
    } else {
        Err(style_error(message))
    }
}

/// Scales the old leading with the size, then keeps it from overlapping the next line.
fn scaled_leading(current_leading: f64, old_size: f64, new_size: f64) -> f64 {
    let raw = if old_size.is_finite() && old_size > 0.0 && current_leading.is_finite() && current_leading > 0.0
    {
        current_leading * (new_size / old_size)
    } else {
        new_size * 1.2
    };
    if !raw.is_finite() {
        return new_size;
    }
    raw.clamp(new_size, 400.0)
}

fn normalize_token(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_family(raw: &str) -> PdfResult<StandardFamily> {
    match normalize_token(raw).as_str() {
        "helvetica" | "arial" => Ok(StandardFamily::Helvetica),
        "times" | "times roman" | "times new roman" => Ok(StandardFamily::Times),
        "courier" | "courier new" => Ok(StandardFamily::Courier),
        _ => Err(style_error(
            "Font must be Helvetica, Times, or Courier.",
        )),
    }
}

fn family_from_css(family: &str) -> Option<StandardFamily> {
    let lower = family.to_ascii_lowercase();
    if lower.contains("arial") || lower.contains("helvetica") {
        Some(StandardFamily::Helvetica)
    } else if lower.contains("times") {
        Some(StandardFamily::Times)
    } else if lower.contains("courier") {
        Some(StandardFamily::Courier)
    } else {
        None
    }
}

fn standard_face(family: StandardFamily, bold: bool, italic: bool) -> (&'static str, &'static str) {
    match (family, bold, italic) {
        (StandardFamily::Helvetica, false, false) => ("F_ST_Helv", "Helvetica"),
        (StandardFamily::Helvetica, true, false) => ("F_ST_HeBo", "Helvetica-Bold"),
        (StandardFamily::Helvetica, false, true) => ("F_ST_HeOb", "Helvetica-Oblique"),
        (StandardFamily::Helvetica, true, true) => ("F_ST_HeBO", "Helvetica-BoldOblique"),
        (StandardFamily::Times, false, false) => ("F_ST_TimR", "Times-Roman"),
        (StandardFamily::Times, true, false) => ("F_ST_TiBo", "Times-Bold"),
        (StandardFamily::Times, false, true) => ("F_ST_TiIt", "Times-Italic"),
        (StandardFamily::Times, true, true) => ("F_ST_TiBI", "Times-BoldItalic"),
        (StandardFamily::Courier, false, false) => ("F_ST_Cour", "Courier"),
        (StandardFamily::Courier, true, false) => ("F_ST_CoBo", "Courier-Bold"),
        (StandardFamily::Courier, false, true) => ("F_ST_CoOb", "Courier-Oblique"),
        (StandardFamily::Courier, true, true) => ("F_ST_CoBO", "Courier-BoldOblique"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::SurgicalEditor;
    use crate::fonts::FontMetrics;
    use crate::layout::LayoutReconstructor;
    use crate::stream::{build_ast_from_operations, serialize_ast, ContentStreamTokenizer};

    fn prepare_current(style: &ParagraphStyle, family: &str, weight: u16, css_style: &str) -> PdfResult<PreparedParagraphStyle> {
        style.prepare("F1", 12.0, 14.0, TextAlignment::Left, family, weight, css_style)
    }

    fn edit_stream(source: &str, text: &str, style: &ParagraphStyle) -> (String, Vec<crate::layout::ParagraphBlock>) {
        let mut tokenizer = ContentStreamTokenizer::new(source.as_bytes());
        let ops = tokenizer.tokenize_all().expect("tokenize");
        let mut ast = build_ast_from_operations(ops);
        let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
        let paragraphs = LayoutReconstructor::new(&ast)
            .with_font("F1", metrics.clone())
            .reconstruct()
            .expect("reconstruct");
        let prepared = style
            .prepare(
                &paragraphs[0].font_resource(),
                paragraphs[0].lines[0].spans[0].font_size,
                paragraphs[0].leading,
                paragraphs[0].alignment,
                "",
                400,
                "normal",
            )
            .expect("prepare");
        SurgicalEditor::edit_paragraph_prepared(&mut ast, &paragraphs[0], text, &metrics, &prepared)
            .expect("edit");
        let serialized = String::from_utf8(serialize_ast(&ast)).expect("utf8");
        let after = LayoutReconstructor::new(&ast)
            .with_font("F1", metrics.clone())
            .with_font(&prepared.font_resource, metrics)
            .reconstruct()
            .expect("reconstruct after");
        (serialized, after)
    }

    #[test]
    fn empty_style_keeps_the_current_face() {
        let prepared = prepare_current(&ParagraphStyle::default(), "Calibri", 700, "italic").unwrap();
        assert!(!prepared.registers_font);
        assert_eq!(prepared.font_resource, "F1");
        assert!(prepared.base_font.is_none());
        assert!((prepared.size - 12.0).abs() < f64::EPSILON);
        assert!((prepared.leading - 14.0).abs() < f64::EPSILON);
        assert_eq!(prepared.alignment, TextAlignment::Left);
    }

    #[test]
    fn named_family_does_not_inherit_the_old_weight() {
        let style = ParagraphStyle {
            family: Some("Times New Roman".to_string()),
            italic: Some(true),
            ..ParagraphStyle::default()
        };
        let prepared = prepare_current(&style, "Arial", 700, "normal").unwrap();
        assert_eq!(prepared.font_resource, "F_ST_TiIt");
        assert_eq!(prepared.base_font.as_deref(), Some("Times-Italic"));
        assert_eq!(prepared.weight, 400);
        assert_eq!(prepared.css_style, "italic");
        assert_eq!(prepared.css_family, "Times New Roman");
    }

    #[test]
    fn bold_without_a_family_uses_the_current_css_face() {
        let style = ParagraphStyle {
            bold: Some(true),
            ..ParagraphStyle::default()
        };
        let helvetica = prepare_current(&style, "Arial", 400, "normal").unwrap();
        assert_eq!(helvetica.font_resource, "F_ST_HeBo");
        assert_eq!(helvetica.base_font.as_deref(), Some("Helvetica-Bold"));

        let times_italic = prepare_current(&style, "Times New Roman", 400, "italic").unwrap();
        assert_eq!(times_italic.font_resource, "F_ST_TiBI");
        assert_eq!(times_italic.base_font.as_deref(), Some("Times-BoldItalic"));
    }

    #[test]
    fn bold_on_an_unknown_face_is_rejected() {
        let style = ParagraphStyle {
            bold: Some(true),
            ..ParagraphStyle::default()
        };
        for family in ["", "Calibri", "F1"] {
            let error = prepare_current(&style, family, 400, "normal").unwrap_err();
            let text = error.to_string();
            assert!(
                text.contains("Style: The current font cannot change weight or style."),
                "{family}: {text}"
            );
        }
    }

    #[test]
    fn rejected_family_size_leading_and_color() {
        let bad_family = ParagraphStyle {
            family: Some("Comic Sans".to_string()),
            ..ParagraphStyle::default()
        };
        assert!(prepare_current(&bad_family, "", 400, "normal")
            .unwrap_err()
            .to_string()
            .contains("Style: Font must be Helvetica, Times, or Courier."));

        for size in [0.0, -1.0, 144.1, f64::NAN, f64::INFINITY] {
            let style = ParagraphStyle {
                size: Some(size),
                ..ParagraphStyle::default()
            };
            assert!(prepare_current(&style, "", 400, "normal")
                .unwrap_err()
                .to_string()
                .contains("Style: Font size must be a finite number from 1 to 144."));
        }

        for leading in [0.0, 400.1, f64::NAN] {
            let style = ParagraphStyle {
                leading: Some(leading),
                ..ParagraphStyle::default()
            };
            assert!(prepare_current(&style, "", 400, "normal")
                .unwrap_err()
                .to_string()
                .contains("Style: Leading must be a finite number from 1 to 400."));
        }

        assert!(ParagraphStyle::color_from_components(&[1.0, 0.0, 1.2])
            .unwrap_err()
            .to_string()
            .contains("Style: Color components must be finite numbers from 0 to 1."));
        assert!(ParagraphStyle::parse_alignment("middle")
            .unwrap_err()
            .to_string()
            .contains("Style: Alignment must be left, center, right, or justified."));
    }

    #[test]
    fn size_change_scales_leading_and_keeps_the_resource() {
        let style = ParagraphStyle {
            size: Some(24.0),
            ..ParagraphStyle::default()
        };
        let prepared = prepare_current(&style, "Calibri", 400, "normal").unwrap();
        assert_eq!(prepared.font_resource, "F1");
        assert!(!prepared.registers_font);
        assert!((prepared.leading - 28.0).abs() < 0.001);
    }

    #[test]
    fn aliases_select_the_standard_faces() {
        let cases = [
            ("Helvetica", false, false, "F_ST_Helv", "Helvetica"),
            ("arial", true, false, "F_ST_HeBo", "Helvetica-Bold"),
            ("Times-Roman", false, false, "F_ST_TimR", "Times-Roman"),
            ("Courier New", true, true, "F_ST_CoBO", "Courier-BoldOblique"),
        ];
        for (name, bold, italic, alias, base) in cases {
            let style = ParagraphStyle {
                family: Some(name.to_string()),
                bold: Some(bold),
                italic: Some(italic),
                ..ParagraphStyle::default()
            };
            let prepared = prepare_current(&style, "", 400, "normal").unwrap();
            assert_eq!(prepared.font_resource, alias, "{name}");
            assert_eq!(prepared.base_font.as_deref(), Some(base), "{name}");
        }
    }

    #[test]
    fn unstyled_edit_matches_the_encoder_path_and_omits_color() {
        let source = "BT\n/F1 12 Tf\n50 700 Tm\n(Original Draft Agreement) Tj\nET\n";
        let mut tokenizer = ContentStreamTokenizer::new(source.as_bytes());
        let ops = tokenizer.tokenize_all().unwrap();
        let mut ast = build_ast_from_operations(ops);
        let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
        let paragraphs = LayoutReconstructor::new(&ast)
            .with_font("F1", metrics.clone())
            .reconstruct()
            .unwrap();
        let mut styled = ast.clone();
        let prepared = ParagraphStyle::default()
            .prepare("F1", 12.0, paragraphs[0].leading, paragraphs[0].alignment, "", 400, "normal")
            .unwrap();
        SurgicalEditor::edit_paragraph(&mut ast, &paragraphs[0], "Amended terms.", &metrics).unwrap();
        SurgicalEditor::edit_paragraph_prepared(
            &mut styled,
            &paragraphs[0],
            "Amended terms.",
            &metrics,
            &prepared,
        )
        .unwrap();
        let plain = serialize_ast(&ast);
        assert_eq!(plain, serialize_ast(&styled));
        assert!(plain.starts_with(b"BT\n"));
        assert!(plain.ends_with(b"ET\n"));
        assert!(!plain.windows(4).any(|window| window == b" rg\n"));
        assert!(!plain.windows(3).any(|window| window == b"\nq\n"));
    }

    #[test]
    fn styled_edit_writes_the_face_size_color_and_centered_lines() {
        let source = "BT\n/F1 12 Tf\n50 700 Tm\n(Contract Agreement Terms And Conditions) Tj\nET\n";
        let style = ParagraphStyle {
            family: Some("Helvetica".to_string()),
            size: Some(18.0),
            bold: Some(true),
            color: Some([1.0, 0.0, 0.0]),
            alignment: Some(TextAlignment::Center),
            leading: Some(20.0),
            ..ParagraphStyle::default()
        };
        let (serialized, paragraphs) = edit_stream(source, "Hi\nHello there", &style);
        assert!(serialized.contains("q\n"));
        assert!(serialized.contains("1 0 0 rg\n"));
        assert!(serialized.contains("/F_ST_HeBo 18 Tf\n"));
        assert!(serialized.contains("Q\n"));
        assert!(serialized.contains("(Hi)"));
        assert!(serialized.contains("(Hello there)"));
        assert!(!serialized.contains("Contract Agreement"));
        assert_eq!(paragraphs.len(), 1, "{serialized}");
        assert_eq!(paragraphs[0].alignment, TextAlignment::Center);
        assert!(paragraphs[0].text().contains("Hi"));
        assert!(paragraphs[0].text().contains("Hello there"));
        assert!((paragraphs[0].leading - 20.0).abs() < 0.01);
        assert!((paragraphs[0].rendered_size() - 18.0).abs() < 0.01);
        assert_eq!(paragraphs[0].font_resource(), "F_ST_HeBo");
    }
}
