//! Deletion of annotations and associated appearance resources according to ISO 32000-1 §12.5.

use crate::cos::object::{ObjectId, PdfObject};
use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};

/// Deletes an annotation from a page's `/Annots` array and purges its objects from the document.
///
/// Returns `true` if the annotation was found and deleted, `false` otherwise.
pub fn delete_annotation(
    doc: &mut PdfDocument,
    page_index: usize,
    annot_id: ObjectId,
) -> PdfResult<bool> {
    let pages = doc.get_pages()?;
    let page_id = pages
        .get(page_index)
        .copied()
        .ok_or_else(|| PdfError::InvalidPageNumber {
            page: page_index + 1,
            total: pages.len(),
        })?;

    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => return Ok(false),
    };

    let mut removed = false;

    // Check direct /Annots array
    if let Some(PdfObject::Array(ref mut arr)) = page_dict.get_mut("Annots") {
        let initial_len = arr.len();
        arr.retain(|item| match item {
            PdfObject::Reference(id) => *id != annot_id,
            _ => true,
        });
        if arr.len() < initial_len {
            removed = true;
        }
    } else if let Some(ref_id) = page_dict.get("Annots").and_then(|a| a.as_reference()) {
        // Indirect /Annots array
        if let Ok(PdfObject::Array(mut arr)) = doc.get_object(ref_id) {
            let initial_len = arr.len();
            arr.retain(|item| match item {
                PdfObject::Reference(id) => *id != annot_id,
                _ => true,
            });
            if arr.len() < initial_len {
                removed = true;
                doc.set_object(ref_id, PdfObject::Array(arr));
            }
        }
    }

    if removed {
        // Collect appearance stream object IDs to clean up
        let ap_stream_ids = if let Ok(PdfObject::Dictionary(annot_dict)) = doc.get_object(annot_id)
        {
            collect_appearance_stream_ids(&annot_dict)
        } else {
            Vec::new()
        };

        // Purge appearance streams
        for ap_id in ap_stream_ids {
            doc.objects.remove(&ap_id);
        }

        // Purge annotation object itself
        doc.objects.remove(&annot_id);

        // Update page dictionary
        doc.set_object(page_id, PdfObject::Dictionary(page_dict));
    }

    Ok(removed)
}

/// Helper resolving appearance stream ObjectIds from an annotation dictionary.
fn collect_appearance_stream_ids(dict: &crate::cos::object::PdfDictionary) -> Vec<ObjectId> {
    let mut ids = Vec::new();
    if let Some(PdfObject::Dictionary(ap_dict)) = dict.get("AP") {
        for v in ap_dict.0.values() {
            match v {
                PdfObject::Reference(id) => ids.push(*id),
                PdfObject::Dictionary(sub_dict) => {
                    for sub_v in sub_dict.0.values() {
                        if let PdfObject::Reference(sub_id) = sub_v {
                            ids.push(*sub_id);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    ids
}
