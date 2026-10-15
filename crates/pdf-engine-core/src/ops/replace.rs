//! Find-and-replace over reconstructed paragraph text.
//!
//! A subset font is kept only when every character of the new paragraph,
//! including a space a line break needs, round-trips to a byte that has a real
//! width. Otherwise that paragraph moves to Helvetica, Times, or Courier.
//! A character those faces cannot draw aborts the replace and writes nothing.
//! This does not add glyphs to the embedded subset.
//!
//! The hit cap can stop inside a paragraph: only the matches collected before
//! the cap are rewritten. Later pages are not scanned and are not skipped.
//! An empty replacement deletes the match. A newline is allowed. The whole
//! paragraph is rewritten in the first span's face, and color is not preserved.

use std::collections::BTreeMap;

use crate::cos::{ObjectId, PdfDictionary, PdfDocument, PdfObject, PdfStream};
use crate::editor::{ParagraphStyle, SurgicalEditor};
use crate::error::{PdfError, PdfResult};
use crate::fonts::{
    resolve_page_fonts, FontEncoder, FontMetrics, ResolvedFont, ToUnicodeMap, WinAnsiEncoding,
};
use crate::layout::{LayoutReconstructor, ParagraphBlock, TextAlignment};
use crate::ops::layers::resolve_page_hidden_ocg_props;
use crate::ops::search::{is_skippable, locate_matches, MAX_QUERY_SCALARS, MAX_SEARCH_HITS};
use crate::stream::{serialize_ast, ContentAst, ContentParser};
use crate::watermark::helpers::ensure_font_resource;

const MAX_REPLACEMENT_SCALARS: usize = 4000;
const MAX_PARAGRAPH_SCALARS: usize = 16000;

/// Counts for one find-and-replace.
///
/// `truncated` is true when the scan stopped because the match count reached
/// `max_hits`, including when that match was the last one in the document.
/// `pages_skipped` uses 0-based page indexes.
#[derive(Debug, Clone, PartialEq)]
pub struct TextReplaceReport {
    /// Matches rewritten. A cap can omit later matches in the same paragraph.
    pub replacements: usize,
    /// Paragraphs rewritten. Each paragraph is edited once.
    pub paragraphs: usize,
    /// Paragraphs moved onto Helvetica, Times, or Courier.
    pub paragraphs_on_standard_face: usize,
    /// True when the match cap stopped the scan.
    pub truncated: bool,
    /// 0-based indexes of pages skipped after an ordinary parse, layout, or font failure.
    pub pages_skipped: Vec<usize>,
}

struct PagePlan {
    page_id: ObjectId,
    ast: ContentAst,
    edits: Vec<ParagraphEdit>,
}

struct ParagraphEdit {
    paragraph: ParagraphBlock,
    new_text: String,
    metrics: FontMetrics,
    encoder: FontEncoder,
    prepared: crate::editor::PreparedParagraphStyle,
}

struct KeptFace {
    metrics: FontMetrics,
    encoder: FontEncoder,
    prepared: crate::editor::PreparedParagraphStyle,
}

/// Replaces reconstructed paragraph text.
///
/// `max_hits` must be in `1..=500`. The query must be non-empty and at most
/// 512 Unicode scalars. The replacement may be empty and is at most 4000
/// scalars. A newline is allowed. Other C0 controls are rejected.
///
/// The document is unchanged when there is no match, when a character cannot
/// be drawn, or when any other check fails.
pub fn replace_document_text(
    doc: &mut PdfDocument,
    query: &str,
    replacement: &str,
    ignore_case: bool,
    max_hits: usize,
) -> PdfResult<TextReplaceReport> {
    if query.is_empty() {
        return Err(marked("The search query is empty."));
    }
    if query.chars().count() > MAX_QUERY_SCALARS {
        return Err(marked(format!(
            "The search query is longer than {MAX_QUERY_SCALARS} characters."
        )));
    }
    if max_hits == 0 || max_hits > MAX_SEARCH_HITS {
        return Err(marked(format!(
            "max_hits must be from 1 to {MAX_SEARCH_HITS}."
        )));
    }
    if replacement.chars().count() > MAX_REPLACEMENT_SCALARS {
        return Err(marked(format!(
            "The replacement is longer than {MAX_REPLACEMENT_SCALARS} characters."
        )));
    }
    if replacement
        .chars()
        .any(|ch| (ch as u32) < 32 && ch != '\n')
    {
        return Err(marked("The replacement contains a control character."));
    }

    let page_ids = doc.get_pages()?;
    let mut plans = Vec::new();
    let mut pages_skipped = Vec::new();
    let mut replacements = 0usize;
    let mut paragraph_count = 0usize;
    let mut standard_count = 0usize;
    let mut truncated = false;

    for (page_index, page_id) in page_ids.into_iter().enumerate() {
        let (ast, paragraphs, faces) = match load_page(doc, page_id) {
            Ok(loaded) => loaded,
            Err(err) if is_skippable(&err) => {
                pages_skipped.push(page_index);
                continue;
            }
            Err(err) => return Err(err),
        };

        let mut page_edits = Vec::new();
        let mut stop = false;
        for paragraph in paragraphs {
            let text = paragraph.text();
            let matches = locate_matches(&text, query, ignore_case);
            if matches.is_empty() {
                continue;
            }

            let mut taken = Vec::new();
            for hit in matches {
                taken.push(hit);
                replacements += 1;
                if replacements == max_hits {
                    truncated = true;
                    break;
                }
            }

            let new_text = splice(&text, &taken, replacement)?;
            if new_text.chars().count() > MAX_PARAGRAPH_SCALARS {
                return Err(marked(format!(
                    "The paragraph would be longer than {MAX_PARAGRAPH_SCALARS} characters."
                )));
            }

            let edit = plan_edit(page_index, &paragraph, &faces, new_text)?;
            if edit.on_standard {
                standard_count += 1;
            }
            paragraph_count += 1;
            page_edits.push(edit.edit);
            if truncated {
                stop = true;
                break;
            }
        }

        if !page_edits.is_empty() {
            plans.push(PagePlan {
                page_id,
                ast,
                edits: page_edits,
            });
        }
        if stop {
            break;
        }
    }

    let report = TextReplaceReport {
        replacements,
        paragraphs: paragraph_count,
        paragraphs_on_standard_face: standard_count,
        truncated,
        pages_skipped,
    };
    if plans.is_empty() {
        return Ok(report);
    }

    let snapshot = doc.clone();
    if let Err(error) = apply(doc, plans) {
        *doc = snapshot;
        return Err(error);
    }
    Ok(report)
}

struct PlannedEdit {
    edit: ParagraphEdit,
    on_standard: bool,
}

fn plan_edit(
    page_index: usize,
    paragraph: &ParagraphBlock,
    faces: &BTreeMap<String, ResolvedFont>,
    new_text: String,
) -> PdfResult<PlannedEdit> {
    let (resource, size) = span_source(paragraph)?;
    let required = required_chars(&new_text);
    let face = faces.get(&resource);
    if let Some(kept) = try_keep(
        &resource,
        size,
        paragraph.leading,
        paragraph.alignment,
        face,
        &required,
    )? {
        return Ok(PlannedEdit {
            edit: ParagraphEdit {
                paragraph: paragraph.clone(),
                new_text,
                metrics: kept.metrics,
                encoder: kept.encoder,
                prepared: kept.prepared,
            },
            on_standard: false,
        });
    }

    let Some(face) = face else {
        return Err(unsupported_character(page_index));
    };
    if required
        .iter()
        .any(|ch| WinAnsiEncoding::unicode_to_code(*ch).is_none())
    {
        return Err(unsupported_character(page_index));
    }
    let moved = standard_plan(
        &resource,
        size,
        paragraph.leading,
        paragraph.alignment,
        face,
    )?;
    Ok(PlannedEdit {
        edit: ParagraphEdit {
            paragraph: paragraph.clone(),
            new_text,
            metrics: moved.metrics,
            encoder: moved.encoder,
            prepared: moved.prepared,
        },
        on_standard: true,
    })
}

fn try_keep(
    resource: &str,
    size: f64,
    leading: f64,
    alignment: TextAlignment,
    face: Option<&ResolvedFont>,
    required: &[char],
) -> PdfResult<Option<KeptFace>> {
    let Some(face) = face else {
        if required
            .iter()
            .any(|ch| WinAnsiEncoding::unicode_to_code(*ch).is_none())
        {
            return Ok(None);
        }
        let prepared = ParagraphStyle::default().prepare(
            resource, size, leading, alignment, "", 400, "normal",
        )?;
        return Ok(Some(KeptFace {
            metrics: FontMetrics::new(0, 255, vec![500.0; 256], 500.0),
            encoder: FontEncoder::new(),
            prepared,
        }));
    };

    if face.is_composite {
        return Ok(None);
    }

    let mut codes = Vec::with_capacity(required.len());
    for ch in required {
        let code = match &face.cmap {
            Some(cmap) => {
                let Some(encoded) = cmap.encode_char(&ch.to_string()) else {
                    return Ok(None);
                };
                if encoded > 0xFF || cmap.decode_code(encoded) != Some(ch.to_string().as_str()) {
                    return Ok(None);
                }
                encoded
            }
            None => {
                let code = *ch as u32;
                if code > 0xFF {
                    return Ok(None);
                }
                code
            }
        };
        if !width_covers(&face.metrics, code) {
            return Ok(None);
        }
        codes.push((*ch, code));
    }

    let mut map = ToUnicodeMap::new();
    for (ch, code) in &codes {
        map.insert(*code, ch.to_string());
    }
    for (ch, _) in &codes {
        let Some(code) = map.encode_char(&ch.to_string()) else {
            return Ok(None);
        };
        if code > 0xFF || map.decode_code(code) != Some(ch.to_string().as_str()) {
            return Ok(None);
        }
    }
    let encoder = FontEncoder::new().with_tounicode(map);
    for (ch, _) in &codes {
        let (code, fallback) = encoder.encode_char(*ch);
        if fallback || code > 0xFF {
            return Ok(None);
        }
    }

    let prepared = ParagraphStyle::default().prepare(
        resource,
        size,
        leading,
        alignment,
        &face.family,
        face.weight,
        &face.style,
    )?;
    Ok(Some(KeptFace {
        metrics: face.metrics.clone(),
        encoder,
        prepared,
    }))
}

fn standard_plan(
    resource: &str,
    size: f64,
    leading: f64,
    alignment: TextAlignment,
    face: &ResolvedFont,
) -> PdfResult<KeptFace> {
    let style = ParagraphStyle {
        family: Some(standard_family(&face.family).to_string()),
        bold: Some(face.weight >= 700),
        italic: Some(face.style == "italic"),
        ..ParagraphStyle::default()
    };
    let prepared = style.prepare(
        resource,
        size,
        leading,
        alignment,
        &face.family,
        face.weight,
        &face.style,
    )?;
    Ok(KeptFace {
        metrics: FontMetrics::new(0, 255, vec![500.0; 256], 500.0),
        encoder: FontEncoder::new(),
        prepared,
    })
}

fn standard_family(css_family: &str) -> &'static str {
    let lower = css_family.to_ascii_lowercase();
    if lower.contains("times") {
        "Times"
    } else if lower.contains("courier") {
        "Courier"
    } else {
        "Helvetica"
    }
}

fn width_covers(metrics: &FontMetrics, code: u32) -> bool {
    if let Some(cids) = &metrics.cid_widths {
        return cids.contains_key(&code);
    }
    if metrics.widths.is_empty() {
        return true;
    }
    if code < metrics.first_char || code > metrics.last_char {
        return false;
    }
    let index = (code - metrics.first_char) as usize;
    index < metrics.widths.len()
}

fn required_chars(text: &str) -> Vec<char> {
    let mut chars: Vec<char> = text.chars().filter(|ch| *ch != '\n').collect();
    let needs_space = text
        .split('\n')
        .any(|line| line.split_whitespace().count() > 1);
    if needs_space && !chars.contains(&' ') {
        chars.push(' ');
    }
    chars
}

fn span_source(paragraph: &ParagraphBlock) -> PdfResult<(String, f64)> {
    let line = paragraph.lines.first().ok_or_else(|| {
        PdfError::LayoutError("Target paragraph contains no lines".to_string())
    })?;
    let span = line.spans.first().ok_or_else(|| {
        PdfError::LayoutError("Target paragraph contains no styled spans".to_string())
    })?;
    Ok((
        span.font_name.trim_start_matches('/').to_string(),
        span.font_size,
    ))
}

fn splice(text: &str, matches: &[(usize, usize)], replacement: &str) -> PdfResult<String> {
    let mut chars: Vec<char> = text.chars().collect();
    let repl: Vec<char> = replacement.chars().collect();
    for &(start, end) in matches.iter().rev() {
        if start > end || end > chars.len() {
            return Err(PdfError::LayoutError(
                "A match offset fell outside the paragraph.".to_string(),
            ));
        }
        let _ = chars.splice(start..end, repl.iter().copied());
    }
    Ok(chars.into_iter().collect())
}

fn apply(doc: &mut PdfDocument, plans: Vec<PagePlan>) -> PdfResult<()> {
    for mut plan in plans {
        plan.edits
            .sort_by(|left, right| right.paragraph.id.cmp(&left.paragraph.id));
        for edit in &plan.edits {
            SurgicalEditor::edit_paragraph_encoded(
                &mut plan.ast,
                &edit.paragraph,
                &edit.new_text,
                &edit.metrics,
                &edit.encoder,
                &edit.prepared,
            )?;
        }
        for edit in &plan.edits {
            if !edit.prepared.registers_font {
                continue;
            }
            let base = edit.prepared.base_font.as_deref().ok_or_else(|| {
                PdfError::OperationError("The page dictionary could not be updated.".to_string())
            })?;
            ensure_font_resource(doc, plan.page_id, &edit.prepared.font_resource, base)?;
        }
        replace_page_contents(doc, plan.page_id, serialize_ast(&plan.ast))?;
    }
    Ok(())
}

fn replace_page_contents(doc: &mut PdfDocument, page_id: ObjectId, bytes: Vec<u8>) -> PdfResult<()> {
    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(dictionary) => dictionary,
        _ => {
            return Err(PdfError::OperationError(
                "The page dictionary could not be updated.".to_string(),
            ));
        }
    };
    let contents_id = doc.alloc_object_id();
    let mut stream_dict = PdfDictionary::new();
    stream_dict.insert("Length", PdfObject::Integer(bytes.len() as i64));
    doc.set_object(
        contents_id,
        PdfObject::Stream(PdfStream::new(stream_dict, bytes)),
    );
    page_dict.insert("Contents", PdfObject::Reference(contents_id));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));
    Ok(())
}

fn load_page(
    doc: &mut PdfDocument,
    page_id: ObjectId,
) -> PdfResult<(ContentAst, Vec<ParagraphBlock>, BTreeMap<String, ResolvedFont>)> {
    let faces = resolve_page_fonts(doc, page_id)?;
    let hidden = resolve_page_hidden_ocg_props(doc, page_id)?;
    let bytes = doc.get_page_content_bytes(page_id)?;
    let ast = ContentParser::new(&bytes).parse()?;
    let paragraphs = LayoutReconstructor::new(&ast)
        .with_resolved(&faces)
        .with_hidden_ocg_props(hidden)
        .reconstruct()?;
    Ok((ast, paragraphs, faces))
}

fn marked(detail: impl std::fmt::Display) -> PdfError {
    PdfError::OperationError(format!("Find and replace: {detail}"))
}

fn unsupported_character(page_index: usize) -> PdfError {
    marked(format!(
        "A character on page {} is not in the standard fonts.",
        page_index + 1
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cos::PdfDocument;
    use crate::error::PdfError;

    const CONTRACT: &[u8] =
        b"BT\n/F1 14 Tf\n50 700 Tm\n(Contract Agreement Terms) Tj\n0 -16 Td\n(All clauses active.) Tj\nET\n";
    const SUBSET_TEXT: &[u8] = b"BT\n/F1 20 Tf\n50 700 Tm\n(\x01\x03\x01) Tj\nET\n";
    const TO_UNICODE: &[u8] = b"/CIDInit /ProcSet findresource begin\n3 beginbfchar\n<01> <0041>\n<02> <0042>\n<03> <0020>\nendbfchar\nend\n";

    fn message(error: PdfError) -> String {
        error.to_string()
    }

    fn pdf_from_objects(objects: &[Vec<u8>]) -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(body);
            if !body.ends_with(b"\n") {
                pdf.push(b'\n');
            }
            pdf.extend_from_slice(b"endobj\n");
        }
        let xref = pdf.len();
        let size = objects.len() + 1;
        pdf.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF").as_bytes(),
        );
        pdf
    }

    fn stream_body(data: &[u8]) -> Vec<u8> {
        let mut body = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"endstream");
        body
    }

    fn plain_pdf(stream: &[u8]) -> Vec<u8> {
        pdf_from_objects(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_vec(),
            stream_body(stream),
        ])
    }

    fn font_pdf(font: &str, stream: &[u8]) -> Vec<u8> {
        pdf_from_objects(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            stream_body(stream),
            font.as_bytes().to_vec(),
        ])
    }

    fn subset_pdf(base_font: &str, stream: &[u8]) -> Vec<u8> {
        let font = format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /{base_font} /FirstChar 1 /LastChar 3 /Widths [100 900 200] /ToUnicode 6 0 R >>"
        );
        pdf_from_objects(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            stream_body(stream),
            font.into_bytes(),
            stream_body(TO_UNICODE),
        ])
    }

    fn two_page_pdf(first: &[u8], second: &[u8]) -> Vec<u8> {
        pdf_from_objects(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R >>".to_vec(),
            stream_body(first),
            stream_body(second),
        ])
    }

    fn load(bytes: &[u8]) -> PdfDocument {
        PdfDocument::load(bytes).expect("pdf")
    }

    fn page_text(doc: &mut PdfDocument, page_index: usize) -> String {
        let pages = doc.get_pages().unwrap();
        let (_ast, paragraphs, _faces) = load_page(doc, pages[page_index]).unwrap();
        paragraphs
            .iter()
            .map(|paragraph| paragraph.text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn page_bytes(doc: &mut PdfDocument, page_index: usize) -> Vec<u8> {
        let pages = doc.get_pages().unwrap();
        doc.get_page_content_bytes(pages[page_index]).unwrap()
    }

    fn assert_unchanged(bytes: &[u8], query: &str, replacement: &str, ignore_case: bool, max_hits: usize) -> PdfError {
        let mut doc = load(bytes);
        let _ = doc.save_to_vec().unwrap();
        let before = doc.save_to_vec().unwrap();
        let error = replace_document_text(&mut doc, query, replacement, ignore_case, max_hits)
            .unwrap_err();
        let after = doc.save_to_vec().unwrap();
        assert_eq!(before, after);
        error
    }

    #[test]
    fn replace_keeps_a_fontless_paragraph_on_its_resource() {
        let mut doc = load(&plain_pdf(CONTRACT));
        let report = replace_document_text(&mut doc, "Agreement", "Accord", false, 100).unwrap();
        assert_eq!(report.replacements, 1);
        assert_eq!(report.paragraphs, 1);
        assert_eq!(report.paragraphs_on_standard_face, 0);
        assert!(!report.truncated);
        assert!(report.pages_skipped.is_empty());
        assert_eq!(
            page_text(&mut doc, 0),
            "Contract Accord Terms\nAll clauses active."
        );
        let bytes = page_bytes(&mut doc, 0);
        assert!(!bytes.windows(10).any(|window| window == b"F_ST_Helv"));
        assert!(String::from_utf8_lossy(&bytes).contains("Accord"));
        assert!(!String::from_utf8_lossy(&bytes).contains("Agreement"));
    }

    #[test]
    fn replace_splices_every_match_in_one_paragraph_once() {
        let stream = b"BT\n/F1 10 Tf\n50 700 Tm\n(red red) Tj\nET\n";
        let mut doc = load(&plain_pdf(stream));
        let report = replace_document_text(&mut doc, "red", "blue", false, 100).unwrap();
        assert_eq!(report.replacements, 2);
        assert_eq!(report.paragraphs, 1);
        assert!(!report.truncated);
        assert_eq!(page_text(&mut doc, 0), "blue blue");
    }

    #[test]
    fn empty_replacement_deletes_the_match() {
        let mut doc = load(&plain_pdf(CONTRACT));
        let report = replace_document_text(&mut doc, "Agreement", "", false, 100).unwrap();
        assert_eq!(report.replacements, 1);
        assert_eq!(
            page_text(&mut doc, 0),
            "Contract Terms\nAll clauses active."
        );
    }

    #[test]
    fn no_match_writes_nothing() {
        let bytes = plain_pdf(CONTRACT);
        let mut doc = load(&bytes);
        let _ = doc.save_to_vec().unwrap();
        let before = doc.save_to_vec().unwrap();
        let report = replace_document_text(&mut doc, "zzzz", "other", false, 100).unwrap();
        let after = doc.save_to_vec().unwrap();
        assert_eq!(before, after);
        assert_eq!(report.replacements, 0);
        assert_eq!(report.paragraphs, 0);
        assert_eq!(report.paragraphs_on_standard_face, 0);
        assert!(!report.truncated);
        assert!(report.pages_skipped.is_empty());
    }

    #[test]
    fn replace_rejects_bad_input_without_writing() {
        let bytes = plain_pdf(CONTRACT);
        let empty = assert_unchanged(&bytes, "", "Accord", false, 100);
        assert!(message(empty).contains("Find and replace: The search query is empty."));

        let long_query = "A".repeat(513);
        let query = assert_unchanged(&bytes, &long_query, "Accord", false, 100);
        assert!(message(query).contains("Find and replace: The search query is longer than 512 characters."));

        let long_replacement = "B".repeat(4001);
        let replacement = assert_unchanged(&bytes, "Agreement", &long_replacement, false, 100);
        assert!(message(replacement).contains("Find and replace: The replacement is longer than 4000 characters."));

        let tab = assert_unchanged(&bytes, "Agreement", "Acc\tord", false, 100);
        assert!(message(tab).contains("Find and replace: The replacement contains a control character."));

        let zero = assert_unchanged(&bytes, "Agreement", "Accord", false, 0);
        assert!(message(zero).contains("Find and replace: max_hits must be from 1 to 500."));
        let over = assert_unchanged(&bytes, "Agreement", "Accord", false, 501);
        assert!(message(over).contains("Find and replace: max_hits must be from 1 to 500."));
    }

    #[test]
    fn subset_keep_measures_real_widths_and_wraps() {
        let mut doc = load(&subset_pdf("ABCDEF+Calibri", SUBSET_TEXT));
        let report = replace_document_text(&mut doc, "A", "BB", false, 100).unwrap();
        assert_eq!(report.replacements, 2);
        assert_eq!(report.paragraphs, 1);
        assert_eq!(report.paragraphs_on_standard_face, 0);
        assert_eq!(page_text(&mut doc, 0), "BB\nBB");
        let bytes = page_bytes(&mut doc, 0);
        assert!(bytes.windows(4).any(|window| window == [b'(', 2, 2, b')']));
        assert!(!bytes.windows(4).any(|window| window == b"(BB)"));
        assert!(!bytes.windows(10).any(|window| window == b"F_ST_Helv"));
        assert!(bytes.windows(3).any(|window| window == b"/F1"));
    }

    #[test]
    fn subset_moves_a_missing_glyph_onto_a_standard_face() {
        let mut doc = load(&subset_pdf("ABCDEF+Calibri", SUBSET_TEXT));
        let report = replace_document_text(&mut doc, "A", "C", false, 100).unwrap();
        assert_eq!(report.paragraphs_on_standard_face, 1);
        assert_eq!(report.paragraphs, 1);
        assert_eq!(page_text(&mut doc, 0), "C C");
        let bytes = page_bytes(&mut doc, 0);
        assert!(bytes.windows(5).any(|window| window == b"(C C)"));
        assert!(bytes.windows(10).any(|window| window == b"/F_ST_Helv"));
        assert!(!bytes.contains(&1));
        let saved = String::from_utf8_lossy(&doc.save_to_vec().unwrap()).into_owned();
        assert!(saved.contains("/BaseFont /Helvetica"));
        assert!(!saved.contains("/Helvetica-Bold"));
    }

    #[test]
    fn subset_move_follows_bold_and_times() {
        let mut bold = load(&subset_pdf("ABCDEF+Calibri-Bold", SUBSET_TEXT));
        let bold_report = replace_document_text(&mut bold, "A", "C", false, 100).unwrap();
        assert_eq!(bold_report.paragraphs_on_standard_face, 1);
        let bold_page = page_bytes(&mut bold, 0);
        assert!(bold_page.windows(10).any(|window| window == b"/F_ST_HeBo"));
        let bold_saved = String::from_utf8_lossy(&bold.save_to_vec().unwrap()).into_owned();
        assert!(bold_saved.contains("/BaseFont /Helvetica-Bold"));

        let mut times = load(&subset_pdf("Times-Bold", SUBSET_TEXT));
        let times_report = replace_document_text(&mut times, "A", "C", false, 100).unwrap();
        assert_eq!(times_report.paragraphs_on_standard_face, 1);
        let times_page = page_bytes(&mut times, 0);
        assert!(times_page.windows(10).any(|window| window == b"/F_ST_TiBo"));
        let times_saved = String::from_utf8_lossy(&times.save_to_vec().unwrap()).into_owned();
        assert!(times_saved.contains("/BaseFont /Times-Bold"));
    }

    #[test]
    fn subset_character_outside_the_standard_faces_writes_nothing() {
        let bytes = subset_pdf("ABCDEF+Calibri", SUBSET_TEXT);
        let error = assert_unchanged(&bytes, "A", "\u{03A9}", false, 100);
        let text = message(error);
        assert!(text.contains("is not in the standard fonts"));
        assert!(text.contains("page 1"));
        let mut doc = load(&bytes);
        assert!(page_bytes(&mut doc, 0).contains(&1));
    }

    #[test]
    fn ignore_case_inserts_the_replacement_as_given() {
        let stream = b"BT\n/F1 12 Tf\n50 700 Tm\n(Hello) Tj\nET\n";
        let mut doc = load(&plain_pdf(stream));
        let report = replace_document_text(&mut doc, "hello", "Hi", true, 100).unwrap();
        assert_eq!(report.replacements, 1);
        assert_eq!(page_text(&mut doc, 0), "Hi");
    }

    #[test]
    fn hit_cap_stops_inside_the_paragraph() {
        let stream = b"BT\n/F1 10 Tf\n50 700 Tm\n(red red) Tj\nET\n";
        let bytes = plain_pdf(stream);
        let mut doc = load(&bytes);
        let capped = replace_document_text(&mut doc, "red", "blue", false, 1).unwrap();
        assert!(capped.truncated);
        assert_eq!(capped.replacements, 1);
        assert_eq!(capped.paragraphs, 1);
        assert_eq!(page_text(&mut doc, 0), "blue red");

        let mut both = load(&bytes);
        let exact = replace_document_text(&mut both, "red", "blue", false, 2).unwrap();
        assert!(exact.truncated);
        assert_eq!(exact.replacements, 2);
        assert_eq!(page_text(&mut both, 0), "blue blue");
    }

    #[test]
    fn unreadable_page_is_skipped_and_a_later_failure_writes_nothing() {
        let readable = b"BT\n/F1 12 Tf\n50 700 Tm\n(Hello there) Tj\nET\n";
        let mut doc = load(&two_page_pdf(readable, b"("));
        let report = replace_document_text(&mut doc, "Hello", "Hi", false, 100).unwrap();
        assert_eq!(report.pages_skipped, vec![1]);
        assert_eq!(report.replacements, 1);
        assert_eq!(page_text(&mut doc, 0), "Hi there");

        let first = b"BT\n/F1 14 Tf\n50 700 Tm\n(Hello) Tj\nET\n";
        let mut second = b"BT\n/F1 14 Tf\n50 700 Tm\n(Hello".to_vec();
        second.push(0x80);
        second.extend_from_slice(b") Tj\nET\n");
        let bytes = two_page_pdf(first, &second);
        let error = assert_unchanged(&bytes, "Hello", "Hi", false, 100);
        let text = message(error);
        assert!(text.contains("is not in the standard fonts"));
        assert!(text.contains("page 2"));
    }

    #[test]
    fn a_paragraph_past_16000_scalars_writes_nothing() {
        let mut stream = b"BT\n/F1 12 Tf\n50 700 Tm\n(".to_vec();
        stream.extend(std::iter::repeat(b'A').take(15000));
        stream.extend_from_slice(b") Tj\nET\n");
        let bytes = plain_pdf(&stream);
        let replacement = "B".repeat(4000);
        let error = assert_unchanged(&bytes, "A", &replacement, false, 1);
        assert!(message(error).contains("The paragraph would be longer than 16000 characters."));
    }

    #[test]
    fn resolved_font_without_widths_stays_on_its_resource() {
        let stream = b"BT\n/F1 12 Tf\n50 700 Tm\n(Agreement) Tj\nET\n";
        let font = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";
        let mut doc = load(&font_pdf(font, stream));
        let report = replace_document_text(&mut doc, "Agreement", "Accord", false, 100).unwrap();
        assert_eq!(report.paragraphs_on_standard_face, 0);
        assert_eq!(page_text(&mut doc, 0), "Accord");
        let bytes = page_bytes(&mut doc, 0);
        assert!(!bytes.windows(10).any(|window| window == b"F_ST_Helv"));
        assert!(bytes.windows(3).any(|window| window == b"/F1"));
    }

    #[test]
    fn resolved_byte_font_keeps_a_non_winansi_byte() {
        let mut stream = b"BT\n/F1 12 Tf\n50 700 Tm\n(A".to_vec();
        stream.push(0x80);
        stream.extend_from_slice(b") Tj\nET\n");
        let font = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";
        let mut doc = load(&font_pdf(&font, &stream));
        let report = replace_document_text(&mut doc, "A", "B", false, 100).unwrap();
        assert_eq!(report.paragraphs_on_standard_face, 0);
        let bytes = page_bytes(&mut doc, 0);
        assert!(bytes.contains(&0x80));
        assert!(bytes.contains(&b'B'));
        assert!(!bytes.windows(10).any(|window| window == b"F_ST_Helv"));
    }

    #[test]
    fn fontless_byte_outside_winansi_rejects_the_replace() {
        let mut stream = b"BT\n/F1 12 Tf\n50 700 Tm\n(A".to_vec();
        stream.push(0x80);
        stream.extend_from_slice(b") Tj\nET\n");
        let bytes = plain_pdf(&stream);
        let error = assert_unchanged(&bytes, "A", "B", false, 100);
        let text = message(error);
        assert!(text.contains("is not in the standard fonts"));
        assert!(text.contains("page 1"));
    }
}
