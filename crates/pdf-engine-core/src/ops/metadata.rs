//! Document metadata extraction, editing, and synchronization (ISO 32000-1 §14.3.3 & §14.4).
//!
//! Provides bidirectional synchronization between the classic document `/Info` dictionary
//! and the Extensible Metadata Platform (XMP) XML packet stored in the catalog `/Metadata` stream.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::cos::filters::decode_stream;
use crate::cos::{PdfDictionary, PdfDocument, PdfName, PdfObject, PdfStream, PdfString};
use crate::error::{PdfError, PdfResult};

/// High-level document metadata container.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocumentMetadata {
    /// Document title (`/Title`, `dc:title`).
    pub title: Option<String>,
    /// Author / primary creator (`/Author`, `dc:creator`).
    pub author: Option<String>,
    /// Subject or description (`/Subject`, `dc:description`).
    pub subject: Option<String>,
    /// Search keywords (`/Keywords`, `pdf:Keywords`).
    pub keywords: Option<String>,
    /// Creating application (`/Creator`, `xmp:CreatorTool`).
    pub creator: Option<String>,
    /// PDF producer (`/Producer`, `pdf:Producer`).
    pub producer: Option<String>,
    /// Creation timestamp (`/CreationDate`, `xmp:CreateDate`).
    pub creation_date: Option<String>,
    /// Modification timestamp (`/ModDate`, `xmp:ModifyDate`).
    pub mod_date: Option<String>,
}

/// Converts a civil UTC seconds timestamp into (iso_8601, pdf_date) strings.
pub fn format_utc_timestamps(seconds: u64) -> (String, String) {
    let (year, month, day, hour, minute, second) = civil_utc(seconds);
    let iso = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z");
    let pdf = format!("D:{year:04}{month:02}{day:02}{hour:02}{minute:02}{second:02}Z");
    (iso, pdf)
}

/// Returns current UTC timestamp in ISO 8601 and PDF date formats.
pub fn current_timestamps() -> (String, String) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_utc_timestamps(seconds)
}

/// Howard Hinnant's algorithm to convert Unix seconds into UTC calendar components.
fn civil_utc(seconds: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = (seconds / 86_400) as i64;
    let rem = (seconds % 86_400) as u32;
    let hour = rem / 3_600;
    let minute = (rem % 3_600) / 60;
    let second = rem % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    (year as i32, month as u32, day as u32, hour, minute, second)
}

/// Converts PDF date format (`D:YYYYMMDDHHmmSS...`) to ISO 8601 (`YYYY-MM-DDTHH:mm:SSZ`).
pub fn pdf_date_to_iso(pdf_date: &str) -> String {
    let raw = pdf_date.strip_prefix("D:").unwrap_or(pdf_date).trim();
    if raw.len() < 4 {
        return pdf_date.to_string();
    }

    let year = &raw[0..4];
    let month = if raw.len() >= 6 { &raw[4..6] } else { "01" };
    let day = if raw.len() >= 8 { &raw[6..8] } else { "01" };
    let hour = if raw.len() >= 10 { &raw[8..10] } else { "00" };
    let minute = if raw.len() >= 12 { &raw[10..12] } else { "00" };
    let second = if raw.len() >= 14 { &raw[12..14] } else { "00" };

    let tz = if raw.len() > 14 {
        let rest = &raw[14..];
        if rest.starts_with('Z') {
            "Z".to_string()
        } else if rest.starts_with('+') || rest.starts_with('-') {
            let sign = &rest[0..1];
            let tz_digits: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            if tz_digits.len() >= 4 {
                format!("{}{}:{}", sign, &tz_digits[0..2], &tz_digits[2..4])
            } else if tz_digits.len() >= 2 {
                format!("{}{}:00", sign, &tz_digits[0..2])
            } else {
                "Z".to_string()
            }
        } else {
            "Z".to_string()
        }
    } else {
        "Z".to_string()
    };

    format!("{year}-{month}-{day}T{hour}:{minute}:{second}{tz}")
}

/// Converts ISO 8601 string or numeric string into PDF date format (`D:YYYYMMDDHHmmSSZ`).
pub fn iso_to_pdf_date(iso_date: &str) -> String {
    let trimmed = iso_date.trim();
    if trimmed.starts_with("D:") {
        return trimmed.to_string();
    }

    // Collect all ASCII digits
    let digits: Vec<char> = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return format!("D:{}", trimmed);
    }

    let year: String = digits.iter().take(4).collect();
    let month: String = if digits.len() >= 6 {
        digits.iter().skip(4).take(2).collect()
    } else {
        "01".to_string()
    };
    let day: String = if digits.len() >= 8 {
        digits.iter().skip(6).take(2).collect()
    } else {
        "01".to_string()
    };
    let hour: String = if digits.len() >= 10 {
        digits.iter().skip(8).take(2).collect()
    } else {
        "00".to_string()
    };
    let min: String = if digits.len() >= 12 {
        digits.iter().skip(10).take(2).collect()
    } else {
        "00".to_string()
    };
    let sec: String = if digits.len() >= 14 {
        digits.iter().skip(12).take(2).collect()
    } else {
        "00".to_string()
    };

    format!("D:{year}{month}{day}{hour}{min}{sec}Z")
}

/// Encodes a UTF-8 string into a `PdfString`, using UTF-16BE with BOM for non-ASCII text.
fn str_to_pdf_string(s: &str) -> PdfString {
    if s.is_ascii() {
        PdfString::literal(s.as_bytes().to_vec())
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in s.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        PdfString::literal(bytes)
    }
}

/// Extracts a string property from a PDF dictionary.
fn dict_string(dict: &PdfDictionary, key: &str) -> Option<String> {
    match dict.get(key) {
        Some(PdfObject::String(s)) => {
            let text = s.to_string_lossy().trim().to_string();
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        _ => None,
    }
}

/// Escapes standard XML characters.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Unescapes standard XML entities.
fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Extracts body content of an XML element `<tag ...>body</tag>`.
fn extract_xml_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open_prefix = format!("<{}", tag);
    let close_tag = format!("</{}>", tag);

    let mut search_from = 0;
    while let Some(open_rel) = xml[search_from..].find(&open_prefix) {
        let open_idx = search_from + open_rel;
        let after_tag = open_idx + open_prefix.len();
        if after_tag < xml.len() {
            let next_byte = xml.as_bytes()[after_tag];
            if next_byte == b'>' || next_byte.is_ascii_whitespace() || next_byte == b'/' {
                if let Some(close_bracket) = xml[after_tag..].find('>') {
                    let body_start = after_tag + close_bracket + 1;
                    if let Some(end_rel) = xml[body_start..].find(&close_tag) {
                        return Some(&xml[body_start..body_start + end_rel]);
                    }
                }
            }
        }
        search_from = after_tag;
    }
    None
}

/// Extracts an XML attribute value `attr="value"`.
fn extract_xml_attr(xml: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr);
    if let Some(idx) = xml.find(&pattern) {
        let val_start = idx + pattern.len();
        if let Some(end_rel) = xml[val_start..].find('"') {
            return Some(xml_unescape(&xml[val_start..val_start + end_rel]));
        }
    }
    None
}

/// Extracts text value from a Dublin Core tag (handles `<rdf:li>` or direct text).
fn extract_dc_value(xml: &str, tag: &str) -> Option<String> {
    if let Some(content) = extract_xml_tag(xml, tag) {
        if let Some(li) = extract_xml_tag(content, "rdf:li") {
            let text = xml_unescape(li.trim());
            if !text.is_empty() {
                return Some(text);
            }
        }
        let text = xml_unescape(content.trim());
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

/// Extracts text value from simple tag or attribute.
fn extract_simple_tag_or_attr(xml: &str, tag: &str) -> Option<String> {
    if let Some(content) = extract_xml_tag(xml, tag) {
        let text = xml_unescape(content.trim());
        if !text.is_empty() {
            return Some(text);
        }
    }
    extract_xml_attr(xml, tag)
}

/// Reads XMP metadata packet from document catalog `/Metadata` stream.
fn read_xmp_metadata(doc: &mut PdfDocument) -> PdfResult<Option<DocumentMetadata>> {
    let Some(catalog_id) = doc.catalog_id() else {
        return Ok(None);
    };

    let meta_ref = match doc.get_object(catalog_id)? {
        PdfObject::Dictionary(cat) => cat.get("Metadata").and_then(|m| m.as_reference()),
        _ => None,
    };

    let Some(meta_id) = meta_ref else {
        return Ok(None);
    };

    let meta_stream = match doc.get_object(meta_id)? {
        PdfObject::Stream(s) => s,
        _ => return Ok(None),
    };

    let limits = doc.limits.clone();
    let bytes = if let Some(filter_obj) = meta_stream.dict.get("Filter") {
        let filter_name = filter_obj.as_name().unwrap_or("");
        decode_stream(filter_name, None, &meta_stream.content, &limits)?
    } else {
        meta_stream.content
    };

    let xml = String::from_utf8_lossy(&bytes);

    let title = extract_dc_value(&xml, "dc:title");
    let author = extract_dc_value(&xml, "dc:creator");
    let subject = extract_dc_value(&xml, "dc:description");
    let keywords = extract_simple_tag_or_attr(&xml, "pdf:Keywords");
    let producer = extract_simple_tag_or_attr(&xml, "pdf:Producer");
    let creator = extract_simple_tag_or_attr(&xml, "xmp:CreatorTool");
    let creation_date = extract_simple_tag_or_attr(&xml, "xmp:CreateDate");
    let mod_date = extract_simple_tag_or_attr(&xml, "xmp:ModifyDate");

    Ok(Some(DocumentMetadata {
        title,
        author,
        subject,
        keywords,
        creator,
        producer,
        creation_date,
        mod_date,
    }))
}

/// Extracts document metadata combining `/Info` and `/Root /Metadata` (XMP).
pub fn extract_metadata(doc: &mut PdfDocument) -> PdfResult<DocumentMetadata> {
    // 1. Read /Info dictionary
    let mut info_meta = DocumentMetadata::default();
    if let Some(info_ref) = doc.xref.trailer.get("Info").and_then(|i| i.as_reference()) {
        if let Ok(PdfObject::Dictionary(info_dict)) = doc.get_object(info_ref) {
            info_meta.title = dict_string(&info_dict, "Title");
            info_meta.author = dict_string(&info_dict, "Author");
            info_meta.subject = dict_string(&info_dict, "Subject");
            info_meta.keywords = dict_string(&info_dict, "Keywords");
            info_meta.creator = dict_string(&info_dict, "Creator");
            info_meta.producer = dict_string(&info_dict, "Producer");
            info_meta.creation_date =
                dict_string(&info_dict, "CreationDate").map(|d| pdf_date_to_iso(&d));
            info_meta.mod_date = dict_string(&info_dict, "ModDate").map(|d| pdf_date_to_iso(&d));
        }
    }

    // 2. Read XMP stream
    let xmp_meta = read_xmp_metadata(doc)?.unwrap_or_default();

    // 3. Bidirectional fallback: prefer /Info, fallback to XMP
    Ok(DocumentMetadata {
        title: info_meta.title.or(xmp_meta.title),
        author: info_meta.author.or(xmp_meta.author),
        subject: info_meta.subject.or(xmp_meta.subject),
        keywords: info_meta.keywords.or(xmp_meta.keywords),
        creator: info_meta.creator.or(xmp_meta.creator),
        producer: info_meta.producer.or(xmp_meta.producer),
        creation_date: info_meta.creation_date.or(xmp_meta.creation_date),
        mod_date: info_meta.mod_date.or(xmp_meta.mod_date),
    })
}

/// Generates a valid synchronized XMP RDF/XML packet.
fn build_xmp_packet(
    meta: &DocumentMetadata,
    creation_iso: &str,
    mod_iso: &str,
    pdfaid: Option<(u8, String)>,
) -> Vec<u8> {
    let mut desc = String::new();

    // Dublin Core schema
    desc.push_str("    <!-- Dublin Core Schema -->\n");
    if let Some(ref title) = meta.title {
        desc.push_str(&format!(
            "    <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\n",
            xml_escape(title)
        ));
    }
    if let Some(ref author) = meta.author {
        desc.push_str(&format!(
            "    <dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>\n",
            xml_escape(author)
        ));
    }
    if let Some(ref subject) = meta.subject {
        desc.push_str(&format!(
            "    <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>\n",
            xml_escape(subject)
        ));
    }

    // Adobe PDF schema
    desc.push_str("    <!-- Adobe PDF Schema -->\n");
    if let Some(ref kw) = meta.keywords {
        desc.push_str(&format!(
            "    <pdf:Keywords>{}</pdf:Keywords>\n",
            xml_escape(kw)
        ));
    }
    if let Some(ref prod) = meta.producer {
        desc.push_str(&format!(
            "    <pdf:Producer>{}</pdf:Producer>\n",
            xml_escape(prod)
        ));
    }

    // XMP Basic schema
    desc.push_str("    <!-- XMP Basic Schema -->\n");
    if let Some(ref creator) = meta.creator {
        desc.push_str(&format!(
            "    <xmp:CreatorTool>{}</xmp:CreatorTool>\n",
            xml_escape(creator)
        ));
    }
    desc.push_str(&format!(
        "    <xmp:CreateDate>{}</xmp:CreateDate>\n",
        xml_escape(creation_iso)
    ));
    desc.push_str(&format!(
        "    <xmp:ModifyDate>{}</xmp:ModifyDate>\n",
        xml_escape(mod_iso)
    ));
    desc.push_str(&format!(
        "    <xmp:MetadataDate>{}</xmp:MetadataDate>\n",
        xml_escape(mod_iso)
    ));

    // Optional PDF/A identification schema preservation
    let mut pdfaid_block = String::new();
    if let Some((part, conf)) = pdfaid {
        pdfaid_block.push_str(&format!(
            "  <rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\">\n\
             \x20   <pdfaid:part>{}</pdfaid:part>\n\
             \x20   <pdfaid:conformance>{}</pdfaid:conformance>\n\
             \x20 </rdf:Description>\n",
            part,
            xml_escape(&conf)
        ));
    }

    format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
         <rdf:Description rdf:about=\"\"\n\
         \x20   xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n\
         \x20   xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"\n\
         \x20   xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n\
         {}\
         \x20 </rdf:Description>\n\
         {}\
         </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>\n",
        desc, pdfaid_block
    )
    .into_bytes()
}

/// Updates document metadata and synchronizes `/Info` and `/Root /Metadata` (XMP).
pub fn update_metadata(doc: &mut PdfDocument, meta: &DocumentMetadata) -> PdfResult<()> {
    let (now_iso, _now_pdf) = current_timestamps();

    // 1. Resolve timestamps
    let creation_iso = meta
        .creation_date
        .as_deref()
        .map(pdf_date_to_iso)
        .unwrap_or_else(|| now_iso.clone());
    let creation_pdf = iso_to_pdf_date(&creation_iso);

    let mod_iso = meta
        .mod_date
        .as_deref()
        .map(pdf_date_to_iso)
        .unwrap_or_else(|| now_iso.clone());
    let mod_pdf = iso_to_pdf_date(&mod_iso);

    // 2. Update /Info dictionary
    let info_id = match doc.xref.trailer.get("Info").and_then(|i| i.as_reference()) {
        Some(id) => id,
        None => {
            let id = doc.alloc_object_id();
            doc.xref.trailer.insert("Info", id);
            id
        }
    };

    let mut info_dict = match doc.get_object(info_id) {
        Ok(PdfObject::Dictionary(d)) => d,
        _ => PdfDictionary::new(),
    };

    let set_field = |dict: &mut PdfDictionary, key: &str, val: Option<&String>| {
        if let Some(v) = val {
            if !v.trim().is_empty() {
                dict.insert(key, str_to_pdf_string(v));
            } else {
                dict.remove(key);
            }
        } else {
            dict.remove(key);
        }
    };

    set_field(&mut info_dict, "Title", meta.title.as_ref());
    set_field(&mut info_dict, "Author", meta.author.as_ref());
    set_field(&mut info_dict, "Subject", meta.subject.as_ref());
    set_field(&mut info_dict, "Keywords", meta.keywords.as_ref());
    set_field(&mut info_dict, "Creator", meta.creator.as_ref());
    set_field(&mut info_dict, "Producer", meta.producer.as_ref());
    info_dict.insert(
        "CreationDate",
        PdfString::literal(creation_pdf.into_bytes()),
    );
    info_dict.insert("ModDate", PdfString::literal(mod_pdf.into_bytes()));

    doc.set_object(info_id, PdfObject::Dictionary(info_dict));

    // 3. Update /Root /Metadata stream (XMP)
    let catalog_id = doc.catalog_id().ok_or_else(|| {
        PdfError::OperationError(
            "Cannot synchronize metadata: Missing document Catalog".to_string(),
        )
    })?;

    let mut cat_dict = match doc.get_object(catalog_id)? {
        PdfObject::Dictionary(d) => d,
        _ => {
            return Err(PdfError::OperationError(
                "Catalog is not a dictionary".to_string(),
            ))
        }
    };

    // Inspect existing XMP to preserve PDF/A conformance tags
    let mut pdfaid: Option<(u8, String)> = None;
    if let Some(existing_meta_id) = cat_dict.get("Metadata").and_then(|m| m.as_reference()) {
        if let Ok(PdfObject::Stream(s)) = doc.get_object(existing_meta_id) {
            let raw_xml = String::from_utf8_lossy(&s.content);
            if let Some(part_str) = extract_xml_tag(&raw_xml, "pdfaid:part") {
                if let Ok(part) = part_str.trim().parse::<u8>() {
                    let conf = extract_xml_tag(&raw_xml, "pdfaid:conformance")
                        .unwrap_or("B")
                        .trim()
                        .to_string();
                    pdfaid = Some((part, conf));
                }
            }
        }
    }

    let packet = build_xmp_packet(meta, &creation_iso, &mod_iso, pdfaid);

    let mut meta_stream_dict = PdfDictionary::new();
    meta_stream_dict.insert("Type", PdfName::new("Metadata"));
    meta_stream_dict.insert("Subtype", PdfName::new("XML"));
    meta_stream_dict.insert("Length", packet.len() as i64);

    let meta_id = match cat_dict.get("Metadata").and_then(|m| m.as_reference()) {
        Some(id) => id,
        None => {
            let id = doc.alloc_object_id();
            cat_dict.insert("Metadata", id);
            doc.set_object(catalog_id, PdfObject::Dictionary(cat_dict));
            id
        }
    };

    doc.set_object(
        meta_id,
        PdfObject::Stream(PdfStream::new(meta_stream_dict, packet)),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_date_conversion_roundtrip() {
        let pdf = "D:20261008123045Z";
        let iso = pdf_date_to_iso(pdf);
        assert_eq!(iso, "2026-10-08T12:30:45Z");
        let back_pdf = iso_to_pdf_date(&iso);
        assert_eq!(back_pdf, "D:20261008123045Z");

        let pdf_offset = "D:20261008123045+02'00'";
        let iso_offset = pdf_date_to_iso(pdf_offset);
        assert_eq!(iso_offset, "2026-10-08T12:30:45+02:00");
    }

    #[test]
    fn test_empty_doc_metadata_and_update() {
        let mut doc = PdfDocument::empty();
        let empty_meta = extract_metadata(&mut doc).unwrap();
        assert_eq!(empty_meta.title, None);
        assert_eq!(empty_meta.author, None);

        let update = DocumentMetadata {
            title: Some("Declaración Anual 2026".to_string()),
            author: Some("SAT México".to_string()),
            subject: Some("Impuestos y Finanzas".to_string()),
            keywords: Some("fiscal, hacienda, sat".to_string()),
            creator: Some("PDFEngine Web Studio".to_string()),
            producer: Some("PDFEngine Core 1.0".to_string()),
            creation_date: Some("2026-01-01T00:00:00Z".to_string()),
            mod_date: Some("2026-10-08T12:00:00Z".to_string()),
        };

        update_metadata(&mut doc, &update).unwrap();

        let extracted = extract_metadata(&mut doc).unwrap();
        assert_eq!(extracted.title.as_deref(), Some("Declaración Anual 2026"));
        assert_eq!(extracted.author.as_deref(), Some("SAT México"));
        assert_eq!(extracted.subject.as_deref(), Some("Impuestos y Finanzas"));
        assert_eq!(extracted.keywords.as_deref(), Some("fiscal, hacienda, sat"));
        assert_eq!(extracted.creator.as_deref(), Some("PDFEngine Web Studio"));
        assert_eq!(extracted.producer.as_deref(), Some("PDFEngine Core 1.0"));
        assert_eq!(
            extracted.creation_date.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        assert_eq!(extracted.mod_date.as_deref(), Some("2026-10-08T12:00:00Z"));

        // Verify XMP stream was written to Catalog
        let catalog_id = doc.catalog_id().unwrap();
        let cat_dict = doc
            .get_object(catalog_id)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        let meta_id = cat_dict.get("Metadata").unwrap().as_reference().unwrap();
        let stream = match doc.get_object(meta_id).unwrap() {
            PdfObject::Stream(s) => s,
            _ => panic!("Expected stream"),
        };
        let xml = String::from_utf8_lossy(&stream.content);
        assert!(xml.contains("Declaración Anual 2026"));
        assert!(xml.contains("SAT México"));
        assert!(xml.contains("PDFEngine Web Studio"));
    }

    #[test]
    fn test_pdfa_conformance_preservation() {
        let mut doc = PdfDocument::empty();
        // Manually install a PDF/A-1b packet
        let catalog_id = doc.catalog_id().unwrap();
        let mut cat_dict = doc
            .get_object(catalog_id)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        let init_xmp = b"<?xpacket begin=\"\xef\xbb\xbf\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
            <x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
            <rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\">\n\
            <pdfaid:part>1</pdfaid:part><pdfaid:conformance>B</pdfaid:conformance>\n\
            </rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>\n";

        let mut stream_dict = PdfDictionary::new();
        stream_dict.insert("Type", PdfName::new("Metadata"));
        stream_dict.insert("Subtype", PdfName::new("XML"));
        let meta_id = doc.alloc_object_id();
        doc.set_object(
            meta_id,
            PdfObject::Stream(PdfStream::new(stream_dict, init_xmp.to_vec())),
        );
        cat_dict.insert("Metadata", meta_id);
        doc.set_object(catalog_id, PdfObject::Dictionary(cat_dict));

        // Update metadata
        let meta = DocumentMetadata {
            title: Some("Archived Title".to_string()),
            ..Default::default()
        };
        update_metadata(&mut doc, &meta).unwrap();

        let stream = match doc.get_object(meta_id).unwrap() {
            PdfObject::Stream(s) => s,
            _ => panic!("Expected stream"),
        };
        let xml = String::from_utf8_lossy(&stream.content);
        assert!(xml.contains("<pdfaid:part>1</pdfaid:part>"));
        assert!(xml.contains("<pdfaid:conformance>B</pdfaid:conformance>"));
        assert!(xml.contains("Archived Title"));
    }

    #[test]
    fn test_non_ascii_unicode_encoding() {
        let mut doc = PdfDocument::empty();
        let meta = DocumentMetadata {
            title: Some("Contrato de Español (Año 2026) 🚀".to_string()),
            author: Some("José García & Peña".to_string()),
            ..Default::default()
        };
        update_metadata(&mut doc, &meta).unwrap();

        let extracted = extract_metadata(&mut doc).unwrap();
        assert_eq!(
            extracted.title.as_deref(),
            Some("Contrato de Español (Año 2026) 🚀")
        );
        assert_eq!(extracted.author.as_deref(), Some("José García & Peña"));
    }
}
