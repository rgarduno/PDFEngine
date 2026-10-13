//! Content-stream text drawn inside a rectangle.
//!
//! The text is appended to the page content. It is not a `/FreeText` annotation
//! and not a sticky `/Text` note, so it cannot be dragged as an annotation.
//! After the page is read again it is a normal paragraph. A real viewer uses
//! the standard 14 face. This engine's raster has no outlines for that face
//! and paints the same built-in 5 by 7 glyphs it uses for other outline-less text.

use crate::cos::{PdfDocument, PdfName, PdfObject, PdfString};
use crate::editor::reflow::ReflowEngine;
use crate::editor::style::{ParagraphStyle, PreparedParagraphStyle};
use crate::error::{PdfError, PdfResult};
use crate::fonts::{FontEncoder, FontMetrics};
use crate::layout::geometry::Rect;
use crate::layout::TextAlignment;
use crate::stream::graphics_state::TextState;
use crate::stream::{serialize_ast, ContentAst, ContentNode, Operation};
use crate::watermark::helpers::{append_content_ops, ensure_font_resource};

const MAX_TEXT_SCALARS: usize = 4000;
const MIN_SIDE: f64 = 8.0;
const MAX_COORDINATE: f64 = 20000.0;
const DEFAULT_SIZE: f64 = 12.0;

/// Where to draw, and the optional style. Omitted style fields become
/// Helvetica, 12 points, black, left, and a leading of 1.2 times the size.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBoxSpec {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub style: ParagraphStyle,
}

/// Font registered for the box and how many lines were painted.
#[derive(Debug, Clone, PartialEq)]
pub struct AppendedTextBox {
    /// Resource name used by `Tf`, without a leading slash.
    pub font_resource: String,
    /// Standard 14 `/BaseFont`.
    pub base_font: String,
    /// Lines whose baseline still falls inside the rectangle.
    pub line_count: usize,
}

/// Appends text to one page. `page_index` is 0-based.
///
/// Lines that start below the rectangle are omitted. The graphics state is
/// saved and restored, and the fill color is device RGB outside the text object.
pub fn append_text_box(
    doc: &mut PdfDocument,
    page_index: usize,
    spec: &TextBoxSpec,
) -> PdfResult<AppendedTextBox> {
    reject_text(&spec.text)?;
    reject_rectangle(spec.x, spec.y, spec.width, spec.height)?;
    let prepared = resolve_style(&spec.style)?;
    let rect = Rect::from_origin_size(spec.x, spec.y, spec.width, spec.height);
    if rect.height() < prepared.size {
        return Err(box_error("The rectangle is shorter than the font size."));
    }

    let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
    let mut text_state = TextState::default();
    text_state.font_name = prepared.font_resource.clone();
    text_state.font_size = prepared.size;
    let reflowed = ReflowEngine::reflow(
        &spec.text,
        rect.width(),
        &metrics,
        &text_state,
        prepared.alignment,
        prepared.leading,
    );
    let first_baseline = rect.max_y - (0.8 * prepared.size);
    let painted: Vec<_> = reflowed
        .into_iter()
        .filter(|line| first_baseline + line.offset_y >= rect.min_y)
        .collect();
    if painted.is_empty() {
        return Err(box_error("The rectangle is too short for one line."));
    }
    let line_count = painted.len();
    let color = prepared.color.unwrap_or([0.0, 0.0, 0.0]);
    let ops = serialize_ops(&operations_for(
        &painted,
        rect.min_x,
        first_baseline,
        &prepared.font_resource,
        prepared.size,
        color,
    ));

    let pages = doc.get_pages()?;
    if page_index >= pages.len() {
        return Err(box_error("Page index is out of range."));
    }
    let page_id = pages[page_index];
    let base_font = prepared
        .base_font
        .clone()
        .ok_or_else(|| box_error("The standard face was not registered."))?;
    ensure_font_resource(doc, page_id, &prepared.font_resource, &base_font)?;
    append_content_ops(doc, page_id, &ops)?;
    Ok(AppendedTextBox {
        font_resource: prepared.font_resource,
        base_font,
        line_count,
    })
}

fn box_error(message: &str) -> PdfError {
    PdfError::OperationError(format!("Text box: {message}"))
}

fn reject_text(text: &str) -> PdfResult<()> {
    if text.trim().is_empty() {
        return Err(box_error("Text is empty."));
    }
    if text.chars().count() > MAX_TEXT_SCALARS {
        return Err(box_error("Text is longer than 4000 characters."));
    }
    if text.chars().any(|ch| (ch as u32) < 32 && ch != '\n') {
        return Err(box_error("Text contains a control character."));
    }
    Ok(())
}

fn reject_rectangle(x: f64, y: f64, width: f64, height: f64) -> PdfResult<()> {
    let far_x = x + width;
    let far_y = y + height;
    if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() || !far_x.is_finite() || !far_y.is_finite()
    {
        return Err(box_error("The rectangle is not finite."));
    }
    if width < MIN_SIDE || height < MIN_SIDE {
        return Err(box_error(
            "The rectangle must be at least 8 points wide and 8 points tall.",
        ));
    }
    if [x, y, far_x, far_y]
        .iter()
        .any(|value| value.abs() > MAX_COORDINATE)
    {
        return Err(box_error(
            "A rectangle coordinate is outside -20000 to 20000.",
        ));
    }
    Ok(())
}

/// Fills the text-box defaults, then resolves a standard 14 face.
fn resolve_style(style: &ParagraphStyle) -> PdfResult<PreparedParagraphStyle> {
    let mut filled = style.clone();
    if filled.family.is_none() {
        filled.family = Some("Helvetica".to_string());
    }
    if filled.size.is_none() {
        filled.size = Some(DEFAULT_SIZE);
    }
    if filled.color.is_none() {
        filled.color = Some([0.0, 0.0, 0.0]);
    }
    let size = filled.size.unwrap_or(DEFAULT_SIZE);
    filled.prepare(
        "F_ST_Helv",
        size,
        size * 1.2,
        TextAlignment::Left,
        "Arial",
        400,
        "normal",
    )
}

fn operations_for(
    lines: &[crate::editor::reflow::ReflowLine],
    origin_x: f64,
    first_baseline: f64,
    font_resource: &str,
    size: f64,
    color: [f64; 3],
) -> Vec<Operation> {
    let encoder = FontEncoder::new();
    let mut ops = Vec::new();
    ops.push(Operation::new("q", Vec::new()));
    ops.push(Operation::new(
        "rg",
        vec![
            PdfObject::Real(color[0]),
            PdfObject::Real(color[1]),
            PdfObject::Real(color[2]),
        ],
    ));
    ops.push(Operation::new("BT", Vec::new()));
    ops.push(Operation::new(
        "Tf",
        vec![
            PdfObject::Name(PdfName::new(font_resource)),
            PdfObject::Real(size),
        ],
    ));
    for line in lines {
        if line.word_spacing > 0.0 {
            ops.push(Operation::new("Tw", vec![PdfObject::Real(line.word_spacing)]));
        }
        ops.push(Operation::new(
            "Tm",
            vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
                PdfObject::Real(origin_x + line.offset_x),
                PdfObject::Real(first_baseline + line.offset_y),
            ],
        ));
        ops.push(Operation::new(
            "Tj",
            vec![PdfObject::String(PdfString::literal(
                encoder.encode_string(&line.text),
            ))],
        ));
        if line.word_spacing > 0.0 {
            ops.push(Operation::new("Tw", vec![PdfObject::Real(0.0)]));
        }
    }
    ops.push(Operation::new("ET", Vec::new()));
    ops.push(Operation::new("Q", Vec::new()));
    ops
}

fn serialize_ops(ops: &[Operation]) -> String {
    let mut ast = ContentAst::new();
    for op in ops {
        let id = ast.alloc_id();
        ast.nodes.push(ContentNode::Instruction {
            id,
            operation: op.clone(),
        });
    }
    String::from_utf8_lossy(&serialize_ast(&ast)).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::LayoutReconstructor;
    use crate::stream::{build_ast_from_operations, ContentStreamTokenizer};

    fn sample_pdf() -> Vec<u8> {
        pdf_with_stream(b"BT\n/F1 12 Tf\n72 700 Td\n(Page One) Tj\nET\n")
    }

    fn pdf_with_stream(stream: &[u8]) -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        let offset1 = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let offset2 = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        let offset3 = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n",
        );
        let offset4 = pdf.len();
        pdf.extend_from_slice(
            format!("4 0 obj\n<< /Length {} >>\nstream\n", stream.len()).as_bytes(),
        );
        pdf.extend_from_slice(stream);
        pdf.extend_from_slice(b"endstream\nendobj\n");
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in [offset1, offset2, offset3, offset4] {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n");
        pdf.extend_from_slice(format!("{xref}\n%%EOF").as_bytes());
        pdf
    }

    fn spec_at(text: &str, width: f64, height: f64, style: ParagraphStyle) -> TextBoxSpec {
        TextBoxSpec {
            text: text.to_string(),
            x: 72.0,
            y: 400.0,
            width,
            height,
            style,
        }
    }

    fn message(error: PdfError) -> String {
        error.to_string()
    }

    #[test]
    fn text_box_rejects_blank_text_controls_and_a_bad_rectangle() {
        let mut doc = PdfDocument::load(&sample_pdf()).unwrap();
        let blank = append_text_box(&mut doc, 0, &spec_at("  \n", 80.0, 40.0, ParagraphStyle::default()));
        assert!(message(blank.unwrap_err()).contains("Text box: Text is empty."));

        let mut doc = PdfDocument::load(&sample_pdf()).unwrap();
        let control = append_text_box(&mut doc, 0, &spec_at("Hello\tthere", 80.0, 40.0, ParagraphStyle::default()));
        assert!(message(control.unwrap_err()).contains("Text box: Text contains a control character."));

        let mut doc = PdfDocument::load(&sample_pdf()).unwrap();
        let long = "A".repeat(4001);
        let too_long = append_text_box(&mut doc, 0, &spec_at(&long, 80.0, 40.0, ParagraphStyle::default()));
        assert!(message(too_long.unwrap_err()).contains("Text box: Text is longer than 4000 characters."));

        let mut doc = PdfDocument::load(&sample_pdf()).unwrap();
        let narrow = append_text_box(&mut doc, 0, &spec_at("Note", 4.0, 40.0, ParagraphStyle::default()));
        assert!(message(narrow.unwrap_err()).contains("at least 8 points wide"));

        let mut doc = PdfDocument::load(&sample_pdf()).unwrap();
        let outside = append_text_box(
            &mut doc,
            0,
            &TextBoxSpec {
                text: "Note".to_string(),
                x: 19990.0,
                y: 40.0,
                width: 20.0,
                height: 20.0,
                style: ParagraphStyle::default(),
            },
        );
        assert!(message(outside.unwrap_err()).contains("outside -20000 to 20000"));

        let mut doc = PdfDocument::load(&sample_pdf()).unwrap();
        let short = append_text_box(
            &mut doc,
            0,
            &spec_at(
                "Note",
                80.0,
                20.0,
                ParagraphStyle {
                    size: Some(24.0),
                    ..ParagraphStyle::default()
                },
            ),
        );
        assert!(message(short.unwrap_err()).contains("shorter than the font size"));

        let mut doc = PdfDocument::load(&sample_pdf()).unwrap();
        let pushed = append_text_box(&mut doc, 0, &spec_at("\nHello", 80.0, 12.0, ParagraphStyle::default()));
        assert!(message(pushed.unwrap_err()).contains("too short for one line"));

        let mut doc = PdfDocument::load(&sample_pdf()).unwrap();
        let missing = append_text_box(&mut doc, 3, &spec_at("Note", 80.0, 40.0, ParagraphStyle::default()));
        assert!(message(missing.unwrap_err()).contains("Page index is out of range"));
    }

    #[test]
    fn text_box_appends_a_standard_face_and_keeps_the_original_page() {
        let mut doc = PdfDocument::load(&sample_pdf()).unwrap();
        let style = ParagraphStyle {
            family: Some("Times".to_string()),
            italic: Some(true),
            color: Some([0.0, 0.0, 1.0]),
            alignment: Some(TextAlignment::Center),
            leading: Some(16.0),
            ..ParagraphStyle::default()
        };
        let appended = append_text_box(&mut doc, 0, &spec_at("Hello PDF", 40.0, 48.0, style)).unwrap();
        assert_eq!(appended.font_resource, "F_ST_TiIt");
        assert_eq!(appended.base_font, "Times-Italic");
        assert_eq!(appended.line_count, 2);

        let saved = doc.save_to_vec().unwrap();
        let saved_text = String::from_utf8_lossy(&saved);
        assert!(!saved_text.contains("/FreeText"));
        assert!(!saved_text.contains("/Annots"));
        assert!(saved_text.contains("/Times-Italic"));
        assert!(saved_text.contains("/F_ST_TiIt"));
        let mut reloaded = PdfDocument::load(&saved).unwrap();
        let pages = reloaded.get_pages().unwrap();
        let content = reloaded.get_page_content_bytes(pages[0]).unwrap();
        let text = String::from_utf8_lossy(&content);
        assert!(text.contains("(Page One)"));
        assert!(text.contains("(Hello)"));
        assert!(text.contains("(PDF)"));
        assert!(text.contains("/F_ST_TiIt"));
        assert!(text.contains("0 0 1 rg"));
        assert!(text.contains("\nq\n"));
        assert!(text.contains("\nQ\n"));

        let mut tokenizer = ContentStreamTokenizer::new(content.as_slice());
        let ops = tokenizer.tokenize_all().unwrap();
        let ast = build_ast_from_operations(ops);
        let paragraphs = LayoutReconstructor::new(&ast).reconstruct().unwrap();
        let drawn = paragraphs
            .iter()
            .find(|paragraph| paragraph.text().contains("Hello"))
            .expect("text box paragraph");
        assert!(drawn.text().contains("PDF"));
        assert_eq!(drawn.alignment, TextAlignment::Center);
        assert_eq!(drawn.font_resource(), "F_ST_TiIt");
        assert!(paragraphs.iter().any(|paragraph| paragraph.text().contains("Page One")));
    }

    #[test]
    fn text_box_drops_a_line_that_falls_below_the_rectangle() {
        let mut doc = PdfDocument::load(&sample_pdf()).unwrap();
        let style = ParagraphStyle {
            leading: Some(16.0),
            ..ParagraphStyle::default()
        };
        let appended = append_text_box(&mut doc, 0, &spec_at("Hello PDF", 40.0, 20.0, style)).unwrap();
        assert_eq!(appended.line_count, 1);
        assert_eq!(appended.font_resource, "F_ST_Helv");
        assert_eq!(appended.base_font, "Helvetica");
        let saved = String::from_utf8_lossy(&doc.save_to_vec().unwrap()).into_owned();
        assert!(saved.contains("(Hello)"));
        assert!(!saved.contains("(PDF)"));
        assert!(saved.contains("0 0 0 rg"));
        assert!(saved.contains("/Helvetica"));
        assert!(saved.contains("/F_ST_Helv"));
        assert!(!saved.contains("/FreeText"));
    }
}
