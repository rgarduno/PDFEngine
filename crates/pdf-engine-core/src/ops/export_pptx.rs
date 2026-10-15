//! Slide export of reconstructed pages to Office Open XML (`.pptx`).
//!
//! One slide per page. Paragraphs and tables come from the same reconstructor
//! the Word export uses. A paragraph is a text box on its box. A table is a
//! text table. Images are not embedded. This is not a binary `.ppt` file.

use std::collections::BTreeMap;
use std::io::Write;

use flate2::write::DeflateEncoder;
use flate2::Compression;

use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::fonts::{resolve_page_fonts, ResolvedFont};
use crate::layout::{LayoutReconstructor, ParagraphBlock, Rect, TextAlignment};
use crate::ops::layers::resolve_page_hidden_ocg_props;
use crate::stream::parser::{build_ast_from_operations, ContentStreamTokenizer};
use crate::tables::{detect_borderless_tables, detect_lattice_tables, DetectedTable};

/// One point, in English Metric Units. 914400 EMU is one inch, and one inch is 72 points.
const PT_TO_EMU: f64 = 12_700.0;
/// PowerPoint's largest slide side, 56 inches.
const EMU_MAX: i64 = 56 * 914_400;
/// Smallest slide side this export writes, one point.
const EMU_MIN: i64 = 12_700;
const LETTER_W: f64 = 612.0;
const LETTER_H: f64 = 792.0;
const CELL_CHAR_LIMIT: usize = 32_767;
const TABLE_FONT_HUNDREDTHS: i64 = 1_200;

/// A block on one page, in the same order the Word export uses.
enum PageElement {
    Paragraph(ParagraphBlock),
    Table(DetectedTable),
}

impl PageElement {
    fn bbox(&self) -> Rect {
        match self {
            PageElement::Paragraph(paragraph) => paragraph.bbox,
            PageElement::Table(table) => table.bbox,
        }
    }
}

/// Writes one slide per page into a PPTX package.
///
/// The deck has one slide size, taken from the first page's CropBox, or its
/// MediaBox when CropBox is absent. A later page is stretched onto that size.
/// A document with no pages still emits one empty US Letter slide. A page the
/// table detector refuses fails this export.
pub fn export_document_pptx(doc: &mut PdfDocument) -> PdfResult<Vec<u8>> {
    doc.require_copy()?;
    let page_count = doc.get_pages()?.len();
    if page_count == 0 {
        let (cx, cy) = (slide_emu(LETTER_W, LETTER_W), slide_emu(LETTER_H, LETTER_H));
        return write_package(cx, cy, &[String::new()]);
    }

    let first = displayed_box(doc, 0)?;
    let deck_w = positive_side(first.width(), LETTER_W);
    let deck_h = positive_side(first.height(), LETTER_H);
    let cx = slide_emu(deck_w, LETTER_W);
    let cy = slide_emu(deck_h, LETTER_H);

    let mut slides = Vec::with_capacity(page_count);
    for page_idx in 1..=page_count {
        let page = displayed_box(doc, page_idx - 1)?;
        let (elements, fonts) = extract_page_elements(doc, page_idx)?;
        slides.push(slide_body(&elements, &fonts, &page, deck_w, deck_h, cx, cy));
    }
    write_package(cx, cy, &slides)
}

fn displayed_box(doc: &mut PdfDocument, page_index: usize) -> PdfResult<Rect> {
    let boxes = super::page_boxes(doc, page_index)?;
    Ok(boxes.crop.unwrap_or(boxes.media))
}

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
    let paragraphs = LayoutReconstructor::new(&ast)
        .with_resolved(&fonts)
        .with_hidden_ocg_props(hidden_ocg_props)
        .reconstruct()?;

    let mut tables = detect_lattice_tables(&ast, &paragraphs, page_idx)?;
    if tables.is_empty() {
        tables = detect_borderless_tables(&paragraphs, page_idx);
    }

    let mut elements = Vec::new();
    for paragraph in paragraphs {
        if !tables.iter().any(|table| paragraph_inside_table(&paragraph, table)) {
            elements.push(PageElement::Paragraph(paragraph));
        }
    }
    for table in tables {
        elements.push(PageElement::Table(table));
    }
    elements.sort_by(|left, right| {
        let left_box = left.bbox();
        let right_box = right.bbox();
        right_box
            .max_y
            .partial_cmp(&left_box.max_y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left_box
                    .min_x
                    .partial_cmp(&right_box.min_x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    Ok((elements, fonts))
}

fn paragraph_inside_table(paragraph: &ParagraphBlock, table: &DetectedTable) -> bool {
    let cx = (paragraph.bbox.min_x + paragraph.bbox.max_x) * 0.5;
    let cy = (paragraph.bbox.min_y + paragraph.bbox.max_y) * 0.5;
    let margin = 2.0;
    cx >= table.bbox.min_x - margin
        && cx <= table.bbox.max_x + margin
        && cy >= table.bbox.min_y - margin
        && cy <= table.bbox.max_y + margin
}

fn slide_body(
    elements: &[PageElement],
    fonts: &BTreeMap<String, ResolvedFont>,
    page: &Rect,
    deck_w: f64,
    deck_h: f64,
    deck_cx: i64,
    deck_cy: i64,
) -> String {
    let mut next_id = 2u32;
    let mut body = String::new();
    for element in elements {
        match element {
            PageElement::Paragraph(paragraph) => {
                if let Some(shape) = paragraph_shape(
                    paragraph, fonts, page, deck_w, deck_h, deck_cx, deck_cy, next_id,
                ) {
                    body.push_str(&shape);
                    next_id += 1;
                }
            }
            PageElement::Table(table) => {
                if let Some(shape) =
                    table_shape(table, page, deck_w, deck_h, deck_cx, deck_cy, next_id)
                {
                    body.push_str(&shape);
                    next_id += 1;
                }
            }
        }
    }
    body
}

fn paragraph_shape(
    paragraph: &ParagraphBlock,
    fonts: &BTreeMap<String, ResolvedFont>,
    page: &Rect,
    deck_w: f64,
    deck_h: f64,
    deck_cx: i64,
    deck_cy: i64,
    shape_id: u32,
) -> Option<String> {
    let (x, y, cx, cy) = map_box(page, deck_w, deck_h, deck_cx, deck_cy, paragraph.bbox);
    let align = match paragraph.alignment {
        TextAlignment::Center => "ctr",
        TextAlignment::Right => "r",
        TextAlignment::Justified => "just",
        TextAlignment::Left => "l",
    };
    let mut paragraphs_xml = String::new();
    for line in &paragraph.lines {
        let runs = line_runs(line, fonts, paragraph);
        if runs.is_empty() {
            continue;
        }
        paragraphs_xml.push_str(&format!("<a:p><a:pPr algn=\"{align}\"/>{runs}</a:p>"));
    }
    if paragraphs_xml.is_empty() {
        return None;
    }
    Some(format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{shape_id}\" name=\"Text {shape_id}\"/>\
        <p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm>\
        <a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/>\
        </a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>\
        <a:noFill/><a:ln><a:noFill/></a:ln></p:spPr><p:txBody>\
        <a:bodyPr wrap=\"square\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" rtlCol=\"0\" anchor=\"t\"/>\
        <a:lstStyle/>{paragraphs_xml}</p:txBody></p:sp>"
    ))
}

fn line_runs(
    line: &crate::layout::TextLine,
    fonts: &BTreeMap<String, ResolvedFont>,
    paragraph: &ParagraphBlock,
) -> String {
    if line.spans.is_empty() {
        return text_run(&line.text, "Calibri", false, false, hundredths(paragraph.rendered_size()));
    }
    let mut runs = String::new();
    for span in &line.spans {
        let (family, bold, italic) = font_face(&span.font_name, fonts);
        let size = if span.rendered_size.is_finite() && span.rendered_size > 0.0 {
            span.rendered_size
        } else {
            paragraph.rendered_size()
        };
        runs.push_str(&text_run(&span.text, &family, bold, italic, hundredths(size)));
    }
    runs
}

fn text_run(text: &str, family: &str, bold: bool, italic: bool, size: i64) -> String {
    let value = truncate_chars(text, CELL_CHAR_LIMIT);
    if value.is_empty() {
        return String::new();
    }
    let bold_attr = if bold { " b=\"1\"" } else { "" };
    let italic_attr = if italic { " i=\"1\"" } else { "" };
    let space = if needs_preserve(&value) {
        " xml:space=\"preserve\""
    } else {
        ""
    };
    format!(
        "<a:r><a:rPr lang=\"en-US\" sz=\"{size}\" dirty=\"0\"{bold_attr}{italic_attr}>\
        <a:solidFill><a:srgbClr val=\"000000\"/></a:solidFill>\
        <a:latin typeface=\"{}\"/></a:rPr><a:t{space}>{}</a:t></a:r>",
        xml_text(family),
        xml_text(&value)
    )
}

fn table_shape(
    table: &DetectedTable,
    page: &Rect,
    deck_w: f64,
    deck_h: f64,
    deck_cx: i64,
    deck_cy: i64,
    shape_id: u32,
) -> Option<String> {
    let grid = table_grid(table);
    if grid.is_empty() || grid[0].is_empty() {
        return None;
    }
    let (x, y, cx, cy) = map_box(page, deck_w, deck_h, deck_cx, deck_cy, table.bbox);
    let cols = grid[0].len();
    let rows = grid.len();
    let col_widths = split_emu(cx, cols);
    let row_heights = split_emu(cy, rows);
    let mut grid_xml = String::from("<a:tblGrid>");
    for width in &col_widths {
        grid_xml.push_str(&format!("<a:gridCol w=\"{width}\"/>"));
    }
    grid_xml.push_str("</a:tblGrid>");
    let mut rows_xml = String::new();
    for (row_idx, row) in grid.iter().enumerate() {
        let height = row_heights[row_idx];
        rows_xml.push_str(&format!("<a:tr h=\"{height}\">"));
        for (col_idx, cell) in row.iter().enumerate() {
            let width = col_widths.get(col_idx).copied().unwrap_or(EMU_MIN);
            let run = text_run(cell, "Arial", false, false, TABLE_FONT_HUNDREDTHS);
            rows_xml.push_str(&format!(
                "<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p>{run}</a:p></a:txBody>\
                <a:tcPr><a:tcW w=\"{width}\"/></a:tcPr></a:tc>"
            ));
        }
        rows_xml.push_str("</a:tr>");
    }
    Some(format!(
        "<p:graphicFrame><p:nvGraphicFramePr>\
        <p:cNvPr id=\"{shape_id}\" name=\"Table {shape_id}\"/>\
        <p:cNvGraphicFramePr><a:graphicFrameLocks noGrp=\"1\"/></p:cNvGraphicFramePr>\
        <p:nvPr/></p:nvGraphicFramePr><p:xfrm>\
        <a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></p:xfrm>\
        <a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\">\
        <a:tbl><a:tblPr/>{grid_xml}{rows_xml}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"
    ))
}

fn table_grid(table: &DetectedTable) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    if !table.headers.is_empty() {
        rows.push(table.headers.clone());
    }
    rows.extend(table.rows.iter().cloned());
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return Vec::new();
    }
    for row in &mut rows {
        row.resize(cols, String::new());
        for cell in row.iter_mut() {
            *cell = truncate_chars(cell, CELL_CHAR_LIMIT);
        }
    }
    rows
}

fn map_box(
    page: &Rect,
    deck_w: f64,
    deck_h: f64,
    deck_cx: i64,
    deck_cy: i64,
    bbox: Rect,
) -> (i64, i64, i64, i64) {
    let page_w = positive_side(page.width(), deck_w);
    let page_h = positive_side(page.height(), deck_h);
    let scale_x = deck_w / page_w;
    let scale_y = deck_h / page_h;
    let x_pt = (bbox.min_x - page.min_x) * scale_x;
    let y_pt = (page.max_y - bbox.max_y) * scale_y;
    let w_pt = bbox.width() * scale_x;
    let h_pt = bbox.height() * scale_y;
    let mut x = emu_offset(x_pt);
    let mut y = emu_offset(y_pt);
    let mut cx = emu_extent(w_pt);
    let mut cy = emu_extent(h_pt);
    if x >= deck_cx {
        x = (deck_cx - EMU_MIN).max(0);
    }
    if y >= deck_cy {
        y = (deck_cy - EMU_MIN).max(0);
    }
    if x + cx > deck_cx {
        cx = (deck_cx - x).max(EMU_MIN).min(deck_cx);
        if x + cx > deck_cx {
            x = (deck_cx - cx).max(0);
        }
    }
    if y + cy > deck_cy {
        cy = (deck_cy - y).max(EMU_MIN).min(deck_cy);
        if y + cy > deck_cy {
            y = (deck_cy - cy).max(0);
        }
    }
    (x, y, cx, cy)
}

fn split_emu(total: i64, count: usize) -> Vec<i64> {
    if count == 0 {
        return Vec::new();
    }
    let total = total.max(EMU_MIN);
    let base = total / count as i64;
    let mut widths = vec![base.max(1); count];
    let used: i64 = widths.iter().sum();
    widths[count - 1] += total - used;
    if widths[count - 1] < 1 {
        widths[count - 1] = 1;
    }
    widths
}

fn positive_side(points: f64, fallback: f64) -> f64 {
    if points.is_finite() && points > 0.0 {
        points
    } else {
        fallback
    }
}

fn slide_emu(points: f64, fallback: f64) -> i64 {
    let points = positive_side(points, fallback);
    let emu = (points * PT_TO_EMU).round();
    if !emu.is_finite() {
        return slide_emu(fallback, LETTER_W);
    }
    (emu as i64).clamp(EMU_MIN, EMU_MAX)
}

fn emu_offset(points: f64) -> i64 {
    if !points.is_finite() || points <= 0.0 {
        return 0;
    }
    let emu = (points * PT_TO_EMU).round();
    if !emu.is_finite() || emu <= 0.0 {
        return 0;
    }
    (emu as i64).min(EMU_MAX)
}

fn emu_extent(points: f64) -> i64 {
    if !points.is_finite() || points <= 0.0 {
        return EMU_MIN;
    }
    let emu = (points * PT_TO_EMU).round();
    if !emu.is_finite() || emu < 1.0 {
        return EMU_MIN;
    }
    (emu as i64).clamp(1, EMU_MAX)
}

fn hundredths(points: f64) -> i64 {
    let points = if points.is_finite() && points > 0.0 {
        points
    } else {
        12.0
    };
    let value = (points * 100.0).round();
    if !value.is_finite() {
        return TABLE_FONT_HUNDREDTHS;
    }
    (value as i64).clamp(100, 400_000)
}

fn font_face(font_name: &str, fonts: &BTreeMap<String, ResolvedFont>) -> (String, bool, bool) {
    let clean = font_name.trim_start_matches('/');
    if let Some(resolved) = fonts.get(clean) {
        let family = match resolved.family.as_str() {
            "serif" => "Times New Roman",
            "monospace" => "Courier New",
            "sans-serif" => "Arial",
            other if !other.is_empty() => other,
            _ => "Calibri",
        };
        let bold = resolved.weight >= 700 || name_has(clean, &["bold", "-bd"]) || clean.to_ascii_lowercase().ends_with("bd");
        let italic = resolved.style == "italic" || name_has(clean, &["italic", "oblique"]) || clean.to_ascii_lowercase().ends_with("it");
        return (family.to_string(), bold, italic);
    }
    let lower = clean.to_ascii_lowercase();
    let family = if lower.contains("times") || lower.contains("serif") {
        "Times New Roman"
    } else if lower.contains("courier") || lower.contains("mono") {
        "Courier New"
    } else if lower.contains("helvetica") || lower.contains("arial") {
        "Arial"
    } else {
        "Calibri"
    };
    (
        family.to_string(),
        name_has(clean, &["bold", "-bd"]) || lower.ends_with("bd"),
        name_has(clean, &["italic", "oblique"]) || lower.ends_with("it"),
    )
}

fn name_has(name: &str, needles: &[&str]) -> bool {
    let lower = name.to_ascii_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

fn needs_preserve(value: &str) -> bool {
    value.starts_with(' ')
        || value.ends_with(' ')
        || value.starts_with('\t')
        || value.ends_with('\t')
        || value.contains(['\n', '\r', '\t'])
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn xml_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\u{9}' | '\u{A}' | '\u{D}' => out.push(ch),
            c if c.is_control() => {}
            '\u{FFFE}' | '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

fn write_package(cx: i64, cy: i64, slides: &[String]) -> PdfResult<Vec<u8>> {
    let count = slides.len().max(1);
    let mut zip = PkZipWriter::new();
    zip.add_file("[Content_Types].xml", content_types(count).as_bytes())?;
    zip.add_file("_rels/.rels", ROOT_RELS.as_bytes())?;
    zip.add_file("ppt/presentation.xml", presentation_xml(cx, cy, count).as_bytes())?;
    zip.add_file(
        "ppt/_rels/presentation.xml.rels",
        presentation_rels(count).as_bytes(),
    )?;
    zip.add_file("ppt/slideMasters/slideMaster1.xml", SLIDE_MASTER.as_bytes())?;
    zip.add_file(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        MASTER_RELS.as_bytes(),
    )?;
    zip.add_file("ppt/slideLayouts/slideLayout1.xml", SLIDE_LAYOUT.as_bytes())?;
    zip.add_file(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        LAYOUT_RELS.as_bytes(),
    )?;
    zip.add_file("ppt/theme/theme1.xml", THEME.as_bytes())?;
    zip.add_file(
        "ppt/slides/_rels/slide1.xml.rels",
        SLIDE_RELS.as_bytes(),
    )?;
    for (index, body) in slides.iter().enumerate() {
        let path = format!("ppt/slides/slide{}.xml", index + 1);
        zip.add_file(&path, slide_xml(cx, cy, body).as_bytes())?;
        if index > 0 {
            let rels = format!("ppt/slides/_rels/slide{}.xml.rels", index + 1);
            zip.add_file(&rels, SLIDE_RELS.as_bytes())?;
        }
    }
    if slides.is_empty() {
        zip.add_file("ppt/slides/slide1.xml", slide_xml(cx, cy, "").as_bytes())?;
    }
    Ok(zip.finish())
}

fn content_types(slide_count: usize) -> String {
    let mut overrides = String::from(
        "<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
        <Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
        <Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
        <Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>",
    );
    for index in 1..=slide_count {
        overrides.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{index}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        {overrides}</Types>"
    )
}

fn presentation_xml(cx: i64, cy: i64, slide_count: usize) -> String {
    let mut slides = String::new();
    for index in 0..slide_count {
        let rel = index + 2;
        let id = 256 + index;
        slides.push_str(&format!("<p:sldId id=\"{id}\" r:id=\"rId{rel}\"/>"));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <p:presentation xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
        xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
        xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
        <p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst>\
        <p:sldIdLst>{slides}</p:sldIdLst>\
        <p:sldSz cx=\"{cx}\" cy=\"{cy}\"/>\
        <p:notesSz cx=\"6858000\" cy=\"9144000\"/>\
        </p:presentation>"
    )
}

fn presentation_rels(slide_count: usize) -> String {
    let mut rels = String::from(
        "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>",
    );
    for index in 1..=slide_count {
        let rel = index + 1;
        rels.push_str(&format!(
            "<Relationship Id=\"rId{rel}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{index}.xml\"/>"
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels}</Relationships>"
    )
}

fn slide_xml(cx: i64, cy: i64, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
        xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
        xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
        <p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/>\
        </p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/>\
        <a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm></p:grpSpPr>\
        {body}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
    )
}

const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/>\
</Relationships>";

const SLIDE_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>\
</Relationships>";

const MASTER_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"../theme/theme1.xml\"/>\
</Relationships>";

const LAYOUT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/>\
</Relationships>";

const SLIDE_MASTER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<p:sldMaster xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
<p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg>\
<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
</p:spTree></p:cSld>\
<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>\
<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst>\
<p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz=\"1800\"/></a:lvl1pPr></p:titleStyle>\
<p:bodyStyle><a:lvl1pPr><a:defRPr sz=\"1400\"/></a:lvl1pPr></p:bodyStyle>\
<p:otherStyle><a:lvl1pPr><a:defRPr sz=\"1200\"/></a:lvl1pPr></p:otherStyle></p:txStyles>\
</p:sldMaster>";

const SLIDE_LAYOUT: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<p:sldLayout xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" type=\"blank\" preserve=\"1\">\
<p:cSld name=\"Blank\"><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>";

const THEME: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Office\">\
<a:themeElements><a:clrScheme name=\"Office\">\
<a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1>\
<a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>\
<a:dk2><a:srgbClr val=\"1F497D\"/></a:dk2><a:lt2><a:srgbClr val=\"EEECE1\"/></a:lt2>\
<a:accent1><a:srgbClr val=\"4F81BD\"/></a:accent1><a:accent2><a:srgbClr val=\"C0504D\"/></a:accent2>\
<a:accent3><a:srgbClr val=\"9BBB59\"/></a:accent3><a:accent4><a:srgbClr val=\"8064A2\"/></a:accent4>\
<a:accent5><a:srgbClr val=\"4BACC6\"/></a:accent5><a:accent6><a:srgbClr val=\"F79646\"/></a:accent6>\
<a:hlink><a:srgbClr val=\"0000FF\"/></a:hlink><a:folHlink><a:srgbClr val=\"800080\"/></a:folHlink>\
</a:clrScheme><a:fontScheme name=\"Office\">\
<a:majorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont>\
<a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont>\
</a:fontScheme><a:fmtScheme name=\"Office\">\
<a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:fillStyleLst>\
<a:lnStyleLst><a:ln w=\"6350\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln><a:ln w=\"12700\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln><a:ln w=\"19050\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln></a:lnStyleLst>\
<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>\
<a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:bgFillStyleLst>\
</a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>";

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

        self.buf.extend_from_slice(&0x04034b50u32.to_le_bytes());
        self.buf.extend_from_slice(&20u16.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&8u16.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&0x0021u16.to_le_bytes());
        self.buf.extend_from_slice(&crc.to_le_bytes());
        self.buf.extend_from_slice(&compressed_size.to_le_bytes());
        self.buf.extend_from_slice(&uncompressed_size.to_le_bytes());
        self.buf
            .extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
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
            self.buf.extend_from_slice(&0x02014b50u32.to_le_bytes());
            self.buf.extend_from_slice(&20u16.to_le_bytes());
            self.buf.extend_from_slice(&20u16.to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            self.buf.extend_from_slice(&8u16.to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            self.buf.extend_from_slice(&0x0021u16.to_le_bytes());
            self.buf.extend_from_slice(&entry.crc32.to_le_bytes());
            self.buf
                .extend_from_slice(&entry.compressed_size.to_le_bytes());
            self.buf
                .extend_from_slice(&entry.uncompressed_size.to_le_bytes());
            self.buf
                .extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            self.buf.extend_from_slice(&0u16.to_le_bytes());
            self.buf.extend_from_slice(&0u32.to_le_bytes());
            self.buf
                .extend_from_slice(&entry.local_header_offset.to_le_bytes());
            self.buf.extend_from_slice(name_bytes);
        }

        let central_dir_size = (self.buf.len() as u32) - central_dir_offset;
        let entry_count = self.entries.len() as u16;
        self.buf.extend_from_slice(&0x06054b50u32.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf.extend_from_slice(&entry_count.to_le_bytes());
        self.buf.extend_from_slice(&entry_count.to_le_bytes());
        self.buf.extend_from_slice(&central_dir_size.to_le_bytes());
        self.buf
            .extend_from_slice(&central_dir_offset.to_le_bytes());
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        self.buf
    }
}

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

    struct PageSpec {
        width: i64,
        height: i64,
        crop: Option<(i64, i64, i64, i64)>,
        stream: Vec<u8>,
    }

    fn document_with_pages(specs: &[PageSpec]) -> PdfDocument {
        let mut doc = PdfDocument::empty();

        let font_id = ObjectId::new(10);
        let mut font = PdfDictionary::new();
        font.insert("Type", PdfObject::Name(PdfName::new("Font")));
        font.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
        font.insert("BaseFont", PdfObject::Name(PdfName::new("Helvetica")));
        doc.set_object(font_id, PdfObject::Dictionary(font));

        let mut fonts = PdfDictionary::new();
        fonts.insert("F1", PdfObject::Reference(font_id));
        let mut resources = PdfDictionary::new();
        resources.insert("Font", PdfObject::Dictionary(fonts));

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

            let mut page = PdfDictionary::new();
            page.insert("Type", PdfObject::Name(PdfName::new("Page")));
            page.insert("Parent", PdfObject::Reference(ObjectId::new(2)));
            page.insert(
                "MediaBox",
                PdfObject::Array(vec![
                    PdfObject::Integer(0),
                    PdfObject::Integer(0),
                    PdfObject::Integer(spec.width),
                    PdfObject::Integer(spec.height),
                ]),
            );
            if let Some((x0, y0, x1, y1)) = spec.crop {
                page.insert(
                    "CropBox",
                    PdfObject::Array(vec![
                        PdfObject::Integer(x0),
                        PdfObject::Integer(y0),
                        PdfObject::Integer(x1),
                        PdfObject::Integer(y1),
                    ]),
                );
            }
            page.insert("Resources", PdfObject::Dictionary(resources.clone()));
            page.insert("Contents", PdfObject::Reference(content_id));
            doc.set_object(page_id, PdfObject::Dictionary(page));
            kids.push(PdfObject::Reference(page_id));
        }

        if let Some(pages) = doc
            .objects
            .get_mut(&ObjectId::new(2))
            .and_then(|object| object.as_dict_mut())
        {
            pages.insert("Kids", PdfObject::Array(kids));
            pages.insert("Count", PdfObject::Integer(specs.len() as i64));
        }
        doc
    }

    fn inflate_part(archive: &[u8], name: &str) -> String {
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
                assert_eq!(method, 8, "{name} is stored deflated");
                let mut decoder = flate2::read::DeflateDecoder::new(&archive[data_start..data_end]);
                let mut xml = String::new();
                decoder.read_to_string(&mut xml).expect("part inflates");
                return xml;
            }
            offset = data_end;
        }
        panic!("part {name} was not in the presentation");
    }

    fn lattice(top_left: &str, top_right: &str, bottom_left: &str, bottom_right: &str) -> Vec<u8> {
        format!(
            "100 700 m 300 700 l S\n\
            100 650 m 300 650 l S\n\
            100 600 m 300 600 l S\n\
            100 600 m 100 700 l S\n\
            200 600 m 200 700 l S\n\
            300 600 m 300 700 l S\n\
            BT /F1 10 Tf 110 670 Tm ({top_left}) Tj ET\n\
            BT /F1 10 Tf 210 670 Tm ({top_right}) Tj ET\n\
            BT /F1 10 Tf 110 620 Tm ({bottom_left}) Tj ET\n\
            BT /F1 10 Tf 210 620 Tm ({bottom_right}) Tj ET\n"
        )
        .into_bytes()
    }

    #[test]
    fn a_document_without_pages_still_opens_as_one_letter_slide() {
        let mut doc = PdfDocument::empty();
        let bytes = export_document_pptx(&mut doc).expect("empty document exports");
        assert!(bytes.windows(4).any(|window| window == b"PK\x05\x06"));
        let presentation = inflate_part(&bytes, "ppt/presentation.xml");
        assert!(presentation.contains("<p:sldSz cx=\"7772400\" cy=\"10058400\"/>"), "{presentation}");
        assert_eq!(presentation.matches("<p:sldId ").count(), 1);
        let slide = inflate_part(&bytes, "ppt/slides/slide1.xml");
        assert!(slide.contains("<p:sld"));
        assert!(!slide.contains("<a:t"));
        assert!(!slide.contains("<a:tbl"));
        assert!(!slide.contains("a:blip"));
        assert!(inflate_part(&bytes, "[Content_Types].xml").contains("presentation.main+xml"));
        assert!(inflate_part(&bytes, "ppt/theme/theme1.xml").contains("<a:theme"));
    }

    #[test]
    fn each_page_is_a_slide_and_a_paragraph_keeps_its_box() {
        let stream =
            b"BT /F1 12 Tf 40 460 Td (Left edge note.) Tj ET BT /F1 12 Tf 180 400 Td (Indented body.) Tj ET";
        let mut doc = document_with_pages(&[PageSpec {
            width: 400,
            height: 500,
            crop: None,
            stream: stream.to_vec(),
        }]);
        let bytes = export_document_pptx(&mut doc).expect("pptx");
        let presentation = inflate_part(&bytes, "ppt/presentation.xml");
        assert!(
            presentation.contains("<p:sldSz cx=\"5080000\" cy=\"6350000\"/>"),
            "{presentation}"
        );
        assert_eq!(presentation.matches("<p:sldId ").count(), 1);
        let slide = inflate_part(&bytes, "ppt/slides/slide1.xml");
        assert_eq!(slide.matches("txBox=\"1\"").count(), 2, "{slide}");
        assert!(slide.contains("<a:t>Left edge note.</a:t>"), "{slide}");
        assert!(slide.contains("<a:t>Indented body.</a:t>"), "{slide}");
        assert!(slide.contains("typeface=\"Arial\""), "{slide}");
        assert!(slide.contains("sz=\"1200\""), "{slide}");
        assert!(slide.contains("<a:off x=\"508000\" y=\"355600\"/>"), "{slide}");
        assert!(slide.contains("<a:off x=\"2286000\" y=\"1117600\"/>"), "{slide}");
        assert!(!slide.contains("a:blip"), "{slide}");
        assert!(!slide.contains("<a:tbl"), "{slide}");
    }

    #[test]
    fn slide_size_follows_the_crop_box_and_a_later_page_is_stretched() {
        let stream = b"BT /F1 12 Tf 40 80 Td (Page line.) Tj ET";
        let mut doc = document_with_pages(&[
            PageSpec {
                width: 612,
                height: 792,
                crop: Some((0, 0, 400, 500)),
                stream: stream.to_vec(),
            },
            PageSpec {
                width: 200,
                height: 300,
                crop: None,
                stream: stream.to_vec(),
            },
        ]);
        let bytes = export_document_pptx(&mut doc).expect("pptx");
        let presentation = inflate_part(&bytes, "ppt/presentation.xml");
        assert!(
            presentation.contains("<p:sldSz cx=\"5080000\" cy=\"6350000\"/>"),
            "{presentation}"
        );
        assert!(!presentation.contains("7772400"), "{presentation}");
        assert!(!presentation.contains("10058400"), "{presentation}");
        assert_eq!(presentation.matches("<p:sldId ").count(), 2, "{presentation}");
        let first = inflate_part(&bytes, "ppt/slides/slide1.xml");
        let second = inflate_part(&bytes, "ppt/slides/slide2.xml");
        assert!(first.contains("<a:t>Page line.</a:t>"), "{first}");
        assert!(second.contains("<a:t>Page line.</a:t>"), "{second}");
        assert!(
            second.contains("<a:off x=\"1016000\" y=\"4402667\"/>"),
            "{second}"
        );
    }

    #[test]
    fn a_detected_table_is_text_and_not_a_second_copy_outside_it() {
        let mut doc = document_with_pages(&[PageSpec {
            width: 612,
            height: 792,
            crop: None,
            stream: lattice("Item", "A<B&C", "=SUM", "42"),
        }]);
        let bytes = export_document_pptx(&mut doc).expect("pptx");
        let slide = inflate_part(&bytes, "ppt/slides/slide1.xml");
        assert_eq!(slide.matches("<a:tbl>").count(), 1, "{slide}");
        assert!(slide.contains("<a:t>Item</a:t>"), "{slide}");
        assert!(slide.contains("<a:t>A&lt;B&amp;C</a:t>"), "{slide}");
        assert!(slide.contains("<a:t>=SUM</a:t>"), "{slide}");
        assert!(slide.contains("<a:t>42</a:t>"), "{slide}");
        assert_eq!(slide.matches("Item").count(), 1, "{slide}");
        assert!(!slide.contains("txBox=\"1\""), "{slide}");
        assert!(!slide.contains("a:blip"), "{slide}");
        assert_eq!(xml_text("a\u{1}b\"c"), "ab&quot;c");
    }
}
