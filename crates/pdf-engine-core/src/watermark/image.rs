//! Semi-transparent image watermarks with transformation matrices and opacity (ISO 32000-1 §8.7, §8.9).

use crate::cos::PdfDocument;
use crate::error::PdfResult;
use crate::images::create_image_xobject;
use crate::watermark::helpers::{
    append_content_ops, ensure_extgstate_resource, ensure_xobject_resource, get_page_mediabox,
    prepend_content_ops,
};
use crate::watermark::types::{ImageWatermarkConfig, WatermarkPlacement};

/// Embeds an image and applies it as a semi-transparent rotated watermark across target pages.
///
/// Returns the number of pages successfully stamped.
pub fn apply_image_watermark(
    doc: &mut PdfDocument,
    config: &ImageWatermarkConfig,
) -> PdfResult<usize> {
    let pages = doc.get_pages()?;
    if pages.is_empty() {
        return Ok(0);
    }

    let total_pages = pages.len();

    let target_indices: Vec<usize> = match &config.page_indices {
        Some(indices) => indices
            .iter()
            .copied()
            .filter(|&idx| idx < total_pages)
            .collect(),
        None => (0..total_pages).collect(),
    };

    if target_indices.is_empty() {
        return Ok(0);
    }

    // 1. Embed image binary once as an Image XObject in the document
    let (img_id, orig_w, orig_h) = create_image_xobject(doc, &config.image_bytes)?;

    let mut applied_count = 0;

    for &idx in &target_indices {
        let page_id = pages[idx];

        // 2. Register Image XObject and ExtGState in page resources
        ensure_xobject_resource(doc, page_id, "WM_IMG", img_id)?;
        ensure_extgstate_resource(doc, page_id, "GS_WM", config.opacity)?;

        // 3. Calculate dimensions and positioning
        let bbox = get_page_mediabox(doc, page_id);
        let cx = bbox.min_x + bbox.width() / 2.0;
        let cy = bbox.min_y + bbox.height() / 2.0;

        let aspect_ratio = if orig_w > 0 {
            orig_h as f64 / orig_w as f64
        } else {
            1.0
        };

        let target_w = config
            .width
            .unwrap_or_else(|| bbox.width() * 0.50)
            .max(10.0);
        let target_h = config
            .height
            .unwrap_or_else(|| target_w * aspect_ratio)
            .max(10.0);

        // 4. Matrix transform: translate to center, rotate, offset origin, and scale
        let rad = config.rotation_degrees.to_radians();
        let cos_t = rad.cos();
        let sin_t = rad.sin();

        let a = target_w * cos_t;
        let b = target_w * sin_t;
        let c = -target_h * sin_t;
        let d = target_h * cos_t;
        let e = cx - cos_t * (target_w / 2.0) + sin_t * (target_h / 2.0);
        let f = cy - sin_t * (target_w / 2.0) - cos_t * (target_h / 2.0);

        let ops = format!(
            "\nq\n/GS_WM gs\n{:.5} {:.5} {:.5} {:.5} {:.2} {:.2} cm\n/WM_IMG Do\nQ\n",
            a, b, c, d, e, f
        );

        // 5. Inject based on placement layer
        match config.placement {
            WatermarkPlacement::Background => {
                prepend_content_ops(doc, page_id, &ops)?;
            }
            WatermarkPlacement::Foreground => {
                append_content_ops(doc, page_id, &ops)?;
            }
        }

        applied_count += 1;
    }

    Ok(applied_count)
}
