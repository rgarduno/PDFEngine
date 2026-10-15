//! Workbook export of detected tables to Office Open XML (`.xlsx`).
//!
//! One worksheet per table the existing detector returns. Every cell is the
//! reconstructed text, written as an inline string so a value is not a formula
//! and is not coerced to a number. This is not a binary `.xls` file.

use std::collections::BTreeSet;
use std::io::Write;

use flate2::write::DeflateEncoder;
use flate2::Compression;

use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::tables::{detect_tables, DetectedTable};

/// Excel's worksheet grid.
const EXCEL_MAX_ROWS: usize = 1_048_576;
const EXCEL_MAX_COLS: usize = 16_384;
/// Excel's limit for the text stored in one cell.
const EXCEL_MAX_CELL_CHARS: usize = 32_767;
const SHEET_NAME_LIMIT: usize = 31;

/// Writes the document's detected tables into one XLSX workbook.
///
/// Pages are walked from 1. A document with no detected table still emits one
/// empty worksheet named `Tables`, so the package opens. A span is not an Excel
/// merge. A page the detector refuses fails this export.
pub fn export_document_xlsx(doc: &mut PdfDocument) -> PdfResult<Vec<u8>> {
    doc.require_copy()?;
    let page_count = doc.get_pages()?.len();
    let mut used = BTreeSet::new();
    let mut sheets = Vec::new();

    for page_idx in 1..=page_count {
        for table in detect_tables(doc, page_idx)? {
            let name = sheet_name(table.page_number, table.table_idx, &mut used);
            sheets.push((name, table_grid(&table)));
        }
    }

    if sheets.is_empty() {
        sheets.push(("Tables".to_string(), Vec::new()));
    }

    write_package(&sheets)
}

fn sheet_name(page: usize, table_idx: usize, used: &mut BTreeSet<String>) -> String {
    unique_sheet_name(&format!("P{page}T{}", table_idx + 1), used)
}

fn unique_sheet_name(raw: &str, used: &mut BTreeSet<String>) -> String {
    let base = clean_sheet_name(raw);
    if insert_sheet_name(used, base.clone()) {
        return base;
    }
    for n in 2..10_000 {
        let suffix = format!("-{n}");
        let stem = truncate_chars(&base, SHEET_NAME_LIMIT.saturating_sub(suffix.len()));
        let candidate = format!("{stem}{suffix}");
        if insert_sheet_name(used, candidate.clone()) {
            return candidate;
        }
    }
    let fallback = truncate_chars(&format!("T{}", used.len() + 1), SHEET_NAME_LIMIT);
    insert_sheet_name(used, fallback.clone());
    fallback
}

fn insert_sheet_name(used: &mut BTreeSet<String>, name: String) -> bool {
    if used.iter().any(|existing| existing.eq_ignore_ascii_case(&name)) {
        return false;
    }
    used.insert(name);
    true
}

fn clean_sheet_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !matches!(c, ':' | '\\' | '/' | '?' | '*' | '[' | ']'))
        .collect();
    let trimmed = cleaned.trim().trim_matches('\'');
    if trimmed.is_empty() {
        "Table".to_string()
    } else {
        truncate_chars(trimmed, SHEET_NAME_LIMIT)
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn table_grid(table: &DetectedTable) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    if !table.headers.is_empty() {
        rows.push(truncate_cols(&table.headers));
    }
    let room = EXCEL_MAX_ROWS.saturating_sub(rows.len());
    for row in table.rows.iter().take(room) {
        rows.push(truncate_cols(row));
    }
    rows
}

fn truncate_cols(row: &[String]) -> Vec<String> {
    row.iter()
        .take(EXCEL_MAX_COLS)
        .map(|cell| truncate_chars(cell, EXCEL_MAX_CELL_CHARS))
        .collect()
}

fn worksheet_xml(rows: &[Vec<String>]) -> String {
    let mut body = String::new();
    let mut max_col = 0usize;
    for (row_idx, row) in rows.iter().enumerate() {
        if row.is_empty() {
            continue;
        }
        max_col = max_col.max(row.len());
        let row_number = row_idx + 1;
        body.push_str(&format!("<row r=\"{row_number}\">"));
        for (col_idx, value) in row.iter().enumerate() {
            let reference = format!("{}{row_number}", column_name(col_idx));
            body.push_str(&inline_cell(&reference, value));
        }
        body.push_str("</row>");
    }

    let dimension = if rows.is_empty() || max_col == 0 {
        String::new()
    } else {
        format!(
            "<dimension ref=\"A1:{}{}\"/>",
            column_name(max_col - 1),
            rows.len()
        )
    };
    let sheet_data = if body.is_empty() {
        "<sheetData/>".to_string()
    } else {
        format!("<sheetData>{body}</sheetData>")
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
        {dimension}{sheet_data}</worksheet>"
    )
}

fn inline_cell(reference: &str, value: &str) -> String {
    let space = if value.starts_with([' ', '\t'])
        || value.ends_with([' ', '\t'])
        || value.contains(['\n', '\r', '\t'])
    {
        " xml:space=\"preserve\""
    } else {
        ""
    };
    format!(
        "<c r=\"{reference}\" t=\"inlineStr\"><is><t{space}>{}</t></is></c>",
        xml_text(value)
    )
}

fn column_name(index: usize) -> String {
    let mut n = index + 1;
    let mut chars = Vec::new();
    while n > 0 {
        n -= 1;
        chars.push(char::from(b'A' + (n % 26) as u8));
        n /= 26;
    }
    chars.iter().rev().collect()
}

/// Escapes text for XML and drops characters XML 1.0 forbids.
fn xml_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\u{9}' | '\u{A}' | '\u{D}' => out.push(ch),
            c if c.is_control() => {}
            '\u{FFFE}' | '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

fn write_package(sheets: &[(String, Vec<Vec<String>>)]) -> PdfResult<Vec<u8>> {
    let names: Vec<String> = sheets.iter().map(|(name, _)| name.clone()).collect();
    let mut zip = PkZipWriter::new();
    zip.add_file("[Content_Types].xml", content_types(sheets.len()).as_bytes())?;
    zip.add_file("_rels/.rels", ROOT_RELS.as_bytes())?;
    zip.add_file("xl/workbook.xml", workbook_xml(&names).as_bytes())?;
    zip.add_file(
        "xl/_rels/workbook.xml.rels",
        workbook_rels(sheets.len()).as_bytes(),
    )?;
    zip.add_file("xl/styles.xml", STYLES.as_bytes())?;
    for (index, (_, rows)) in sheets.iter().enumerate() {
        let path = format!("xl/worksheets/sheet{}.xml", index + 1);
        zip.add_file(&path, worksheet_xml(rows).as_bytes())?;
    }
    Ok(zip.finish())
}

fn content_types(sheet_count: usize) -> String {
    let mut overrides = String::from(
        "<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
        <Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>",
    );
    for index in 1..=sheet_count {
        overrides.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{index}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
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

fn workbook_xml(names: &[String]) -> String {
    let mut sheets = String::new();
    for (index, name) in names.iter().enumerate() {
        let id = index + 1;
        sheets.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{id}\" r:id=\"rId{id}\"/>",
            xml_text(name)
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
        xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
        <sheets>{sheets}</sheets></workbook>"
    )
}

fn workbook_rels(sheet_count: usize) -> String {
    let mut rels = String::new();
    for index in 1..=sheet_count {
        rels.push_str(&format!(
            "<Relationship Id=\"rId{index}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{index}.xml\"/>"
        ));
    }
    let style_id = sheet_count + 1;
    rels.push_str(&format!(
        "<Relationship Id=\"rId{style_id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>"
    ));
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels}</Relationships>"
    )
}

const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
</Relationships>";

const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
<fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
<fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills>\
<borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
<cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs>\
<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
</styleSheet>";

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
    use crate::layout::Rect;

    struct PageSpec {
        width: i64,
        height: i64,
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
        panic!("part {name} was not in the workbook");
    }

    #[test]
    fn a_document_without_tables_still_opens_as_a_workbook() {
        let mut doc = PdfDocument::empty();
        let bytes = export_document_xlsx(&mut doc).expect("empty document exports");
        assert!(bytes.windows(4).any(|w| w == b"PK\x05\x06"));
        let workbook = inflate_part(&bytes, "xl/workbook.xml");
        assert!(workbook.contains("name=\"Tables\""));
        assert_eq!(workbook.matches("<sheet ").count(), 1);
        let sheet = inflate_part(&bytes, "xl/worksheets/sheet1.xml");
        assert!(sheet.contains("<sheetData/>"));
        assert!(!sheet.contains("inlineStr"));
        assert!(inflate_part(&bytes, "[Content_Types].xml").contains("sheet.main+xml"));
        assert!(inflate_part(&bytes, "xl/styles.xml").contains("<styleSheet"));

        let mut page = document_with_pages(&[PageSpec {
            width: 612,
            height: 792,
            stream: Vec::new(),
        }]);
        let bytes = export_document_xlsx(&mut page).expect("blank page exports");
        let workbook = inflate_part(&bytes, "xl/workbook.xml");
        assert!(workbook.contains("name=\"Tables\""));
        assert!(!inflate_part(&bytes, "xl/worksheets/sheet1.xml").contains("inlineStr"));
    }

    #[test]
    fn each_detected_table_is_a_worksheet_of_text() {
        let mut doc = document_with_pages(&[
            PageSpec {
                width: 612,
                height: 792,
                stream: lattice("Item", "A<B&C", "=SUM", "42"),
            },
            PageSpec {
                width: 612,
                height: 792,
                stream: lattice("Alpha", "Beta", "Gamma", "Delta"),
            },
        ]);

        let bytes = export_document_xlsx(&mut doc).expect("tables export");
        let workbook = inflate_part(&bytes, "xl/workbook.xml");
        assert!(workbook.contains("name=\"P1T1\""));
        assert!(workbook.contains("name=\"P2T1\""));
        assert_eq!(workbook.matches("<sheet ").count(), 2);
        assert!(!workbook.contains("name=\"Tables\""));

        let sheet = inflate_part(&bytes, "xl/worksheets/sheet1.xml");
        assert!(sheet.contains("t=\"inlineStr\""));
        assert!(sheet.contains("<t>Item</t>"));
        assert!(sheet.contains("<t>A&lt;B&amp;C</t>"));
        assert!(sheet.contains("<t>=SUM</t>"));
        assert!(sheet.contains("<t>42</t>"));
        assert!(!sheet.contains("<f"));
        assert!(!sheet.contains("<v>"));
        assert!(sheet.contains("r=\"A1\""));
        assert!(sheet.contains("r=\"B2\""));

        let other = inflate_part(&bytes, "xl/worksheets/sheet2.xml");
        assert!(other.contains("<t>Alpha</t>"));
        assert!(other.contains("<t>Delta</t>"));
        assert!(!other.contains("Item"));
    }

    #[test]
    fn a_sheet_name_stays_unique_and_columns_pass_z() {
        let mut used = BTreeSet::new();
        assert_eq!(unique_sheet_name("P1T1", &mut used), "P1T1");
        assert_eq!(unique_sheet_name("p1t1", &mut used), "p1t1-2");
        assert_eq!(unique_sheet_name("Bad:Name/?", &mut used), "BadName");
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(16_383), "XFD");
        assert_eq!(xml_text("a\u{1}b\"c"), "ab&quot;c");
        assert_eq!(truncate_chars(&"a".repeat(32_768), EXCEL_MAX_CELL_CHARS).chars().count(), 32_767);
    }

    #[test]
    fn a_grid_stops_at_excels_column_limit() {
        let table = DetectedTable {
            table_idx: 0,
            page_number: 1,
            bbox: Rect::new(0.0, 0.0, 10.0, 10.0),
            row_count: 1,
            col_count: EXCEL_MAX_COLS + 1,
            cells: Vec::new(),
            headers: vec!["x".to_string(); EXCEL_MAX_COLS + 1],
            rows: vec![vec!["y".to_string(); 3]],
        };
        let grid = table_grid(&table);
        assert_eq!(grid.len(), 2);
        assert_eq!(grid[0].len(), EXCEL_MAX_COLS);
        assert_eq!(grid[1].len(), 3);
    }
}
