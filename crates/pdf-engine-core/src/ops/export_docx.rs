//! Native structured document export to Microsoft Word OpenXML (`.docx`) and Markdown (`.md`).
//!
//! Reconstructs visual layout hierarchies (`TextSpan` -> `TextLine` -> `ParagraphBlock`),
//! typographic styles (font family, point size, bold/italic weights, alignment), and
//! lattice/borderless tables (`DetectedTable`) into editable WordprocessingML packages
//! and GitHub-Flavored Markdown documents.

use std::collections::BTreeMap;
use std::io::Write;

use flate2::write::DeflateEncoder;
use flate2::Compression;

use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::fonts::{resolve_page_fonts, ResolvedFont};
use crate::layout::{LayoutReconstructor, ParagraphBlock, Rect, TextAlignment};
use crate::ops::layers::resolve_page_hidden_ocg_props;
use crate::ops::metadata::extract_metadata;
use crate::stream::parser::{build_ast_from_operations, ContentStreamTokenizer};
use crate::tables::{
    detect_borderless_tables, detect_lattice_tables, export_to_markdown, DetectedTable,
};

/// A unified structural block on a single PDF page, ordered top-to-bottom.
#[derive(Debug, Clone)]
enum PageElement {
    Paragraph(ParagraphBlock),
    Table(DetectedTable),
}

impl PageElement {
    fn bbox(&self) -> Rect {
        match self {
            PageElement::Paragraph(p) => p.bbox,
            PageElement::Table(t) => t.bbox,
        }
    }
}

/// Checks whether a paragraph block lies inside a detected table's bounding box.
fn is_paragraph_inside_table(para: &ParagraphBlock, table: &DetectedTable) -> bool {
    let cx = (para.bbox.min_x + para.bbox.max_x) * 0.5;
    let cy = (para.bbox.min_y + para.bbox.max_y) * 0.5;
    let margin = 2.0;
    cx >= table.bbox.min_x - margin
        && cx <= table.bbox.max_x + margin
        && cy >= table.bbox.min_y - margin
        && cy <= table.bbox.max_y + margin
}

/// Extracts ordered structural elements (paragraphs and tables) and resolved fonts for a page.
fn extract_page_elements(
    doc: &mut PdfDocument,
    page_idx: usize,
) -> PdfResult<(Vec<PageElement>, BTreeMap<String, ResolvedFont>)> {
    let pages = doc.get_pages()?;
    if page_idx == 0 || page_idx > pages.len() {
        return Err(PdfError::InvalidPageNumber {
            page: page_idx,
            total: pages.len(),
        });
    }

    let page_id = pages[page_idx - 1];
    let fonts = resolve_page_fonts(doc, page_id)?;
    let hidden_ocg_props = resolve_page_hidden_ocg_props(doc, page_id)?;
    let content_bytes = doc.get_page_content_bytes(page_id)?;

    if content_bytes.is_empty() {
        return Ok((Vec::new(), fonts));
    }

    let mut tokenizer = ContentStreamTokenizer::new(&content_bytes);
    let operations = tokenizer.tokenize_all().unwrap_or_default();
    let ast = build_ast_from_operations(operations);

    let reconstructor = LayoutReconstructor::new(&ast)
        .with_resolved(&fonts)
        .with_hidden_ocg_props(hidden_ocg_props);
    let paragraphs = reconstructor.reconstruct()?;

    let mut tables = detect_lattice_tables(&ast, &paragraphs, page_idx)?;
    if tables.is_empty() {
        tables = detect_borderless_tables(&paragraphs, page_idx);
    }

    let mut elements = Vec::new();
    for para in paragraphs {
        if !tables.iter().any(|t| is_paragraph_inside_table(&para, t)) {
            elements.push(PageElement::Paragraph(para));
        }
    }
    for table in tables {
        elements.push(PageElement::Table(table));
    }

    // Sort top-to-bottom (higher max_y first in PDF coordinates), then left-to-right (lower min_x first)
    elements.sort_by(|a, b| {
        let ba = a.bbox();
        let bb = b.bbox();
        bb.max_y
            .partial_cmp(&ba.max_y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                ba.min_x
                    .partial_cmp(&bb.min_x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    Ok((elements, fonts))
}

/// Determines whether a font resource is bold and/or italic.
fn resolve_font_style(
    font_name: &str,
    fonts: &BTreeMap<String, ResolvedFont>,
) -> (String, bool, bool) {
    let clean_name = font_name.trim_start_matches('/');
    if let Some(resolved) = fonts.get(clean_name) {
        let family = match resolved.family.as_str() {
            "serif" => "Times New Roman".to_string(),
            "monospace" => "Courier New".to_string(),
            "sans-serif" => "Arial".to_string(),
            other if !other.is_empty() => other.to_string(),
            _ => "Calibri".to_string(),
        };
        let is_bold = resolved.weight >= 700 || is_name_bold(clean_name);
        let is_italic = resolved.style == "italic" || is_name_italic(clean_name);
        return (family, is_bold, is_italic);
    }

    let lower = clean_name.to_ascii_lowercase();
    let family = if lower.contains("times") || lower.contains("serif") {
        "Times New Roman".to_string()
    } else if lower.contains("courier") || lower.contains("mono") {
        "Courier New".to_string()
    } else if lower.contains("helvetica") || lower.contains("arial") {
        "Arial".to_string()
    } else {
        "Calibri".to_string()
    };

    (family, is_name_bold(clean_name), is_name_italic(clean_name))
}

fn is_name_bold(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("bold") || lower.ends_with("bd") || lower.contains("-bd")
}

fn is_name_italic(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("italic") || lower.contains("oblique") || lower.ends_with("it")
}

/// Exports the entire PDF document into structured GitHub-Flavored Markdown (`.md`).
///
/// Preserves heading levels inferred from rendered font sizes, bold/italic span runs,
/// paragraph breaks, and reconstructed tables.
pub fn export_document_markdown(doc: &mut PdfDocument) -> PdfResult<String> {
    doc.require_copy()?;
    let pages = doc.get_pages()?;
    let mut out = String::new();

    for page_idx in 1..=pages.len() {
        if page_idx > 1 && !out.is_empty() {
            if !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str("---\n\n");
        }

        let (elements, fonts) = extract_page_elements(doc, page_idx)?;
        for element in elements {
            match element {
                PageElement::Paragraph(para) => {
                    let size = para.rendered_size();
                    let raw_text = para.text();
                    let trimmed = raw_text.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    if size >= 20.0 {
                        let single_line = trimmed.lines().map(str::trim).collect::<Vec<_>>().join(" ");
                        out.push_str("# ");
                        out.push_str(&single_line);
                        out.push_str("\n\n");
                    } else if size >= 16.0 {
                        let single_line = trimmed.lines().map(str::trim).collect::<Vec<_>>().join(" ");
                        out.push_str("## ");
                        out.push_str(&single_line);
                        out.push_str("\n\n");
                    } else if size >= 13.5 {
                        let single_line = trimmed.lines().map(str::trim).collect::<Vec<_>>().join(" ");
                        out.push_str("### ");
                        out.push_str(&single_line);
                        out.push_str("\n\n");
                    } else {
                        let mut para_md = String::new();
                        for (line_idx, line) in para.lines.iter().enumerate() {
                            if line_idx > 0 {
                                para_md.push(' ');
                            }
                            if line.spans.is_empty() {
                                para_md.push_str(line.text.trim());
                            } else {
                                for (span_idx, span) in line.spans.iter().enumerate() {
                                    if span_idx > 0
                                        && !para_md.ends_with(' ')
                                        && !span.text.starts_with(' ')
                                    {
                                        para_md.push(' ');
                                    }
                                    let (_, is_bold, is_italic) =
                                        resolve_font_style(&span.font_name, &fonts);
                                    let span_trimmed = span.text.trim();
                                    if span_trimmed.is_empty() {
                                        continue;
                                    }
                                    if is_bold && is_italic {
                                        para_md.push_str("***");
                                        para_md.push_str(span_trimmed);
                                        para_md.push_str("***");
                                    } else if is_bold {
                                        para_md.push_str("**");
                                        para_md.push_str(span_trimmed);
                                        para_md.push_str("**");
                                    } else if is_italic {
                                        para_md.push('*');
                                        para_md.push_str(span_trimmed);
                                        para_md.push('*');
                                    } else {
                                        para_md.push_str(&span.text);
                                    }
                                }
                            }
                        }
                        let final_para = para_md.trim();
                        if !final_para.is_empty() {
                            out.push_str(final_para);
                            out.push_str("\n\n");
                        }
                    }
                }
                PageElement::Table(table) => {
                    let md_table = export_to_markdown(&table);
                    if !md_table.trim().is_empty() {
                        out.push_str(md_table.trim_end());
                        out.push_str("\n\n");
                    }
                }
            }
        }
    }

    Ok(out.trim_end().to_string())
}

/// Escapes special XML characters (`&`, `<`, `>`, `"`, `'`) and strips invalid XML 1.0 control chars.
fn escape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(ch),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

/// One PDF point, in twentieths of a point (Word twips).
const PT_TO_TWIPS: f64 = 20.0;
/// Smallest margin, in points. A tighter content edge is raised to this.
const MARGIN_FLOOR_PT: f64 = 36.0;
/// Largest margin, in points. Empty space beyond this stays in the body.
const MARGIN_CAP_PT: f64 = 72.0;
/// Vertical gaps at or below this are leading, not paragraph space-before.
const SPACING_IGNORE_PT: f64 = 16.0;
/// Space-before cap: 72 points.
const SPACING_CAP_TWIPS: i64 = 1440;
/// Word page size clamp: 7.2 pt through 14400 pt.
const TWIP_MIN: i64 = 144;
const TWIP_MAX: i64 = 288_000;
/// Indent below this is treated as flush with the margin.
const INDENT_EPSILON_PT: f64 = 0.5;

/// Page size and margins derived from the displayed box and the content edges.
struct PageFrame {
    box_rect: Rect,
    width_twips: i64,
    height_twips: i64,
    margin_left: i64,
    margin_right: i64,
    margin_top: i64,
    margin_bottom: i64,
    margin_left_pt: f64,
    margin_top_pt: f64,
    content_width_twips: i64,
}

fn points_to_page_twips(points: f64) -> i64 {
    if !points.is_finite() {
        return TWIP_MIN;
    }
    let twips = (points * PT_TO_TWIPS).round();
    if !twips.is_finite() {
        return TWIP_MIN;
    }
    (twips as i64).clamp(TWIP_MIN, TWIP_MAX)
}

fn margin_bounds(page_pt: f64) -> (f64, f64) {
    let page_pt = if page_pt.is_finite() && page_pt > 0.0 {
        page_pt
    } else {
        0.0
    };
    let cap = MARGIN_CAP_PT.min(page_pt / 4.0);
    let cap = if cap.is_finite() && cap > 0.0 { cap } else { 0.0 };
    (MARGIN_FLOOR_PT.min(cap), cap)
}

fn clamp_margin_pt(raw: f64, page_pt: f64) -> f64 {
    let (floor, cap) = margin_bounds(page_pt);
    if !raw.is_finite() {
        return cap;
    }
    raw.clamp(floor, cap)
}

fn margin_twips(points: f64) -> i64 {
    if !points.is_finite() || points <= 0.0 {
        return 0;
    }
    let twips = (points * PT_TO_TWIPS).round();
    if !twips.is_finite() || twips <= 0.0 {
        return 0;
    }
    twips as i64
}

fn page_frame(box_rect: Rect, elements: &[PageElement]) -> PageFrame {
    let page_w = box_rect.width();
    let page_h = box_rect.height();
    let (left_pt, right_pt, top_pt, bottom_pt) = if elements.is_empty() {
        (
            clamp_margin_pt(MARGIN_CAP_PT, page_w),
            clamp_margin_pt(MARGIN_CAP_PT, page_w),
            clamp_margin_pt(MARGIN_CAP_PT, page_h),
            clamp_margin_pt(MARGIN_CAP_PT, page_h),
        )
    } else {
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for element in elements {
            let bbox = element.bbox();
            min_x = min_x.min(bbox.min_x);
            max_x = max_x.max(bbox.max_x);
            min_y = min_y.min(bbox.min_y);
            max_y = max_y.max(bbox.max_y);
        }
        (
            clamp_margin_pt(min_x - box_rect.min_x, page_w),
            clamp_margin_pt(box_rect.max_x - max_x, page_w),
            clamp_margin_pt(box_rect.max_y - max_y, page_h),
            clamp_margin_pt(min_y - box_rect.min_y, page_h),
        )
    };

    let width_twips = points_to_page_twips(page_w);
    let height_twips = points_to_page_twips(page_h);
    let margin_left = margin_twips(left_pt);
    let margin_right = margin_twips(right_pt);
    let margin_top = margin_twips(top_pt);
    let margin_bottom = margin_twips(bottom_pt);
    let content_width_twips = (width_twips - margin_left - margin_right).max(1);

    PageFrame {
        box_rect,
        width_twips,
        height_twips,
        margin_left,
        margin_right,
        margin_top,
        margin_bottom,
        margin_left_pt: left_pt,
        margin_top_pt: top_pt,
        content_width_twips,
    }
}

fn spacing_before_twips(page: Rect, frame: &PageFrame, previous_min_y: Option<f64>, current: Rect) -> i64 {
    let gap = match previous_min_y {
        None => page.max_y - current.max_y - frame.margin_top_pt,
        Some(previous) => previous - current.max_y,
    };
    if !gap.is_finite() || gap <= SPACING_IGNORE_PT {
        return 0;
    }
    let twips = (gap * PT_TO_TWIPS).round();
    if !twips.is_finite() || twips <= 0.0 {
        return 0;
    }
    (twips as i64).min(SPACING_CAP_TWIPS)
}

fn section_properties_xml(frame: &PageFrame, next_page: bool) -> String {
    let break_type = if next_page {
        "<w:type w:val=\"nextPage\"/>"
    } else {
        ""
    };
    let header = (frame.margin_top / 2).min(720).max(0);
    let footer = (frame.margin_bottom / 2).min(720).max(0);
    format!(
        "<w:sectPr>{break_type}\
            <w:pgSz w:w=\"{w}\" w:h=\"{h}\"/>\
            <w:pgMar w:top=\"{top}\" w:right=\"{right}\" w:bottom=\"{bottom}\" w:left=\"{left}\" w:header=\"{header}\" w:footer=\"{footer}\" w:gutter=\"0\"/>\
        </w:sectPr>",
        w = frame.width_twips,
        h = frame.height_twips,
        top = frame.margin_top,
        right = frame.margin_right,
        bottom = frame.margin_bottom,
        left = frame.margin_left,
        header = header,
        footer = footer,
    )
}

/// Exports the entire PDF document into a native Microsoft Word OpenXML (`.docx`) binary buffer.
///
/// Each PDF page becomes one Word section. The section size is the CropBox, or the
/// MediaBox when CropBox is absent. Margins are the gap to the content, clamped to
/// 36–72 points. A paragraph that starts further right is indented. This remains a
/// reconstruction: there are no floating text boxes and images are not embedded.
pub fn export_document_docx(doc: &mut PdfDocument) -> PdfResult<Vec<u8>> {
    doc.require_copy()?;
    let metadata = extract_metadata(doc).unwrap_or_default();
    let pages = doc.get_pages()?;

    let mut body_xml = String::new();

    if pages.is_empty() {
        let frame = page_frame(Rect::new(0.0, 0.0, 612.0, 792.0), &[]);
        body_xml.push_str(&section_properties_xml(&frame, false));
    }

    for page_idx in 1..=pages.len() {
        let boxes = super::page_boxes(doc, page_idx - 1)?;
        let displayed = boxes.crop.unwrap_or(boxes.media);
        let (elements, fonts) = extract_page_elements(doc, page_idx)?;
        let frame = page_frame(displayed, &elements);
        let mut previous_min_y = None;

        for element in &elements {
            let before = spacing_before_twips(displayed, &frame, previous_min_y, element.bbox());
            match element {
                PageElement::Paragraph(para) => {
                    append_paragraph_xml(&mut body_xml, para, &fonts, &frame, before);
                }
                PageElement::Table(table) => {
                    if before > 0 {
                        append_spacer_paragraph(&mut body_xml, before);
                    }
                    append_table_xml(&mut body_xml, table, &frame);
                }
            }
            previous_min_y = Some(element.bbox().min_y);
        }

        let last = page_idx == pages.len();
        if last {
            body_xml.push_str(&section_properties_xml(&frame, false));
        } else {
            body_xml.push_str("<w:p><w:pPr>");
            body_xml.push_str(&section_properties_xml(&frame, true));
            body_xml.push_str("</w:pPr></w:p>");
        }
    }

    let document_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
                    xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
            <w:body>{body_xml}</w:body>\
        </w:document>"
    );

    let title = escape_xml(metadata.title.as_deref().unwrap_or(""));
    let creator = escape_xml(metadata.author.as_deref().unwrap_or("PDFEngine"));
    let subject = escape_xml(metadata.subject.as_deref().unwrap_or(""));
    let keywords = escape_xml(metadata.keywords.as_deref().unwrap_or(""));

    let core_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
                           xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
                           xmlns:dcterms=\"http://purl.org/dc/terms/\" \
                           xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
            <dc:title>{title}</dc:title>\
            <dc:subject>{subject}</dc:subject>\
            <dc:creator>{creator}</dc:creator>\
            <cp:keywords>{keywords}</cp:keywords>\
        </cp:coreProperties>"
    );

    let mut zip = PkZipWriter::new();
    zip.add_file("[Content_Types].xml", CONTENT_TYPES_XML.as_bytes())?;
    zip.add_file("_rels/.rels", ROOT_RELS_XML.as_bytes())?;
    zip.add_file("docProps/core.xml", core_xml.as_bytes())?;
    zip.add_file("docProps/app.xml", APP_XML.as_bytes())?;
    zip.add_file("word/_rels/document.xml.rels", DOCUMENT_RELS_XML.as_bytes())?;
    zip.add_file("word/styles.xml", STYLES_XML.as_bytes())?;
    zip.add_file("word/document.xml", document_xml.as_bytes())?;

    Ok(zip.finish())
}

fn append_spacer_paragraph(body_xml: &mut String, before_twips: i64) {
    body_xml.push_str(&format!(
        "<w:p><w:pPr><w:spacing w:before=\"{before_twips}\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:p>"
    ));
}

fn left_indent_twips(frame: &PageFrame, min_x: f64) -> i64 {
    let raw = min_x - frame.box_rect.min_x - frame.margin_left_pt;
    if !raw.is_finite() || raw <= INDENT_EPSILON_PT {
        return 0;
    }
    let twips = (raw * PT_TO_TWIPS).round();
    if !twips.is_finite() || twips <= 0.0 {
        return 0;
    }
    (twips as i64).clamp(0, frame.content_width_twips)
}

fn append_paragraph_xml(
    body_xml: &mut String,
    para: &ParagraphBlock,
    fonts: &BTreeMap<String, ResolvedFont>,
    frame: &PageFrame,
    before_twips: i64,
) {
    let size = para.rendered_size();
    let style_id = if size >= 20.0 {
        Some("Heading1")
    } else if size >= 16.0 {
        Some("Heading2")
    } else if size >= 13.5 {
        Some("Heading3")
    } else {
        None
    };

    let jc = match para.alignment {
        TextAlignment::Left => "left",
        TextAlignment::Center => "center",
        TextAlignment::Right => "right",
        TextAlignment::Justified => "both",
    };

    let after_twips = if before_twips > 0 { 0 } else { 120 };
    let indent = match para.alignment {
        TextAlignment::Left | TextAlignment::Justified => left_indent_twips(frame, para.bbox.min_x),
        TextAlignment::Center | TextAlignment::Right => 0,
    };

    body_xml.push_str("<w:p><w:pPr>");
    if let Some(st) = style_id {
        body_xml.push_str(&format!("<w:pStyle w:val=\"{st}\"/>"));
    }
    body_xml.push_str(&format!(
        "<w:spacing w:before=\"{before_twips}\" w:after=\"{after_twips}\" w:line=\"240\" w:lineRule=\"auto\"/>"
    ));
    if indent > 0 {
        body_xml.push_str(&format!("<w:ind w:left=\"{indent}\"/>"));
    }
    body_xml.push_str(&format!("<w:jc w:val=\"{jc}\"/></w:pPr>"));

    for (line_idx, line) in para.lines.iter().enumerate() {
        if line_idx > 0 {
            body_xml.push_str("<w:r><w:t xml:space=\"preserve\"> </w:t></w:r>");
        }
        if line.spans.is_empty() {
            let escaped = escape_xml(&line.text);
            body_xml.push_str(&format!(
                "<w:r><w:t xml:space=\"preserve\">{escaped}</w:t></w:r>"
            ));
        } else {
            for (span_idx, span) in line.spans.iter().enumerate() {
                if span_idx > 0 && !span.text.starts_with(' ') {
                    body_xml.push_str("<w:r><w:t xml:space=\"preserve\"> </w:t></w:r>");
                }
                let (family, is_bold, is_italic) = resolve_font_style(&span.font_name, fonts);
                let half_pts = (span.rendered_size.clamp(6.0, 144.0) * 2.0).round() as u32;
                let escaped_family = escape_xml(&family);
                let escaped_text = escape_xml(&span.text);

                body_xml.push_str("<w:r><w:rPr>");
                body_xml.push_str(&format!(
                    "<w:rFonts w:ascii=\"{escaped_family}\" w:hAnsi=\"{escaped_family}\" w:cs=\"{escaped_family}\"/>"
                ));
                if is_bold {
                    body_xml.push_str("<w:b/><w:bCs/>");
                }
                if is_italic {
                    body_xml.push_str("<w:i/><w:iCs/>");
                }
                body_xml.push_str(&format!(
                    "<w:sz w:val=\"{half_pts}\"/><w:szCs w:val=\"{half_pts}\"/>"
                ));
                body_xml.push_str("</w:rPr>");
                body_xml.push_str(&format!(
                    "<w:t xml:space=\"preserve\">{escaped_text}</w:t></w:r>"
                ));
            }
        }
    }

    body_xml.push_str("</w:p>");
}

fn column_widths(total: i64, col_count: usize) -> Vec<i64> {
    let count = col_count.max(1);
    let total = total.max(count as i64);
    let base = total / count as i64;
    let mut widths = vec![base; count];
    if let Some(last) = widths.last_mut() {
        *last = total - base * (count as i64 - 1);
    }
    widths
}

fn append_table_xml(body_xml: &mut String, table: &DetectedTable, frame: &PageFrame) {
    let col_count = table.col_count.max(1);
    let raw_width = if table.bbox.width().is_finite() {
        (table.bbox.width() * PT_TO_TWIPS).round()
    } else {
        0.0
    };
    let raw_width = if raw_width.is_finite() && raw_width > 0.0 {
        raw_width as i64
    } else {
        col_count as i64
    };
    let table_twips = raw_width.clamp(col_count as i64, frame.content_width_twips.max(col_count as i64));
    let widths = column_widths(table_twips, col_count);
    let indent = left_indent_twips(frame, table.bbox.min_x);
    let indent_xml = if indent > 0 {
        format!("<w:tblInd w:w=\"{indent}\" w:type=\"dxa\"/>")
    } else {
        String::new()
    };

    body_xml.push_str(&format!(
        "<w:tbl>\
            <w:tblPr>\
                <w:tblStyle w:val=\"TableGrid\"/>\
                <w:tblW w:w=\"{table_twips}\" w:type=\"dxa\"/>\
                {indent_xml}\
                <w:tblBorders>\
                    <w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
                    <w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
                    <w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
                    <w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
                    <w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
                    <w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
                </w:tblBorders>\
            </w:tblPr>"
    ));

    body_xml.push_str("<w:tblGrid>");
    for width in &widths {
        body_xml.push_str(&format!("<w:gridCol w:w=\"{width}\"/>"));
    }
    body_xml.push_str("</w:tblGrid>");

    if !table.headers.is_empty() {
        body_xml.push_str("<w:tr><w:trPr><w:tblHeader/></w:trPr>");
        for col_idx in 0..col_count {
            let text = table
                .headers
                .get(col_idx)
                .map(|s| s.as_str())
                .unwrap_or("");
            let escaped = escape_xml(text);
            let width = widths[col_idx];
            body_xml.push_str(&format!(
                "<w:tc>\
                    <w:tcPr>\
                        <w:tcW w:w=\"{width}\" w:type=\"dxa\"/>\
                        <w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F2F2F2\"/>\
                    </w:tcPr>\
                    <w:p>\
                        <w:r>\
                            <w:rPr><w:b/><w:bCs/></w:rPr>\
                            <w:t xml:space=\"preserve\">{escaped}</w:t>\
                        </w:r>\
                    </w:p>\
                </w:tc>"
            ));
        }
        body_xml.push_str("</w:tr>");
    }

    for row in &table.rows {
        body_xml.push_str("<w:tr>");
        for col_idx in 0..col_count {
            let text = row.get(col_idx).map(|s| s.as_str()).unwrap_or("");
            let escaped = escape_xml(text);
            let width = widths[col_idx];
            body_xml.push_str(&format!(
                "<w:tc>\
                    <w:tcPr>\
                        <w:tcW w:w=\"{width}\" w:type=\"dxa\"/>\
                    </w:tcPr>\
                    <w:p>\
                        <w:r>\
                            <w:t xml:space=\"preserve\">{escaped}</w:t>\
                        </w:r>\
                    </w:p>\
                </w:tc>"
            ));
        }
        body_xml.push_str("</w:tr>");
    }

    body_xml.push_str("</w:tbl><w:p/>");
}

const CONTENT_TYPES_XML: &str = "\
<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
    <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
    <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
    <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
    <Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
    <Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
    <Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>\
</Types>";

const ROOT_RELS_XML: &str = "\
<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
    <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
    <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
    <Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>\
</Relationships>";

const DOCUMENT_RELS_XML: &str = "\
<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
    <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
</Relationships>";

const APP_XML: &str = "\
<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\">\
    <Application>PDFEngine</Application>\
    <AppVersion>1.0000</AppVersion>\
</Properties>";

const STYLES_XML: &str = "\
<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
    <w:docDefaults>\
        <w:rPrDefault>\
            <w:rPr>\
                <w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:cs=\"Calibri\"/>\
                <w:sz w:val=\"22\"/>\
                <w:szCs w:val=\"22\"/>\
            </w:rPr>\
        </w:rPrDefault>\
    </w:docDefaults>\
    <w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\">\
        <w:name w:val=\"Normal\"/>\
        <w:qFormat/>\
    </w:style>\
    <w:style w:type=\"paragraph\" w:styleId=\"Heading1\">\
        <w:name w:val=\"heading 1\"/>\
        <w:basedOn w:val=\"Normal\"/>\
        <w:next w:val=\"Normal\"/>\
        <w:qFormat/>\
        <w:pPr><w:keepNext/><w:spacing w:before=\"240\" w:after=\"120\"/></w:pPr>\
        <w:rPr><w:b/><w:bCs/><w:sz w:val=\"40\"/><w:szCs w:val=\"40\"/></w:rPr>\
    </w:style>\
    <w:style w:type=\"paragraph\" w:styleId=\"Heading2\">\
        <w:name w:val=\"heading 2\"/>\
        <w:basedOn w:val=\"Normal\"/>\
        <w:next w:val=\"Normal\"/>\
        <w:qFormat/>\
        <w:pPr><w:keepNext/><w:spacing w:before=\"200\" w:after=\"100\"/></w:pPr>\
        <w:rPr><w:b/><w:bCs/><w:sz w:val=\"32\"/><w:szCs w:val=\"32\"/></w:rPr>\
    </w:style>\
    <w:style w:type=\"paragraph\" w:styleId=\"Heading3\">\
        <w:name w:val=\"heading 3\"/>\
        <w:basedOn w:val=\"Normal\"/>\
        <w:next w:val=\"Normal\"/>\
        <w:qFormat/>\
        <w:pPr><w:keepNext/><w:spacing w:before=\"160\" w:after=\"80\"/></w:pPr>\
        <w:rPr><w:b/><w:bCs/><w:sz w:val=\"28\"/><w:szCs w:val=\"28\"/></w:rPr>\
    </w:style>\
    <w:style w:type=\"table\" w:styleId=\"TableGrid\">\
        <w:name w:val=\"Table Grid\"/>\
        <w:basedOn w:val=\"Normal\"/>\
    </w:style>\
</w:styles>";

/// Minimal in-memory PKZIP archive writer using raw Deflate (method 8) and IEEE 802.3 CRC-32.
struct PkZipEntry {
    name: String,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    local_header_offset: u32,
}

struct PkZipWriter {
    buf: Vec<u8>,
    entries: Vec<PkZipEntry>,
}

impl PkZipWriter {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            entries: Vec::new(),
        }
    }

    fn add_file(&mut self, name: &str, data: &[u8]) -> PdfResult<()> {
        let crc = crc32_ieee(data);
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(data)
            .map_err(|e| PdfError::OperationError(e.to_string()))?;
        let compressed = encoder
            .finish()
            .map_err(|e| PdfError::OperationError(e.to_string()))?;

        let local_header_offset = self.buf.len() as u32;
        let name_bytes = name.as_bytes();
        let compressed_size = compressed.len() as u32;
        let uncompressed_size = data.len() as u32;

        // Local file header signature (0x04034b50)
        self.buf.extend_from_slice(&0x04034b50u32.to_le_bytes());
        // Version needed to extract (20 = 2.0)
        self.buf.extend_from_slice(&20u16.to_le_bytes());
        // General purpose bit flag (0)
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        // Compression method (8 = Deflate)
        self.buf.extend_from_slice(&8u16.to_le_bytes());
        // Last mod file time & date (default 0x0000, 0x0021 = 1980-01-01)
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&0x0021u16.to_le_bytes());
        // CRC-32
        self.buf.extend_from_slice(&crc.to_le_bytes());
        // Compressed & uncompressed sizes
        self.buf.extend_from_slice(&compressed_size.to_le_bytes());
        self.buf.extend_from_slice(&uncompressed_size.to_le_bytes());
        // File name length & extra field length
        self.buf
            .extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        // File name & compressed payload
        self.buf.extend_from_slice(name_bytes);
        self.buf.extend_from_slice(&compressed);

        self.entries.push(PkZipEntry {
            name: name.to_string(),
            crc32: crc,
            compressed_size,
            uncompressed_size,
            local_header_offset,
        });

        Ok(())
    }

    fn finish(mut self) -> Vec<u8> {
        let central_dir_offset = self.buf.len() as u32;

        for entry in &self.entries {
            let name_bytes = entry.name.as_bytes();
            // Central directory file header signature (0x02014b50)
            self.buf.extend_from_slice(&0x02014b50u32.to_le_bytes());
            // Version made by (20) & version needed (20)
            self.buf.extend_from_slice(&20u16.to_le_bytes());
            self.buf.extend_from_slice(&20u16.to_le_bytes());
            // General purpose bit flag (0)
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            // Compression method (8 = Deflate)
            self.buf.extend_from_slice(&8u16.to_le_bytes());
            // Last mod time & date
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            self.buf.extend_from_slice(&0x0021u16.to_le_bytes());
            // CRC-32
            self.buf.extend_from_slice(&entry.crc32.to_le_bytes());
            // Compressed & uncompressed sizes
            self.buf
                .extend_from_slice(&entry.compressed_size.to_le_bytes());
            self.buf
                .extend_from_slice(&entry.uncompressed_size.to_le_bytes());
            // File name length, extra field length, file comment length
            self.buf
                .extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            // Disk number start, internal & external file attributes
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            self.buf.extend_from_slice(&0u32.to_le_bytes());
            // Relative offset of local header
            self.buf
                .extend_from_slice(&entry.local_header_offset.to_le_bytes());
            // File name
            self.buf.extend_from_slice(name_bytes);
        }

        let central_dir_size = (self.buf.len() as u32) - central_dir_offset;
        let entry_count = self.entries.len() as u16;

        // End of central directory record (0x06054b50)
        self.buf.extend_from_slice(&0x06054b50u32.to_le_bytes());
        // Number of this disk & disk where central directory starts
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        // Number of central directory records on this disk & total
        self.buf.extend_from_slice(&entry_count.to_le_bytes());
        self.buf.extend_from_slice(&entry_count.to_le_bytes());
        // Size & offset of central directory
        self.buf.extend_from_slice(&central_dir_size.to_le_bytes());
        self.buf
            .extend_from_slice(&central_dir_offset.to_le_bytes());
        // Comment length
        self.buf.extend_from_slice(&0u16.to_le_bytes());

        self.buf
    }
}

/// Computes standard IEEE 802.3 CRC-32 checksum.
fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cos::{ObjectId, PdfDictionary, PdfName, PdfObject, PdfStream};

    fn create_sample_pdf_with_heading_and_table() -> PdfDocument {
        let mut doc = PdfDocument::empty();

        let f1_id = ObjectId::new(10);
        let mut font_f1 = PdfDictionary::new();
        font_f1.insert("Type", PdfObject::Name(PdfName::new("Font")));
        font_f1.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
        font_f1.insert(
            "BaseFont",
            PdfObject::Name(PdfName::new("Helvetica-Bold")),
        );
        doc.set_object(f1_id, PdfObject::Dictionary(font_f1));

        let f2_id = ObjectId::new(11);
        let mut font_f2 = PdfDictionary::new();
        font_f2.insert("Type", PdfObject::Name(PdfName::new("Font")));
        font_f2.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
        font_f2.insert("BaseFont", PdfObject::Name(PdfName::new("Helvetica")));
        doc.set_object(f2_id, PdfObject::Dictionary(font_f2));

        let mut fonts_dict = PdfDictionary::new();
        fonts_dict.insert("F1", PdfObject::Reference(f1_id));
        fonts_dict.insert("F2", PdfObject::Reference(f2_id));

        let mut resources = PdfDictionary::new();
        resources.insert("Font", PdfObject::Dictionary(fonts_dict));

        // Heading at y=740 (size 22), body paragraph at y=700 (size 11),
        // and a 2x2 ruled lattice table from y=500..600, x=50..350
        let stream_data = b"BT /F1 22 Tf 50 740 Td (Quarterly Financial Report) Tj ET \
            BT /F2 11 Tf 50 700 Td (Executive summary of revenue and growth.) Tj ET \
            50 600 m 350 600 l S \
            50 550 m 350 550 l S \
            50 500 m 350 500 l S \
            50 500 m 50 600 l S \
            200 500 m 200 600 l S \
            350 500 m 350 600 l S \
            BT /F1 11 Tf 60 565 Td (Metric) Tj ET \
            BT /F1 11 Tf 210 565 Td (Value) Tj ET \
            BT /F2 11 Tf 60 515 Td (ARR) Tj ET \
            BT /F2 11 Tf 210 515 Td ($4.2M) Tj ET";

        let content_id = ObjectId::new(4);
        let mut stream_dict = PdfDictionary::new();
        stream_dict.insert("Length", PdfObject::Integer(stream_data.len() as i64));
        doc.set_object(
            content_id,
            PdfObject::Stream(PdfStream::new(stream_dict, stream_data.to_vec())),
        );

        let page_id = ObjectId::new(3);
        let mut page_dict = PdfDictionary::new();
        page_dict.insert("Type", PdfObject::Name(PdfName::new("Page")));
        page_dict.insert("Parent", PdfObject::Reference(ObjectId::new(2)));
        page_dict.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(612),
                PdfObject::Integer(792),
            ]),
        );
        page_dict.insert("Resources", PdfObject::Dictionary(resources));
        page_dict.insert("Contents", PdfObject::Reference(content_id));
        doc.set_object(page_id, PdfObject::Dictionary(page_dict));

        if let Some(pages_dict) = doc
            .objects
            .get_mut(&ObjectId::new(2))
            .and_then(|o| o.as_dict_mut())
        {
            pages_dict.insert(
                "Kids",
                PdfObject::Array(vec![PdfObject::Reference(page_id)]),
            );
            pages_dict.insert("Count", PdfObject::Integer(1));
        }

        doc
    }

    #[test]
    fn test_export_document_markdown_preserves_headings_and_tables() {
        let mut doc = create_sample_pdf_with_heading_and_table();
        let md = export_document_markdown(&mut doc).expect("markdown export should succeed");

        assert!(
            md.contains("# Quarterly Financial Report"),
            "Expected H1 heading in Markdown output, got:\n{md}"
        );
        assert!(
            md.contains("Executive summary of revenue and growth."),
            "Expected body paragraph in Markdown output, got:\n{md}"
        );
        assert!(
            md.contains("| Metric | Value |"),
            "Expected Markdown table header in output, got:\n{md}"
        );
        assert!(
            md.contains("| ARR | $4.2M |"),
            "Expected Markdown table row in output, got:\n{md}"
        );
        // Ensure table cell text wasn't duplicated as standalone paragraphs outside the table
        assert_eq!(
            md.matches("Quarterly Financial Report").count(),
            1,
            "Heading should appear once"
        );
        assert_eq!(
            md.matches("$4.2M").count(),
            1,
            "Table cell value should appear only once (inside table)"
        );
    }

    #[test]
    fn test_export_document_docx_produces_valid_openxml_archive() {
        let mut doc = create_sample_pdf_with_heading_and_table();
        let docx_bytes = export_document_docx(&mut doc).expect("docx export should succeed");

        // Verify PKZIP magic header (PK\x03\x04) and EOCD signature (PK\x05\x06)
        assert!(docx_bytes.len() > 200);
        assert_eq!(&docx_bytes[0..4], b"PK\x03\x04");
        assert!(docx_bytes
            .windows(4)
            .any(|w| w == b"PK\x05\x06"));

        // Verify required OpenXML part names exist in central directory / local headers
        let archive_str = String::from_utf8_lossy(&docx_bytes);
        assert!(archive_str.contains("[Content_Types].xml"));
        assert!(archive_str.contains("_rels/.rels"));
        assert!(archive_str.contains("word/document.xml"));
        assert!(archive_str.contains("word/styles.xml"));
    }

    struct PageSpec {
        width: i64,
        height: i64,
        crop: Option<(i64, i64, i64, i64)>,
        stream: &'static [u8],
    }

    fn document_with_pages(specs: &[PageSpec]) -> PdfDocument {
        let mut doc = PdfDocument::empty();

        let f1_id = ObjectId::new(10);
        let mut font_f1 = PdfDictionary::new();
        font_f1.insert("Type", PdfObject::Name(PdfName::new("Font")));
        font_f1.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
        font_f1.insert("BaseFont", PdfObject::Name(PdfName::new("Helvetica")));
        doc.set_object(f1_id, PdfObject::Dictionary(font_f1));

        let mut fonts_dict = PdfDictionary::new();
        fonts_dict.insert("F1", PdfObject::Reference(f1_id));
        let mut resources = PdfDictionary::new();
        resources.insert("Font", PdfObject::Dictionary(fonts_dict));

        let mut kids = Vec::new();
        for (index, spec) in specs.iter().enumerate() {
            let content_id = ObjectId::new(20 + index as u32 * 2);
            let page_id = ObjectId::new(21 + index as u32 * 2);

            let mut stream_dict = PdfDictionary::new();
            stream_dict.insert("Length", PdfObject::Integer(spec.stream.len() as i64));
            doc.set_object(
                content_id,
                PdfObject::Stream(PdfStream::new(stream_dict, spec.stream.to_vec())),
            );

            let mut page_dict = PdfDictionary::new();
            page_dict.insert("Type", PdfObject::Name(PdfName::new("Page")));
            page_dict.insert("Parent", PdfObject::Reference(ObjectId::new(2)));
            page_dict.insert(
                "MediaBox",
                PdfObject::Array(vec![
                    PdfObject::Integer(0),
                    PdfObject::Integer(0),
                    PdfObject::Integer(spec.width),
                    PdfObject::Integer(spec.height),
                ]),
            );
            if let Some((x0, y0, x1, y1)) = spec.crop {
                page_dict.insert(
                    "CropBox",
                    PdfObject::Array(vec![
                        PdfObject::Integer(x0),
                        PdfObject::Integer(y0),
                        PdfObject::Integer(x1),
                        PdfObject::Integer(y1),
                    ]),
                );
            }
            page_dict.insert("Resources", PdfObject::Dictionary(resources.clone()));
            page_dict.insert("Contents", PdfObject::Reference(content_id));
            doc.set_object(page_id, PdfObject::Dictionary(page_dict));
            kids.push(PdfObject::Reference(page_id));
        }

        if let Some(pages_dict) = doc
            .objects
            .get_mut(&ObjectId::new(2))
            .and_then(|object| object.as_dict_mut())
        {
            pages_dict.insert("Kids", PdfObject::Array(kids));
            pages_dict.insert("Count", PdfObject::Integer(specs.len() as i64));
        }

        doc
    }

    fn inflate_docx_part(archive: &[u8], name: &str) -> String {
        use std::io::Read;

        let mut offset = 0;
        while offset + 30 <= archive.len() && &archive[offset..offset + 4] == b"PK\x03\x04" {
            let method = u16::from_le_bytes([archive[offset + 8], archive[offset + 9]]);
            let compressed_size =
                u32::from_le_bytes(archive[offset + 18..offset + 22].try_into().unwrap()) as usize;
            let name_len =
                u16::from_le_bytes([archive[offset + 26], archive[offset + 27]]) as usize;
            let extra_len =
                u16::from_le_bytes([archive[offset + 28], archive[offset + 29]]) as usize;
            let name_start = offset + 30;
            let name_end = name_start + name_len;
            let data_start = name_end + extra_len;
            let data_end = data_start + compressed_size;
            let entry_name = std::str::from_utf8(&archive[name_start..name_end]).unwrap_or("");
            if entry_name == name {
                assert_eq!(method, 8, "document.xml is stored deflated");
                let mut decoder =
                    flate2::read::DeflateDecoder::new(&archive[data_start..data_end]);
                let mut xml = String::new();
                decoder
                    .read_to_string(&mut xml)
                    .expect("deflated part should inflate");
                return xml;
            }
            offset = data_end;
        }
        panic!("part {name} was not in the docx archive");
    }

    #[test]
    fn docx_section_uses_the_page_box_and_indents_a_paragraph_further_right() {
        let stream =
            b"BT /F1 12 Tf 40 460 Td (Left edge note.) Tj ET BT /F1 12 Tf 180 400 Td (Indented body.) Tj ET";
        let mut doc = document_with_pages(&[PageSpec {
            width: 400,
            height: 500,
            crop: None,
            stream,
        }]);
        let xml = inflate_docx_part(
            &export_document_docx(&mut doc).expect("docx"),
            "word/document.xml",
        );

        assert!(
            xml.contains("<w:pgSz w:w=\"8000\" w:h=\"10000\"/>"),
            "page box should be 400 by 500 points, got:\n{xml}"
        );
        assert!(
            xml.contains("w:left=\"800\""),
            "left margin should be the 40 pt content edge, got:\n{xml}"
        );
        assert!(
            xml.contains("<w:ind w:left=\"2800\"/>"),
            "the paragraph at x=180 should keep the leftover as a left indent, got:\n{xml}"
        );
        assert!(
            !xml.contains("<w:br w:type=\"page\"/>"),
            "a single page does not add a page break"
        );
    }

    #[test]
    fn docx_letter_text_at_one_inch_keeps_the_letter_page_and_that_margin() {
        let stream = b"BT /F1 12 Tf 72 720 Td (Inch mark.) Tj ET";
        let mut doc = document_with_pages(&[PageSpec {
            width: 612,
            height: 792,
            crop: None,
            stream,
        }]);
        let xml = inflate_docx_part(
            &export_document_docx(&mut doc).expect("docx"),
            "word/document.xml",
        );

        assert!(xml.contains("<w:pgSz w:w=\"12240\" w:h=\"15840\"/>"), "{xml}");
        assert_eq!(
            xml.matches("w:left=\"1440\"").count(),
            1,
            "the one-inch edge is the margin, not an indent, got:\n{xml}"
        );
        assert!(!xml.contains("<w:ind "), "{xml}");
    }

    #[test]
    fn docx_page_size_follows_the_crop_box() {
        let mut doc = document_with_pages(&[PageSpec {
            width: 612,
            height: 792,
            crop: Some((0, 0, 400, 500)),
            stream: b"",
        }]);
        let xml = inflate_docx_part(
            &export_document_docx(&mut doc).expect("docx"),
            "word/document.xml",
        );

        assert!(xml.contains("<w:pgSz w:w=\"8000\" w:h=\"10000\"/>"), "{xml}");
        assert!(!xml.contains("12240"), "{xml}");
        assert!(!xml.contains("15840"), "{xml}");
    }

    #[test]
    fn docx_pages_of_different_sizes_are_separate_sections() {
        let stream = b"BT /F1 12 Tf 40 80 Td (Page line.) Tj ET";
        let mut doc = document_with_pages(&[
            PageSpec {
                width: 400,
                height: 500,
                crop: None,
                stream,
            },
            PageSpec {
                width: 200,
                height: 300,
                crop: None,
                stream,
            },
        ]);
        let xml = inflate_docx_part(
            &export_document_docx(&mut doc).expect("docx"),
            "word/document.xml",
        );

        assert_eq!(xml.matches("<w:sectPr>").count(), 2, "{xml}");
        assert_eq!(xml.matches("w:val=\"nextPage\"").count(), 1, "{xml}");
        assert!(xml.contains("<w:pgSz w:w=\"8000\" w:h=\"10000\"/>"), "{xml}");
        assert!(xml.contains("<w:pgSz w:w=\"4000\" w:h=\"6000\"/>"), "{xml}");
        assert!(
            !xml.contains("<w:br w:type=\"page\"/>"),
            "the section break is the page break, got:\n{xml}"
        );
    }
}
