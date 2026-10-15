//! Removes an embedded file when it repeats text a redaction already removed.
//!
//! The whole file is deleted. A string shorter than 4 Unicode scalars does not
//! select a file. The logical name, the filename, the description, and the
//! decoded bytes are checked. When those bytes begin with `%PDF-` and do not
//! already contain the string, the file is opened once and its page text is
//! searched. A file inside that PDF is not opened. Zip and Office packages are
//! not unpacked. A file that does not open stays unless the bytes already
//! contain the string.

use crate::cos::PdfDocument;
use crate::error::PdfResult;
use crate::fonts::{resolve_page_fonts, FontMetrics};
use crate::layout::reconstructor::LayoutReconstructor;
use crate::ops::attachments::{collect_all_attachments, delete_attachment, EmbeddedAttachment};
use crate::stream::parser::{build_ast_from_operations, ContentStreamTokenizer};

pub(super) const MIN_NEEDLE_SCALARS: usize = 4;

/// Deletes embedded files that repeat `exact` or, when set, `ignore_case`.
///
/// `ignore_case` is the original text query. It is used only for a
/// case-insensitive text redaction. PII needles stay in `exact` and are
/// matched as raw bytes. Returns how many attachments repeated a needle.
pub(super) fn remove_embedded_files_repeating(
    doc: &mut PdfDocument,
    exact: &[String],
    ignore_case: Option<&str>,
) -> PdfResult<usize> {
    let exact: Vec<&str> = exact
        .iter()
        .map(String::as_str)
        .filter(|needle| needle.chars().count() >= MIN_NEEDLE_SCALARS)
        .collect();
    let ignore_case = ignore_case.filter(|query| query.chars().count() >= MIN_NEEDLE_SCALARS);
    if exact.is_empty() && ignore_case.is_none() {
        return Ok(0);
    }

    let attachments = collect_all_attachments(doc)?;
    let mut removed = 0usize;
    let mut keys = Vec::new();
    for attachment in &attachments {
        if !attachment_repeats(attachment, &exact, ignore_case) {
            continue;
        }
        removed += 1;
        let key = if attachment.info.filename.is_empty() {
            attachment.info.name.clone()
        } else {
            attachment.info.filename.clone()
        };
        if key.is_empty() || keys.iter().any(|existing: &String| existing == &key) {
            continue;
        }
        keys.push(key);
    }
    for key in &keys {
        delete_attachment(doc, key)?;
    }
    Ok(removed)
}

fn attachment_repeats(
    attachment: &EmbeddedAttachment,
    exact: &[&str],
    ignore_case: Option<&str>,
) -> bool {
    let info = &attachment.info;
    if text_repeats(&info.name, exact, ignore_case)
        || text_repeats(&info.filename, exact, ignore_case)
        || info
            .description
            .as_deref()
            .is_some_and(|description| text_repeats(description, exact, ignore_case))
    {
        return true;
    }
    if payload_repeats(&attachment.data, exact, ignore_case) {
        return true;
    }
    if attachment.data.starts_with(b"%PDF-") {
        if let Some(text) = nested_pdf_page_text(&attachment.data) {
            return text_repeats(&text, exact, ignore_case);
        }
    }
    false
}

pub(super) fn text_repeats(haystack: &str, exact: &[&str], ignore_case: Option<&str>) -> bool {
    if haystack.is_empty() {
        return false;
    }
    if exact.iter().any(|needle| haystack.contains(needle)) {
        return true;
    }
    match ignore_case {
        Some(query) => haystack.to_lowercase().contains(&query.to_lowercase()),
        None => false,
    }
}

pub(super) fn payload_repeats(data: &[u8], exact: &[&str], ignore_case: Option<&str>) -> bool {
    if exact.iter().any(|needle| bytes_contain(data, needle.as_bytes())) {
        return true;
    }
    match ignore_case {
        Some(query) => String::from_utf8_lossy(data)
            .to_lowercase()
            .contains(&query.to_lowercase()),
        None => false,
    }
}

pub(super) fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack.windows(needle.len()).any(|window| window == needle)
}

/// Page text of one embedded PDF. Its own embedded files are not opened.
fn nested_pdf_page_text(data: &[u8]) -> Option<String> {
    let mut nested = PdfDocument::load(data).ok()?;
    let pages = nested.get_pages().ok()?;
    let mut text = String::new();
    for page_id in pages {
        let bytes = match nested.get_page_content_bytes(page_id) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let mut tokenizer = ContentStreamTokenizer::new(&bytes);
        let Ok(ops) = tokenizer.tokenize_all() else {
            continue;
        };
        let ast = build_ast_from_operations(ops);
        let faces = match resolve_page_fonts(&mut nested, page_id) {
            Ok(faces) => faces,
            Err(_) => continue,
        };
        let paragraphs = if faces.is_empty() {
            let fallback = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
            LayoutReconstructor::new(&ast)
                .with_font("F1", fallback)
                .reconstruct()
        } else {
            LayoutReconstructor::new(&ast)
                .with_resolved(&faces)
                .reconstruct()
        };
        let Ok(paragraphs) = paragraphs else {
            continue;
        };
        for paragraph in paragraphs {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&paragraph.text());
        }
    }
    Some(text)
}
