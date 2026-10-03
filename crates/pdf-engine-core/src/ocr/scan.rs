//! Decides whether a page is a single-image scan.

use crate::cos::{ObjectId, PdfDocument, PdfObject};
use crate::error::PdfResult;
use crate::images::{extract_page_images, ImageInfo};
use crate::stream::ContentStreamTokenizer;

const TEXT_SHOWING: &[&str] = &["Tj", "TJ", "'", "\""];

/// Returns the one painted image when the page has no text-showing operators.
///
/// Any other shape of page, including a page that already has `Tj` or `TJ`,
/// returns `None` so a second pass does not stack another text layer.
pub(crate) fn painted_scan_image(
    doc: &mut PdfDocument,
    page_id: ObjectId,
) -> PdfResult<Option<ImageInfo>> {
    let content = doc.get_page_content_bytes(page_id).unwrap_or_default();
    let mut tokenizer = ContentStreamTokenizer::new(&content);
    let operations = match tokenizer.tokenize_all() {
        Ok(operations) => operations,
        Err(_) => return Ok(None),
    };

    let mut text_showing = false;
    let mut painted_name: Option<String> = None;
    for operation in &operations {
        if TEXT_SHOWING.contains(&operation.operator.as_str()) {
            text_showing = true;
            break;
        }
        if operation.operator == "Do" {
            let name = match operation.operands.first() {
                Some(PdfObject::Name(name)) => name.as_str().trim_start_matches('/').to_string(),
                _ => continue,
            };
            if name.is_empty() {
                continue;
            }
            if painted_name.is_some() {
                return Ok(None);
            }
            painted_name = Some(name);
        }
    }

    if text_showing {
        return Ok(None);
    }
    let Some(name) = painted_name else {
        return Ok(None);
    };

    let images = extract_page_images(doc, page_id)?;
    let mut matched = Vec::new();
    for image in images {
        if image.resource_name == name
            && image.width_px > 0
            && image.height_px > 0
            && image.bbox.width() > 0.0
            && image.bbox.height() > 0.0
        {
            matched.push(image);
        }
    }
    if matched.len() == 1 {
        Ok(Some(matched.remove(0)))
    } else {
        Ok(None)
    }
}
