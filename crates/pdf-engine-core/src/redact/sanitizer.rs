//! Document-level redaction executor, annotation pruner, and metadata sanitizer.

use crate::cos::object::{ObjectId, PdfDictionary, PdfObject, PdfStream, PdfString};
use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::fonts::FontMetrics;
use crate::layout::geometry::Rect;
use crate::redact::surgery::{apply_redaction_to_ast, find_pattern_boxes_on_page};
use crate::redact::types::{RedactionConfig, RedactionPattern, RedactionRect, RedactionSummary};
use crate::stream::parser::{build_ast_from_operations, serialize_ast, ContentStreamTokenizer};
use crate::watermark::helpers::ensure_font_resource;

/// Redacts specified rectangular regions on a single page of `doc`.
pub fn redact_page(
    doc: &mut PdfDocument,
    page_index: usize,
    redactions: &[RedactionRect],
    config: &RedactionConfig,
) -> PdfResult<RedactionSummary> {
    let pages = doc.get_pages()?;
    let page_id = pages
        .get(page_index)
        .copied()
        .ok_or_else(|| PdfError::InvalidPageNumber {
            page: page_index + 1,
            total: pages.len(),
        })?;

    if redactions.is_empty() {
        return Ok(RedactionSummary {
            page_index,
            ..Default::default()
        });
    }

    // 1. Ensure Standard Type 1 Helvetica is available in /Resources /Font if overlay labels are used
    let has_overlay_label = redactions.iter().any(|r| r.overlay_text.is_some());
    if has_overlay_label {
        let _ = ensure_font_resource(doc, page_id, "Helvetica", "Helvetica");
    }

    // 2. Load and tokenize page content stream
    let content_bytes = doc.get_page_content_bytes(page_id).unwrap_or_default();
    let mut tokenizer = ContentStreamTokenizer::new(&content_bytes);
    let ops = tokenizer.tokenize_all().unwrap_or_default();
    let mut ast = build_ast_from_operations(ops);

    // 3. Perform surgical redaction mutation on AST
    let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
    let mut summary = apply_redaction_to_ast(&mut ast, redactions, &metrics)?;
    summary.page_index = page_index;

    // 4. Serialize mutated AST back to content stream bytes
    let new_content_bytes = serialize_ast(&ast);

    // 5. Update page Contents stream
    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => {
            return Err(PdfError::OperationError(format!(
                "Page object {:?} is not a dictionary",
                page_id
            )))
        }
    };

    // 5. Update page Contents stream (overwriting old stream to prevent orphaned data leaks)
    let old_contents_refs: Vec<ObjectId> = match page_dict.get("Contents") {
        Some(PdfObject::Reference(r)) => vec![*r],
        Some(PdfObject::Array(arr)) => arr.iter().filter_map(|i| i.as_reference()).collect(),
        _ => Vec::new(),
    };

    let target_contents_id = if let Some(&first_ref) = old_contents_refs.first() {
        for &sec_ref in &old_contents_refs[1..] {
            doc.objects.remove(&sec_ref);
        }
        first_ref
    } else {
        doc.alloc_object_id()
    };

    let mut stream_dict = PdfDictionary::new();
    stream_dict.insert("Length", PdfObject::Integer(new_content_bytes.len() as i64));
    let stream = PdfStream::new(stream_dict, new_content_bytes);
    doc.set_object(target_contents_id, PdfObject::Stream(stream));

    page_dict.insert("Contents", PdfObject::Reference(target_contents_id));
    // Page-level metadata can repeat text that the content stream no longer shows.
    page_dict.remove("Metadata");
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    // 6. Prune intersecting interactive annotations (links, highlights, redact annotations)
    if config.prune_annotations {
        let pruned = prune_page_annotations(doc, page_id, redactions)?;
        summary.pruned_annotations_count = pruned;
    }

    // 7. Scrub document metadata if requested
    if config.scrub_metadata {
        let _ = scrub_document_metadata(doc)?;
    }

    Ok(summary)
}

/// Scans the document for a sensitive data `pattern` and redacts all occurrences.
pub fn redact_document_pattern(
    doc: &mut PdfDocument,
    pattern: &RedactionPattern,
    target_pages: Option<&[usize]>,
    config: &RedactionConfig,
) -> PdfResult<Vec<RedactionSummary>> {
    let pages = doc.get_pages()?;
    let total_pages = pages.len();

    let page_indices: Vec<usize> = match target_pages {
        Some(indices) => indices
            .iter()
            .copied()
            .filter(|&idx| idx < total_pages)
            .collect(),
        None => (0..total_pages).collect(),
    };

    let mut summaries = Vec::new();
    let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);

    for idx in page_indices {
        let page_id = pages[idx];
        let content_bytes = doc.get_page_content_bytes(page_id).unwrap_or_default();
        let mut tokenizer = ContentStreamTokenizer::new(&content_bytes);
        let ops = tokenizer.tokenize_all().unwrap_or_default();
        let ast = build_ast_from_operations(ops);

        // Find matches on this page
        let detected_boxes = find_pattern_boxes_on_page(&ast, pattern, &metrics, config.padding)?;
        if detected_boxes.is_empty() {
            continue;
        }

        // Construct RedactionRect items
        let redactions: Vec<RedactionRect> = detected_boxes
            .into_iter()
            .map(|rect| {
                let mut r = RedactionRect::new(rect).with_fill_color(
                    config.fill_color[0],
                    config.fill_color[1],
                    config.fill_color[2],
                );
                if let Some(ref text) = config.overlay_text {
                    r = r.with_overlay_text(text, Some(config.text_color));
                }
                if let Some(sz) = config.font_size {
                    r = r.with_font_size(sz);
                }
                r
            })
            .collect();

        let summary = redact_page(doc, idx, &redactions, config)?;
        summaries.push(summary);
    }

    if config.scrub_metadata {
        let _ = scrub_document_metadata(doc)?;
    }

    Ok(summaries)
}

/// Redacts explicit bounding box rectangles on a single page.
pub fn redact_document_rectangles(
    doc: &mut PdfDocument,
    page_index: usize,
    rects: &[Rect],
    config: &RedactionConfig,
) -> PdfResult<RedactionSummary> {
    let redactions: Vec<RedactionRect> = rects
        .iter()
        .map(|&rect| {
            let mut r = RedactionRect::new(rect).with_fill_color(
                config.fill_color[0],
                config.fill_color[1],
                config.fill_color[2],
            );
            if let Some(ref text) = config.overlay_text {
                r = r.with_overlay_text(text, Some(config.text_color));
            }
            if let Some(sz) = config.font_size {
                r = r.with_font_size(sz);
            }
            r
        })
        .collect();

    redact_page(doc, page_index, &redactions, config)
}

/// Prunes `/Redact` annotations and any annotations intersecting `redactions` from a page.
pub fn prune_page_annotations(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    redactions: &[RedactionRect],
) -> PdfResult<usize> {
    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => return Ok(0),
    };

    let annot_refs: Vec<ObjectId> = match page_dict.get("Annots") {
        Some(PdfObject::Array(arr)) => arr.iter().filter_map(|item| item.as_reference()).collect(),
        Some(PdfObject::Reference(ref_id)) => {
            if let Ok(PdfObject::Array(arr)) = doc.get_object(*ref_id) {
                arr.iter().filter_map(|item| item.as_reference()).collect()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    };

    if annot_refs.is_empty() {
        return Ok(0);
    }

    let mut to_remove = Vec::new();

    for annot_id in annot_refs {
        if let Ok(PdfObject::Dictionary(annot_dict)) = doc.get_object(annot_id) {
            let subtype = annot_dict
                .get("Subtype")
                .and_then(|s| s.as_name())
                .unwrap_or("");

            // Always remove /Redact annotations
            if subtype == "Redact" {
                to_remove.push(annot_id);
                continue;
            }

            // Extract annotation bounding box
            if let Some(PdfObject::Array(rect_arr)) = annot_dict.get("Rect") {
                if rect_arr.len() == 4 {
                    let min_x = rect_arr[0].as_f64().unwrap_or(0.0);
                    let min_y = rect_arr[1].as_f64().unwrap_or(0.0);
                    let max_x = rect_arr[2].as_f64().unwrap_or(0.0);
                    let max_y = rect_arr[3].as_f64().unwrap_or(0.0);
                    let annot_rect = Rect::new(min_x, min_y, max_x, max_y);

                    if redactions.iter().any(|r| r.rect.intersects(&annot_rect)) {
                        to_remove.push(annot_id);
                    }
                }
            }
        }
    }

    let pruned_count = to_remove.len();
    if pruned_count == 0 {
        return Ok(0);
    }

    // Update /Annots array
    if let Some(PdfObject::Array(ref mut arr)) = page_dict.get_mut("Annots") {
        arr.retain(|item| match item {
            PdfObject::Reference(id) => !to_remove.contains(id),
            _ => true,
        });
    } else if let Some(ref_id) = page_dict.get("Annots").and_then(|a| a.as_reference()) {
        if let Ok(PdfObject::Array(mut arr)) = doc.get_object(ref_id) {
            arr.retain(|item| match item {
                PdfObject::Reference(id) => !to_remove.contains(id),
                _ => true,
            });
            doc.set_object(ref_id, PdfObject::Array(arr));
        }
    }

    // Delete annotation objects from doc
    for id in to_remove {
        doc.objects.remove(&id);
    }

    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    Ok(pruned_count)
}

/// Cleans sensitive document metadata from `/Info` and removes `/Root /Metadata` (XMP).
pub fn scrub_document_metadata(doc: &mut PdfDocument) -> PdfResult<bool> {
    let mut modified = false;

    // 1. Scrub /Info dictionary
    if let Some(info_ref) = doc.xref.trailer.get("Info").and_then(|o| o.as_reference()) {
        if let Ok(PdfObject::Dictionary(mut info_dict)) = doc.get_object(info_ref) {
            info_dict.remove("Title");
            info_dict.remove("Author");
            info_dict.remove("Subject");
            info_dict.remove("Keywords");
            info_dict.remove("Creator");
            info_dict.insert(
                "Producer",
                PdfObject::String(PdfString::literal(b"PDFEngine Sanitizer")),
            );
            doc.set_object(info_ref, PdfObject::Dictionary(info_dict));
            modified = true;
        }
    }

    // 2. Scrub /Metadata (XMP) stream from document catalog
    if let Some(catalog_id) = doc.catalog_id() {
        if let Ok(PdfObject::Dictionary(mut cat_dict)) = doc.get_object(catalog_id) {
            if let Some(meta_ref) = cat_dict.remove("Metadata").and_then(|m| m.as_reference()) {
                doc.objects.remove(&meta_ref);
                doc.set_object(catalog_id, PdfObject::Dictionary(cat_dict));
                modified = true;
            }
        }
    }

    Ok(modified)
}
