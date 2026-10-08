//! Semi-transparent text watermarks with matrix rotation and depth placement (ISO 32000-1 §8.7, §11).

use crate::cos::PdfDocument;
use crate::error::PdfResult;
use crate::watermark::helpers::{
    append_content_ops, ensure_extgstate_resource, ensure_font_resource, escape_pdf,
    get_page_mediabox, prepend_content_ops,
};
use crate::watermark::types::{TextWatermarkConfig, WatermarkPlacement};

/// Applies a semi-transparent rotated text watermark across target pages.
///
/// Returns the number of pages successfully stamped.
pub fn apply_text_watermark(
    doc: &mut PdfDocument,
    config: &TextWatermarkConfig,
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

    let mut applied_count = 0;

    for &idx in &target_indices {
        let page_id = pages[idx];

        // 1. Declare resources: bold font and opacity graphics state
        ensure_font_resource(doc, page_id, "F_WM", "Helvetica-Bold")?;
        ensure_extgstate_resource(doc, page_id, "GS_WM", config.opacity)?;

        // 2. Compute spatial center and text boundaries
        let bbox = get_page_mediabox(doc, page_id);
        let cx = bbox.min_x + bbox.width() / 2.0;
        let cy = bbox.min_y + bbox.height() / 2.0;

        let font_size = config.font_size.clamp(8.0, 250.0);
        let w_text = config.text.len() as f64 * font_size * 0.58;
        let h_text = font_size;

        // 3. Transformation matrix for rotation around page center
        let rad = config.rotation_degrees.to_radians();
        let cos_theta = rad.cos();
        let sin_theta = rad.sin();

        let r = config.color[0].clamp(0.0, 1.0);
        let g = config.color[1].clamp(0.0, 1.0);
        let b = config.color[2].clamp(0.0, 1.0);

        let ops = format!(
            "\nq\n/GS_WM gs\n{:.3} {:.3} {:.3} rg\nBT\n/F_WM {:.2} Tf\n{:.5} {:.5} {:.5} {:.5} {:.2} {:.2} Tm\n{:.2} {:.2} Td\n({}) Tj\nET\nQ\n",
            r, g, b,
            font_size,
            cos_theta, sin_theta, -sin_theta, cos_theta, cx, cy,
            -w_text / 2.0, -h_text * 0.35,
            escape_pdf(&config.text)
        );

        // 4. Inject based on placement layer
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
