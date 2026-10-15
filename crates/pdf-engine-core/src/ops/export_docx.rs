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

/// Exports the entire PDF document into a native Microsoft Word OpenXML (`.docx`) binary buffer.
///
/// Generates a self-contained PKZIP archive conforming to ECMA-376 / ISO/IEC 29500
/// WordprocessingML with styled paragraphs, run-level font families/sizes/weights,
/// and bordered grid tables.
pub fn export_document_docx(doc: &mut PdfDocument) -> PdfResult<Vec<u8>> {
    doc.require_copy()?;
    let metadata = extract_metadata(doc).unwrap_or_default();
    let pages = doc.get_pages()?;

    let mut body_xml = String::new();

    for page_idx in 1..=pages.len() {
        if page_idx > 1 {
            body_xml.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }

        let (elements, fonts) = extract_page_elements(doc, page_idx)?;
        for element in elements {
            match element {
                PageElement::Paragraph(para) => {
                    append_paragraph_xml(&mut body_xml, &para, &fonts);
                }
                PageElement::Table(table) => {
                    append_table_xml(&mut body_xml, &table);
                }
            }
        }
    }

    // Section properties (US Letter 8.5 x 11 in = 12240 x 15840 twips, 1 inch = 1440 twips margins)
    body_xml.push_str(
        "<w:sectPr>\
            <w:pgSz w:w=\"12240\" w:h=\"15840\"/>\
            <w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/>\
        </w:sectPr>",
    );

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

fn append_paragraph_xml(
    body_xml: &mut String,
    para: &ParagraphBlock,
    fonts: &BTreeMap<String, ResolvedFont>,
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

    body_xml.push_str("<w:p><w:pPr>");
    if let Some(st) = style_id {
        body_xml.push_str(&format!("<w:pStyle w:val=\"{st}\"/>"));
    }
    body_xml.push_str(&format!(
        "<w:spacing w:after=\"120\" w:line=\"240\" w:lineRule=\"auto\"/><w:jc w:val=\"{jc}\"/></w:pPr>"
    ));

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

fn append_table_xml(body_xml: &mut String, table: &DetectedTable) {
    let col_count = table.col_count.max(1);
    let col_width_twips = 9360 / col_count;

    body_xml.push_str(
        "<w:tbl>\
            <w:tblPr>\
                <w:tblStyle w:val=\"TableGrid\"/>\
                <w:tblW w:w=\"5000\" w:type=\"pct\"/>\
                <w:tblBorders>\
                    <w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
                    <w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
                    <w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
                    <w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
                    <w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
                    <w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
                </w:tblBorders>\
            </w:tblPr>",
    );

    body_xml.push_str("<w:tblGrid>");
    for _ in 0..col_count {
        body_xml.push_str(&format!("<w:gridCol w:w=\"{col_width_twips}\"/>"));
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
            body_xml.push_str(&format!(
                "<w:tc>\
                    <w:tcPr>\
                        <w:tcW w:w=\"{col_width_twips}\" w:type=\"dxa\"/>\
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
            body_xml.push_str(&format!(
                "<w:tc>\
                    <w:tcPr>\
                        <w:tcW w:w=\"{col_width_twips}\" w:type=\"dxa\"/>\
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
}
