//! Typed, drawn, or image signature and initials as page content.
//!
//! The mark is appended to the page content stream. It is not an `/Ink`
//! annotation, not an AcroForm signature field, and not a PKCS#7 signature.
//! A later full save still rewrites the file. This does not preserve `/ByteRange`.

use crate::editor::textbox::{append_text_box, TextBoxSpec};
use crate::editor::ParagraphStyle;
use crate::error::{PdfError, PdfResult};
use crate::fonts::WinAnsiEncoding;
use crate::images::create_image_xobject;
use crate::watermark::helpers::{append_content_ops, ensure_xobject_resource};

const MIN_SIDE: f64 = 8.0;
const MAX_COORDINATE: f64 = 20000.0;
const MAX_SIGNATURE_SCALARS: usize = 80;
const MAX_INITIALS_SCALARS: usize = 8;
const MAX_STROKES: usize = 30;
const MAX_POINTS_PER_STROKE: usize = 400;
const MAX_POINTS: usize = 2000;
const MAX_IMAGE_BYTES: usize = 2_000_000;
const STROKE_WIDTH: f64 = 1.25;

/// A signature is a name. Initials are a short mark in the same three modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillSignKind {
    Signature,
    Initials,
}

/// One mark. The caller picks exactly one mode.
#[derive(Debug, Clone, PartialEq)]
pub enum FillSignMark {
    /// Times-Italic text. A newline is not allowed.
    Typed(String),
    /// Strokes in page user space. Each stroke has at least two points.
    Drawn(Vec<Vec<(f64, f64)>>),
    /// JPEG or PNG bytes, painted once inside the rectangle.
    Image(Vec<u8>),
}

/// Where to paint the mark, and which mark.
#[derive(Debug, Clone, PartialEq)]
pub struct FillSignSpec {
    pub kind: FillSignKind,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub mark: FillSignMark,
}

/// What was appended. The typed text and the image bytes are not copied here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendedFillSign {
    /// `signature` or `initials`.
    pub kind: String,
    /// `typed`, `drawn`, or `image`.
    pub mode: String,
    /// Font resource, `path`, or the image XObject name. No leading slash.
    pub resource: String,
}

/// Appends one fill-and-sign mark. `page_index` is 0-based.
///
/// A failure restores the document. The mark is content, not a signature field.
pub fn append_fill_sign(
    doc: &mut crate::cos::PdfDocument,
    page_index: usize,
    spec: &FillSignSpec,
) -> PdfResult<AppendedFillSign> {
    let snapshot = doc.clone();
    match write_mark(doc, page_index, spec) {
        Ok(report) => Ok(report),
        Err(err) => {
            *doc = snapshot;
            Err(err)
        }
    }
}

fn write_mark(
    doc: &mut crate::cos::PdfDocument,
    page_index: usize,
    spec: &FillSignSpec,
) -> PdfResult<AppendedFillSign> {
    reject_rectangle(spec.x, spec.y, spec.width, spec.height)?;
    let pages = doc.get_pages()?;
    if page_index >= pages.len() {
        return Err(sign_error("Page index is out of range."));
    }
    let page_id = pages[page_index];
    let kind = match spec.kind {
        FillSignKind::Signature => "signature",
        FillSignKind::Initials => "initials",
    };
    let (mode, resource) = match &spec.mark {
        FillSignMark::Typed(text) => {
            reject_typed(spec.kind, text)?;
            let appended = append_typed(doc, page_index, spec, text).map_err(retarget)?;
            ("typed", appended.font_resource)
        }
        FillSignMark::Drawn(strokes) => {
            let ops = drawn_ops(spec, strokes)?;
            append_content_ops(doc, page_id, &ops)?;
            ("drawn", "path".to_string())
        }
        FillSignMark::Image(bytes) => {
            let name = image_ops(doc, page_id, spec, bytes)?;
            ("image", name)
        }
    };
    Ok(AppendedFillSign {
        kind: kind.to_string(),
        mode: mode.to_string(),
        resource,
    })
}

fn sign_error(message: &str) -> PdfError {
    PdfError::OperationError(format!("Fill and sign: {message}"))
}

fn retarget(err: PdfError) -> PdfError {
    match err {
        PdfError::OperationError(message) => {
            let detail = message
                .strip_prefix("Text box: ")
                .or_else(|| message.strip_prefix("Style: "))
                .unwrap_or(&message);
            sign_error(detail)
        }
        other => other,
    }
}

fn reject_rectangle(x: f64, y: f64, width: f64, height: f64) -> PdfResult<()> {
    let far_x = x + width;
    let far_y = y + height;
    if ![x, y, width, height, far_x, far_y]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(sign_error("The rectangle is not finite."));
    }
    if width < MIN_SIDE || height < MIN_SIDE {
        return Err(sign_error(
            "The rectangle must be at least 8 points wide and 8 points tall.",
        ));
    }
    if [x, y, far_x, far_y]
        .iter()
        .any(|value| value.abs() > MAX_COORDINATE)
    {
        return Err(sign_error(
            "A rectangle coordinate is outside -20000 to 20000.",
        ));
    }
    Ok(())
}

fn reject_typed(kind: FillSignKind, text: &str) -> PdfResult<()> {
    if text.trim().is_empty() {
        return Err(sign_error("Text is empty."));
    }
    let limit = match kind {
        FillSignKind::Signature => MAX_SIGNATURE_SCALARS,
        FillSignKind::Initials => MAX_INITIALS_SCALARS,
    };
    if text.chars().count() > limit {
        return Err(sign_error(match kind {
            FillSignKind::Signature => "A signature is longer than 80 characters.",
            FillSignKind::Initials => "Initials are longer than 8 characters.",
        }));
    }
    if text.chars().any(|ch| (ch as u32) < 32) {
        return Err(sign_error("The mark contains a control character."));
    }
    if text.chars().any(|ch| WinAnsiEncoding::unicode_to_code(ch).is_none()) {
        return Err(sign_error("A character is not in the standard fonts."));
    }
    Ok(())
}

fn append_typed(
    doc: &mut crate::cos::PdfDocument,
    page_index: usize,
    spec: &FillSignSpec,
    text: &str,
) -> PdfResult<crate::editor::AppendedTextBox> {
    let size = (spec.height * 0.6).clamp(1.0, 72.0);
    append_text_box(
        doc,
        page_index,
        &TextBoxSpec {
            text: text.to_string(),
            x: spec.x,
            y: spec.y,
            width: spec.width,
            height: spec.height,
            style: ParagraphStyle {
                family: Some("Times".to_string()),
                size: Some(size),
                bold: Some(false),
                italic: Some(true),
                color: None,
                alignment: None,
                leading: Some(size * 1.2),
            },
        },
    )
}

fn drawn_ops(spec: &FillSignSpec, strokes: &[Vec<(f64, f64)>]) -> PdfResult<String> {
    if strokes.is_empty() {
        return Err(sign_error("A drawn mark needs one stroke."));
    }
    if strokes.len() > MAX_STROKES {
        return Err(sign_error("A drawn mark has more than 30 strokes."));
    }
    let mut total = 0usize;
    for stroke in strokes {
        if stroke.len() < 2 {
            return Err(sign_error("A drawn stroke needs two points."));
        }
        if stroke.len() > MAX_POINTS_PER_STROKE {
            return Err(sign_error("A drawn stroke has more than 400 points."));
        }
        total += stroke.len();
        if total > MAX_POINTS {
            return Err(sign_error("A drawn mark has more than 2000 points."));
        }
        for (x, y) in stroke {
            if !x.is_finite() || !y.is_finite() {
                return Err(sign_error("A drawn point is not finite."));
            }
            if *x < spec.x || *y < spec.y || *x > spec.x + spec.width || *y > spec.y + spec.height
            {
                return Err(sign_error("A drawn point is outside the rectangle."));
            }
        }
    }
    let mut ops = format!(
        "q\n{:.2} {:.2} {:.2} {:.2} re\nW\nn\n{:.2} w\n0 0 0 RG\n",
        spec.x, spec.y, spec.width, spec.height, STROKE_WIDTH
    );
    for stroke in strokes {
        let (x0, y0) = stroke[0];
        ops.push_str(&format!("{x0:.2} {y0:.2} m\n"));
        for (x, y) in stroke.iter().skip(1) {
            ops.push_str(&format!("{x:.2} {y:.2} l\n"));
        }
        ops.push_str("S\n");
    }
    ops.push_str("Q\n");
    Ok(ops)
}

fn image_ops(
    doc: &mut crate::cos::PdfDocument,
    page_id: crate::cos::ObjectId,
    spec: &FillSignSpec,
    bytes: &[u8],
) -> PdfResult<String> {
    if bytes.is_empty() {
        return Err(sign_error("The signature image is empty."));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(sign_error(
            "The signature image is larger than 2000000 bytes.",
        ));
    }
    let (image_id, _, _) = create_image_xobject(doc, bytes).map_err(|_| {
        sign_error("The signature image must be a JPEG or PNG.")
    })?;
    let name = format!("FSIG_{}", image_id.number);
    ensure_xobject_resource(doc, page_id, &name, image_id)?;
    let ops = format!(
        "q\n{:.2} 0 0 {:.2} {:.2} {:.2} cm\n/{name} Do\nQ\n",
        spec.width, spec.height, spec.x, spec.y
    );
    append_content_ops(doc, page_id, &ops)?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cos::PdfDocument;

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

    fn load() -> PdfDocument {
        PdfDocument::load(&pdf_with_stream(b"BT\n/F1 12 Tf\n72 700 Td\n(Page One) Tj\nET\n"))
            .unwrap()
    }

    fn message(error: PdfError) -> String {
        error.to_string()
    }

    fn warmed(doc: &mut PdfDocument) -> Vec<u8> {
        let _ = doc.save_to_vec().unwrap();
        doc.save_to_vec().unwrap()
    }

    fn tiny_jpeg() -> Vec<u8> {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xC0];
        jpeg.extend_from_slice(&8u16.to_be_bytes());
        jpeg.push(8);
        jpeg.extend_from_slice(&1u16.to_be_bytes());
        jpeg.extend_from_slice(&1u16.to_be_bytes());
        jpeg.push(3);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        jpeg
    }

    fn box_spec(kind: FillSignKind, mark: FillSignMark) -> FillSignSpec {
        FillSignSpec {
            kind,
            x: 72.0,
            y: 80.0,
            width: 200.0,
            height: 48.0,
            mark,
        }
    }

    #[test]
    fn typed_signature_is_times_italic_content() {
        let mut doc = load();
        let report = append_fill_sign(
            &mut doc,
            0,
            &box_spec(
                FillSignKind::Signature,
                FillSignMark::Typed("Rafael G".to_string()),
            ),
        )
        .unwrap();
        assert_eq!(report.kind, "signature");
        assert_eq!(report.mode, "typed");
        assert_eq!(report.resource, "F_ST_TiIt");
        let saved = doc.save_to_vec().unwrap();
        let text = String::from_utf8_lossy(&saved);
        assert!(text.contains("(Rafael G)"));
        assert!(text.contains("/Times-Italic"));
        assert!(text.contains("/F_ST_TiIt"));
        assert!(!text.contains("/Ink"));
        assert!(!text.contains("/Annots"));
        assert!(!text.contains("/ByteRange"));
        assert!(!text.contains("/Sig"));
    }

    #[test]
    fn initials_reject_a_long_or_undrawable_mark_without_writing() {
        let mut doc = load();
        let before = warmed(&mut doc);
        let long = append_fill_sign(
            &mut doc,
            0,
            &box_spec(
                FillSignKind::Initials,
                FillSignMark::Typed("TOO LONG!".to_string()),
            ),
        )
        .unwrap_err();
        assert!(message(long).contains("Initials are longer than 8 characters."));
        assert_eq!(doc.save_to_vec().unwrap(), before);

        let omega = append_fill_sign(
            &mut doc,
            0,
            &box_spec(
                FillSignKind::Signature,
                FillSignMark::Typed("Rafael \u{03a9}".to_string()),
            ),
        )
        .unwrap_err();
        assert!(message(omega).contains("is not in the standard fonts"));
        assert_eq!(doc.save_to_vec().unwrap(), before);

        let ok = append_fill_sign(
            &mut doc,
            0,
            &box_spec(FillSignKind::Initials, FillSignMark::Typed("RG".to_string())),
        )
        .unwrap();
        assert_eq!(ok.kind, "initials");
        assert!(String::from_utf8_lossy(&doc.save_to_vec().unwrap()).contains("(RG)"));
    }

    #[test]
    fn drawn_mark_is_a_content_path_inside_the_rectangle() {
        let mut doc = load();
        let report = append_fill_sign(
            &mut doc,
            0,
            &box_spec(
                FillSignKind::Signature,
                FillSignMark::Drawn(vec![vec![(80.0, 90.0), (180.0, 110.0)]]),
            ),
        )
        .unwrap();
        assert_eq!(report.mode, "drawn");
        assert_eq!(report.resource, "path");
        let text = String::from_utf8_lossy(&doc.save_to_vec().unwrap()).to_string();
        assert!(text.contains("80.00 90.00 m\n"));
        assert!(text.contains("180.00 110.00 l\n"));
        assert!(text.contains("S\n"));
        assert!(!text.contains("/Ink"));
        assert!(!text.contains("/Annots"));

        let mut fresh = load();
        let before = warmed(&mut fresh);
        let outside = append_fill_sign(
            &mut fresh,
            0,
            &box_spec(
                FillSignKind::Signature,
                FillSignMark::Drawn(vec![vec![(10.0, 90.0), (180.0, 110.0)]]),
            ),
        )
        .unwrap_err();
        assert!(message(outside).contains("outside the rectangle"));
        assert_eq!(fresh.save_to_vec().unwrap(), before);
    }

    #[test]
    fn image_mark_paints_one_xobject_and_a_bad_file_writes_nothing() {
        let mut doc = load();
        let report = append_fill_sign(
            &mut doc,
            0,
            &box_spec(FillSignKind::Signature, FillSignMark::Image(tiny_jpeg())),
        )
        .unwrap();
        assert_eq!(report.mode, "image");
        assert!(report.resource.starts_with("FSIG_"));
        let text = String::from_utf8_lossy(&doc.save_to_vec().unwrap()).to_string();
        assert!(text.contains(&format!("/{} Do", report.resource)));
        assert!(text.contains("/DCTDecode"));
        assert!(!text.contains("/Annots"));
        assert!(!text.contains("/ByteRange"));

        let mut fresh = load();
        let before = warmed(&mut fresh);
        let bad = append_fill_sign(
            &mut fresh,
            0,
            &box_spec(
                FillSignKind::Initials,
                FillSignMark::Image(b"not an image".to_vec()),
            ),
        )
        .unwrap_err();
        assert!(message(bad).contains("must be a JPEG or PNG"));
        assert_eq!(fresh.save_to_vec().unwrap(), before);
    }
}
