//! Appends invisible text, in rendering mode 3, over a scanned image.

use crate::cos::{ObjectId, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfStream, PdfString};
use crate::error::{PdfError, PdfResult};
use crate::images::{get_image_binary, ImageInfo};
use crate::stream::{serialize_ast, ContentAst, ContentNode, ContentParser, Operation};

use super::recognize::{accept_language, recognize_image, OcrReport, OcrWord};
use super::scan::painted_scan_image;

const MAX_PIXELS: u64 = 20_000_000;
const MAX_SIDE: u32 = 8_000;
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_WORDS: usize = 4_000;
const FONT_RESOURCE: &str = "Focr";
/// Courier advances 600 units per em for every glyph.
const COURIER_ADVANCE: f64 = 0.6;

/// Recognizes single-image pages and appends a searchable invisible text layer.
///
/// `language` selects a `tesseract` traineddata name such as `eng`. `None` uses
/// `eng`. The scan's image operators are preserved. Rendering mode 3 does not
/// fill or stroke the new glyphs, so the page looks the same.
pub fn add_searchable_text_layer(
    doc: &mut PdfDocument,
    language: Option<&str>,
) -> PdfResult<OcrReport> {
    let language = match language {
        Some(language) if !language.is_empty() => language,
        _ => "eng",
    };
    accept_language(language)?;

    let page_ids = doc.get_pages()?;
    let mut report = OcrReport {
        pages_seen: 0,
        pages_recognized: 0,
        words_inserted: 0,
    };

    for page_id in page_ids {
        let Some(image) = painted_scan_image(doc, page_id)? else {
            continue;
        };
        report.pages_seen += 1;
        let pixels = u64::from(image.width_px) * u64::from(image.height_px);
        if pixels > MAX_PIXELS || image.width_px > MAX_SIDE || image.height_px > MAX_SIDE {
            return Err(PdfError::SecurityLimitExceeded(
                "The scanned image exceeds the recognition limit.".to_string(),
            ));
        }
        let (binary, _) = get_image_binary(doc, image.object_id)?;
        if binary.len() > MAX_IMAGE_BYTES {
            return Err(PdfError::SecurityLimitExceeded(
                "The scanned image exceeds the recognition limit.".to_string(),
            ));
        }
        let words = recognize_image(&binary, language)?;
        if words.len() > MAX_WORDS {
            return Err(PdfError::SecurityLimitExceeded(
                "The scanned image exceeds the recognition limit.".to_string(),
            ));
        }
        let inserted = inject_words(doc, page_id, &image, &words)?;
        if inserted > 0 {
            report.pages_recognized += 1;
            report.words_inserted += inserted;
        }
    }
    Ok(report)
}

fn inject_words(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    image: &ImageInfo,
    words: &[OcrWord],
) -> PdfResult<usize> {
    let mut operations = Vec::new();
    operations.push(Operation::new("BT", Vec::new()));
    operations.push(Operation::new("Tr", vec![PdfObject::Integer(3)]));
    let mut inserted = 0usize;
    for word in words {
        let Some(mapped) = map_word(image, word) else {
            continue;
        };
        operations.push(Operation::new(
            "Tf",
            vec![
                PdfObject::Name(PdfName::new(FONT_RESOURCE)),
                PdfObject::Real(mapped.font_size),
            ],
        ));
        operations.push(Operation::new(
            "Tz",
            vec![PdfObject::Real(mapped.horizontal_scale)],
        ));
        operations.push(Operation::new(
            "Tm",
            mapped
                .text_matrix
                .into_iter()
                .map(PdfObject::Real)
                .collect(),
        ));
        operations.push(Operation::new(
            "Tj",
            vec![PdfObject::String(PdfString::literal(mapped.bytes))],
        ));
        inserted += 1;
    }
    operations.push(Operation::new("ET", Vec::new()));
    if inserted == 0 {
        return Ok(0);
    }

    let existing = doc.get_page_content_bytes(page_id).unwrap_or_default();
    let mut ast = if existing.is_empty() {
        ContentAst::new()
    } else {
        ContentParser::new(&existing).parse()?
    };
    let id = ast.alloc_id();
    ast.nodes.push(ContentNode::TextBlock { id, operations });
    let serialized = serialize_ast(&ast);

    ensure_ocr_font(doc, page_id)?;
    replace_page_content(doc, page_id, serialized)?;
    Ok(inserted)
}

pub(crate) struct MappedWord {
    pub(crate) bytes: Vec<u8>,
    pub(crate) font_size: f64,
    pub(crate) horizontal_scale: f64,
    pub(crate) text_matrix: [f64; 6],
}

pub(crate) fn map_word(image: &ImageInfo, word: &OcrWord) -> Option<MappedWord> {
    let bytes = encode_winansi(&word.text);
    if bytes.is_empty() || image.width_px == 0 || image.height_px == 0 {
        return None;
    }
    let width_px = f64::from(image.width_px);
    let height_px = f64::from(image.height_px);
    let right = word.left.saturating_add(word.width);
    let bottom = word.top.saturating_add(word.height);
    if word.left < 0
        || word.top < 0
        || right > image.width_px as i32 + 1
        || bottom > image.height_px as i32 + 1
    {
        return None;
    }

    let u = f64::from(word.left) / width_px;
    let du = f64::from(word.width) / width_px;
    let dv = f64::from(word.height) / height_px;
    let v_bottom = 1.0 - f64::from(bottom) / height_px;
    let matrix = &image.ctm;
    let x_vec = (matrix.a * du, matrix.b * du);
    let y_vec = (matrix.c * dv, matrix.d * dv);
    let x_len = x_vec.0.hypot(x_vec.1);
    let y_len = y_vec.0.hypot(y_vec.1);
    if x_len < 0.5 || y_len < 0.5 {
        return None;
    }
    let font_size = y_len;
    let natural = bytes.len() as f64 * COURIER_ADVANCE * font_size;
    if natural <= 0.0 {
        return None;
    }
    let horizontal_scale = (100.0 * x_len / natural).clamp(1.0, 800.0);
    let (origin_x, origin_y) = matrix.transform_point(u, v_bottom);
    Some(MappedWord {
        bytes,
        font_size,
        horizontal_scale,
        text_matrix: [
            x_vec.0 / x_len,
            x_vec.1 / x_len,
            y_vec.0 / y_len,
            y_vec.1 / y_len,
            origin_x,
            origin_y,
        ],
    })
}

fn encode_winansi(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let code = ch as u32;
        if (0x20..=0x7E).contains(&code) {
            bytes.push(code as u8);
        }
    }
    bytes
}

fn winansi_cmap() -> Vec<u8> {
    let mut mappings = String::new();
    let mut count = 0u32;
    for code in 0x20u32..=0x7Eu32 {
        mappings.push_str(&format!("<{code:02X}> <{code:04X}>\n"));
        count += 1;
    }
    let cmap = format!(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n\
         /CMapType 2 def\n\
         1 begincodespacerange\n\
         <00> <FF>\n\
         endcodespacerange\n\
         {count} beginbfchar\n\
         {mappings}\
         endbfchar\n\
         endcmap\n\
         CMapName currentdict /CMap defineresource pop\n\
         end\n\
         end\n"
    );
    cmap.into_bytes()
}

fn ensure_ocr_font(doc: &mut PdfDocument, page_id: ObjectId) -> PdfResult<()> {
    let cmap = winansi_cmap();
    let cmap_id = doc.alloc_object_id();
    let mut cmap_dict = PdfDictionary::new();
    cmap_dict.insert("Length", PdfObject::Integer(cmap.len() as i64));
    doc.set_object(cmap_id, PdfObject::Stream(PdfStream::new(cmap_dict, cmap)));

    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(dictionary) => dictionary,
        _ => {
            return Err(PdfError::TypeMismatch {
                id: page_id.number,
                gen: page_id.generation,
                expected: "Dictionary",
                found: "Non-dictionary page",
            })
        }
    };

    let mut resources = match page_dict.remove("Resources") {
        Some(PdfObject::Dictionary(dictionary)) => dictionary,
        Some(PdfObject::Reference(id)) => match doc.get_object(id)? {
            PdfObject::Dictionary(dictionary) => dictionary,
            _ => PdfDictionary::new(),
        },
        _ => PdfDictionary::new(),
    };
    let mut fonts = match resources.remove("Font") {
        Some(PdfObject::Dictionary(dictionary)) => dictionary,
        Some(PdfObject::Reference(id)) => match doc.get_object(id)? {
            PdfObject::Dictionary(dictionary) => dictionary,
            _ => PdfDictionary::new(),
        },
        _ => PdfDictionary::new(),
    };

    if !fonts.contains_key(FONT_RESOURCE) {
        let mut font = PdfDictionary::new();
        font.insert("Type", PdfObject::Name(PdfName::new("Font")));
        font.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
        font.insert("BaseFont", PdfObject::Name(PdfName::new("Courier")));
        font.insert("Encoding", PdfObject::Name(PdfName::new("WinAnsiEncoding")));
        font.insert("ToUnicode", PdfObject::Reference(cmap_id));
        fonts.insert(FONT_RESOURCE, PdfObject::Dictionary(font));
    }
    resources.insert("Font", PdfObject::Dictionary(fonts));
    page_dict.insert("Resources", PdfObject::Dictionary(resources));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));
    Ok(())
}

fn replace_page_content(doc: &mut PdfDocument, page_id: ObjectId, bytes: Vec<u8>) -> PdfResult<()> {
    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(dictionary) => dictionary,
        _ => return Ok(()),
    };
    let contents_id = doc.alloc_object_id();
    let mut stream_dict = PdfDictionary::new();
    stream_dict.insert("Length", PdfObject::Integer(bytes.len() as i64));
    doc.set_object(
        contents_id,
        PdfObject::Stream(PdfStream::new(stream_dict, bytes)),
    );
    page_dict.insert("Contents", PdfObject::Reference(contents_id));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));
    Ok(())
}
