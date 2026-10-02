//! Dynamic page numbering, Bates numbering, headers, and footers (ISO 32000-1 §8.4).

use crate::cos::PdfDocument;
use crate::error::PdfResult;
use crate::watermark::helpers::{append_content_ops, ensure_font_resource, escape_pdf, get_page_mediabox};
use crate::watermark::types::{PaginationConfig, PaginationPosition};

/// Injects dynamic headers, footers, or page numbers across target document pages.
///
/// Returns the number of pages successfully paginated.
pub fn apply_pagination(doc: &mut PdfDocument, config: &PaginationConfig) -> PdfResult<usize> {
    let pages = doc.get_pages()?;
    if pages.is_empty() {
        return Ok(0);
    }

    let total_pages = pages.len();

    // Determine target page indices
    let target_indices: Vec<usize> = match &config.page_indices {
        Some(indices) => indices
            .iter()
            .copied()
            .filter(|&idx| idx < total_pages)
            .filter(|&idx| !config.skip_first_page || idx > 0)
            .collect(),
        None => (0..total_pages)
            .filter(|&idx| !config.skip_first_page || idx > 0)
            .collect(),
    };

    if target_indices.is_empty() {
        return Ok(0);
    }

    let mut applied_count = 0;

    for &idx in &target_indices {
        let page_id = pages[idx];

        // 1. Calculate dynamic page number and format template
        let page_number = if config.skip_first_page {
            config.start_page_num + idx.saturating_sub(1)
        } else {
            config.start_page_num + idx
        };

        let formatted = config
            .format
            .replace("{page}", &page_number.to_string())
            .replace("{total}", &total_pages.to_string());

        // 2. Ensure font resource is present on the page
        ensure_font_resource(doc, page_id, "F_PAG", "Helvetica")?;

        // 3. Compute spatial coordinates based on page MediaBox
        let bbox = get_page_mediabox(doc, page_id);
        let margin = config.margin.max(8.0);
        let font_size = config.font_size.clamp(5.0, 72.0);

        // Approximate character width for standard Helvetica (~0.52em)
        let est_char_width = font_size * 0.52;
        let est_text_width = formatted.len() as f64 * est_char_width;

        let x = match config.position {
            PaginationPosition::TopLeft | PaginationPosition::BottomLeft => {
                bbox.min_x + margin
            }
            PaginationPosition::TopCenter | PaginationPosition::BottomCenter => {
                bbox.min_x + ((bbox.width() - est_text_width) / 2.0).max(margin)
            }
            PaginationPosition::TopRight | PaginationPosition::BottomRight => {
                (bbox.max_x - margin - est_text_width).max(bbox.min_x + margin)
            }
        };

        let y = match config.position {
            PaginationPosition::TopLeft | PaginationPosition::TopCenter | PaginationPosition::TopRight => {
                bbox.max_y - margin - font_size
            }
            PaginationPosition::BottomLeft | PaginationPosition::BottomCenter | PaginationPosition::BottomRight => {
                bbox.min_y + margin
            }
        };

        // 4. Synthesize graphics content operations
        let r = config.color[0].clamp(0.0, 1.0);
        let g = config.color[1].clamp(0.0, 1.0);
        let b = config.color[2].clamp(0.0, 1.0);

        let ops = format!(
            "\nq\n{:.3} {:.3} {:.3} rg\nBT\n/F_PAG {:.2} Tf\n{:.2} {:.2} Td\n({}) Tj\nET\nQ\n",
            r, g, b,
            font_size,
            x, y,
            escape_pdf(&formatted)
        );

        // 5. Append to page content stream (foreground)
        append_content_ops(doc, page_id, &ops)?;
        applied_count += 1;
    }

    Ok(applied_count)
}
