//! SHA-256 byte-range integrity attestation.
//!
//! `/Filter` is `/PDFEngine.Approval` and `/SubFilter` is `/PDFEngine.sha256`.
//! `/Contents` is the raw SHA-256 digest of the file bytes selected by `/ByteRange`.
//! The two ranges cover the saved file exactly once and skip only the hex digits
//! of `/Contents`. This records file integrity. It is not a CMS signature and it
//! does not establish the signer's identity.
//!
//! PKCS#7 detached signatures are built by `cms` and use `/SubFilter /adbe.pkcs7.detached`.
//! Saving a PKCS#7 file rewrites offsets and does not reseal that signature.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cos::{
    ObjectId, Parser, PdfArray, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfStream,
    PdfString, XRefEntry,
};
use crate::crypto::sha256;
use crate::error::{PdfError, PdfResult};

const SUBFILTER_NAME: &str = "PDFEngine.sha256";
const FILTER_NAME: &str = "PDFEngine.Approval";
/// 32-byte hole. Its hex encoding is 64 characters, the same width as a SHA-256 digest.
const CONTENTS_HOLE: &[u8] = b"PDFEngine-sha256-contents-hole!!";
const SUBFILTER_MARK: &[u8] = b"/SubFilter /PDFEngine.sha256";
const TYPE_SIG_MARK: &[u8] = b"/Type /Sig";

/// Parameters for stamping an integrity attestation onto one page.
#[derive(Debug, Clone)]
pub struct DigitalSignatureConfig {
    /// Common name or organization stored in `/Name`.
    pub signer_name: String,
    /// Operational note stored in `/Reason`.
    pub reason: String,
    /// Location stored in `/Location`.
    pub location: String,
    /// Visual stamp rectangle `[min_x, min_y, max_x, max_y]` in points.
    pub rect: [f64; 4],
    /// Target page number (1-based).
    pub page_number: usize,
    /// Draw the visual stamp when true.
    pub visual_badge: bool,
    /// Optional `/ContactInfo` value.
    pub contact_info: Option<String>,
}

impl Default for DigitalSignatureConfig {
    fn default() -> Self {
        Self {
            signer_name: "PDFEngine Certified Signer".to_string(),
            reason: "Integridad del archivo".to_string(),
            location: "Mexico City, MX".to_string(),
            rect: [72.0, 72.0, 272.0, 142.0],
            page_number: 1,
            visual_badge: true,
            contact_info: None,
        }
    }
}

/// One signature dictionary read back from a document.
#[derive(Debug, Clone)]
pub struct VerifiedSignature {
    /// Widget field name (`/T`).
    pub field_name: String,
    /// `/Name` value.
    pub signer_name: String,
    /// `/Reason` value.
    pub reason: String,
    /// `/Location` value.
    pub location: String,
    /// `/M` value as stored in the file.
    pub date: String,
    /// `/SubFilter` name. Attestations use `PDFEngine.sha256`. PKCS#7 uses `adbe.pkcs7.detached`.
    pub sub_filter: String,
    /// `/ByteRange` as four non-negative integers.
    pub byte_range: Vec<usize>,
    /// Lowercase hex of `/Contents`, including zero padding.
    /// An attestation stores the SHA-256 digest. PKCS#7 stores the CMS encoding.
    pub contents_hex: String,
    /// True when `/ByteRange` covers the file except the Contents hex and the
    /// attestation digest or the PKCS#7 signature checks against the certificate inside it.
    pub byte_range_valid: bool,
    /// Widget rectangle `[min_x, min_y, max_x, max_y]`.
    pub rect: [f64; 4],
    /// Page number recorded for the widget. Defaults to 1 when the page is not resolved.
    pub page_number: usize,
}

/// Draws the visual stamp. The stamp does not include the digest: the digest
/// covers this stream, so embedding it would change the bytes being hashed.
fn synthesize_signature_appearance(
    config: &DigitalSignatureConfig,
    date_str: &str,
    badge_line: &str,
) -> PdfStream {
    let width = (config.rect[2] - config.rect[0]).max(10.0);
    let height = (config.rect[3] - config.rect[1]).max(10.0);

    let mut dict = PdfDictionary::new();
    dict.insert("Type", PdfObject::Name(PdfName::new("XObject")));
    dict.insert("Subtype", PdfObject::Name(PdfName::new("Form")));

    let mut bbox = PdfArray::new();
    bbox.push(PdfObject::Real(0.0));
    bbox.push(PdfObject::Real(0.0));
    bbox.push(PdfObject::Real(width));
    bbox.push(PdfObject::Real(height));
    dict.insert("BBox", PdfObject::Array(bbox));

    let mut font_dict = PdfDictionary::new();
    let mut f_helv = PdfDictionary::new();
    f_helv.insert("Type", PdfObject::Name(PdfName::new("Font")));
    f_helv.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
    f_helv.insert("BaseFont", PdfObject::Name(PdfName::new("Helvetica")));
    font_dict.insert("Helv", PdfObject::Dictionary(f_helv));

    let mut f_bold = PdfDictionary::new();
    f_bold.insert("Type", PdfObject::Name(PdfName::new("Font")));
    f_bold.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
    f_bold.insert("BaseFont", PdfObject::Name(PdfName::new("Helvetica-Bold")));
    font_dict.insert("HelvB", PdfObject::Dictionary(f_bold));

    let mut res_dict = PdfDictionary::new();
    res_dict.insert("Font", PdfObject::Dictionary(font_dict));
    dict.insert("Resources", PdfObject::Dictionary(res_dict));

    let mut ops = String::new();
    ops.push_str("q\n");
    ops.push_str("0.96 0.98 1.0 rg\n");
    ops.push_str(&format!(
        "0.5 0.5 {:.2} {:.2} re f\n",
        width - 1.0,
        height - 1.0
    ));
    ops.push_str("0.15 0.35 0.70 RG 1.5 w\n");
    ops.push_str(&format!(
        "1.0 1.0 {:.2} {:.2} re S\n",
        width - 2.0,
        height - 2.0
    ));
    ops.push_str("0.40 0.60 0.90 RG 0.5 w\n");
    ops.push_str(&format!(
        "3.5 3.5 {:.2} {:.2} re S\n",
        width - 7.0,
        height - 7.0
    ));

    let badge_cx = 24.0;
    let badge_cy = height / 2.0;
    ops.push_str("0.15 0.45 0.85 rg\n");
    ops.push_str(&format!(
        "{:.2} {:.2} 16 16 re f\n",
        badge_cx - 8.0,
        badge_cy - 8.0
    ));
    ops.push_str("1.0 1.0 1.0 RG 2.0 w\n");
    ops.push_str(&format!(
        "{:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l S\n",
        badge_cx - 4.0,
        badge_cy,
        badge_cx - 1.0,
        badge_cy - 4.0,
        badge_cx + 5.0,
        badge_cy + 4.0
    ));

    ops.push_str("0.1 0.1 0.15 rg\n");
    let text_x = 44.0;
    let line1_y = (height - 18.0).max(12.0);
    let line2_y = (line1_y - 12.0).max(10.0);
    let line3_y = (line2_y - 11.0).max(8.0);
    let line4_y = (line3_y - 10.0).max(6.0);

    ops.push_str("BT\n/HelvB 9 Tf\n");
    ops.push_str(&format!("{:.2} {:.2} Td\n", text_x, line1_y));
    ops.push_str(&format!(
        "(Integridad: {}) Tj\n",
        escape_pdf_str(&config.signer_name)
    ));
    ops.push_str("ET\n");

    ops.push_str("BT\n/Helv 7.5 Tf\n");
    ops.push_str(&format!("{:.2} {:.2} Td\n", text_x, line2_y));
    ops.push_str(&format!(
        "(Motivo: {}) Tj\n",
        escape_pdf_str(&config.reason)
    ));
    ops.push_str("ET\n");

    ops.push_str("BT\n/Helv 7 Tf\n");
    ops.push_str(&format!("{:.2} {:.2} Td\n", text_x, line3_y));
    ops.push_str(&format!(
        "(Lugar: {} | Fecha: {}) Tj\n",
        escape_pdf_str(&config.location),
        date_str
    ));
    ops.push_str("ET\n");

    ops.push_str("0.4 0.45 0.55 rg\n");
    ops.push_str("BT\n/Helv 6 Tf\n");
    ops.push_str(&format!("{:.2} {:.2} Td\n", text_x, line4_y));
    ops.push_str(&format!("({}) Tj\n", escape_pdf_str(badge_line)));
    ops.push_str("ET\n");
    ops.push_str("Q\n");

    let content = ops.into_bytes();
    dict.insert("Length", PdfObject::Integer(content.len() as i64));
    PdfStream::new(dict, content)
}

fn escape_pdf_str(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

pub(crate) fn padded_byte_range(a: u64, b: u64, c: u64, d: u64) -> String {
    format!("[ {:010} {:010} {:010} {:010} ]", a, b, c, d)
}

/// Stamps an integrity attestation and reloads `doc` from the sealed bytes.
///
/// The returned value is read back from those bytes, so `byte_range_valid`
/// reflects the digest of the buffer now stored in the document.
pub fn sign_document(
    doc: &mut PdfDocument,
    config: &DigitalSignatureConfig,
) -> PdfResult<VerifiedSignature> {
    let field_name = stamp_signature(
        doc,
        config,
        FILTER_NAME,
        SUBFILTER_NAME,
        CONTENTS_HOLE,
        "SHA-256 sobre el rango de bytes",
    )?;
    let limits = doc.limits.clone();
    let sealed = doc.save_to_vec()?;
    *doc = PdfDocument::load_with_limits(&sealed, limits)?;
    verify_document_signatures(doc)
        .into_iter()
        .rev()
        .find(|sig| sig.field_name == field_name)
        .ok_or_else(|| {
            PdfError::CryptographyError(
                "The integrity attestation was written but could not be read back.".to_string(),
            )
        })
}

/// Inserts a signature dictionary and widget. Does not save the document.
pub(crate) fn stamp_signature(
    doc: &mut PdfDocument,
    config: &DigitalSignatureConfig,
    filter: &str,
    subfilter: &str,
    contents_hole: &[u8],
    badge_line: &str,
) -> PdfResult<String> {
    let pages = doc.get_pages()?;
    let pages_count = pages.len();
    if config.page_number == 0 || config.page_number > pages_count {
        return Err(PdfError::InvalidPageNumber {
            page: config.page_number,
            total: pages_count,
        });
    }

    let (iso_date, pdf_date) = utc_timestamps();

    let mut byte_range_arr = PdfArray::new();
    byte_range_arr.push(PdfObject::Integer(0));
    byte_range_arr.push(PdfObject::Integer(0));
    byte_range_arr.push(PdfObject::Integer(0));
    byte_range_arr.push(PdfObject::Integer(0));

    let mut sig_dict = PdfDictionary::new();
    sig_dict.insert("Type", PdfObject::Name(PdfName::new("Sig")));
    sig_dict.insert("Filter", PdfObject::Name(PdfName::new(filter)));
    sig_dict.insert("SubFilter", PdfObject::Name(PdfName::new(subfilter)));
    sig_dict.insert(
        "Name",
        PdfObject::String(PdfString::literal(config.signer_name.as_bytes())),
    );
    sig_dict.insert(
        "Reason",
        PdfObject::String(PdfString::literal(config.reason.as_bytes())),
    );
    sig_dict.insert(
        "Location",
        PdfObject::String(PdfString::literal(config.location.as_bytes())),
    );
    sig_dict.insert(
        "M",
        PdfObject::String(PdfString::literal(pdf_date.as_bytes())),
    );
    if let Some(contact) = config
        .contact_info
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        sig_dict.insert(
            "ContactInfo",
            PdfObject::String(PdfString::literal(contact.as_bytes())),
        );
    }
    sig_dict.insert("ByteRange", PdfObject::Array(byte_range_arr));
    sig_dict.insert(
        "Contents",
        PdfObject::String(PdfString::hex(contents_hole.to_vec())),
    );

    let sig_dict_id = doc.alloc_object_id();
    doc.objects
        .insert(sig_dict_id, PdfObject::Dictionary(sig_dict));

    let ap_stream = synthesize_signature_appearance(config, &iso_date, badge_line);
    let ap_stream_id = doc.alloc_object_id();
    doc.objects
        .insert(ap_stream_id, PdfObject::Stream(ap_stream));

    let field_name = format!("SignatureField_{}", sig_dict_id.number);
    let mut annot_dict = PdfDictionary::new();
    annot_dict.insert("Type", PdfObject::Name(PdfName::new("Annot")));
    annot_dict.insert("Subtype", PdfObject::Name(PdfName::new("Widget")));
    annot_dict.insert("FT", PdfObject::Name(PdfName::new("Sig")));
    annot_dict.insert(
        "T",
        PdfObject::String(PdfString::literal(field_name.as_bytes())),
    );
    annot_dict.insert("F", PdfObject::Integer(132));

    let mut rect_arr = PdfArray::new();
    rect_arr.push(PdfObject::Real(config.rect[0]));
    rect_arr.push(PdfObject::Real(config.rect[1]));
    rect_arr.push(PdfObject::Real(config.rect[2]));
    rect_arr.push(PdfObject::Real(config.rect[3]));
    annot_dict.insert("Rect", PdfObject::Array(rect_arr));

    let mut ap_dict = PdfDictionary::new();
    ap_dict.insert("N", PdfObject::Reference(ap_stream_id));
    annot_dict.insert("AP", PdfObject::Dictionary(ap_dict));
    annot_dict.insert("V", PdfObject::Reference(sig_dict_id));

    let annot_id = doc.alloc_object_id();
    let page_id = pages[config.page_number - 1];
    annot_dict.insert("P", PdfObject::Reference(page_id));
    doc.objects
        .insert(annot_id, PdfObject::Dictionary(annot_dict));

    if let Some(page_obj) = doc.objects.get_mut(&page_id) {
        if let Some(dict) = page_obj.as_dict_mut() {
            if !dict.contains_key("Annots") {
                dict.insert("Annots", PdfObject::Array(PdfArray::new()));
            }
            if let Some(annots_arr) = dict.get_mut("Annots").and_then(|o| o.as_array_mut()) {
                annots_arr.push(PdfObject::Reference(annot_id));
            }
        }
    }

    let root_ref = doc
        .xref
        .trailer
        .get("Root")
        .and_then(|o| o.as_reference())
        .ok_or_else(|| PdfError::ParseError {
            offset: 0,
            message: "Missing /Root in trailer".to_string(),
        })?;

    let acroform_ref = if let Some(PdfObject::Dictionary(cat)) = doc.objects.get(&root_ref) {
        cat.get("AcroForm").and_then(|o| o.as_reference())
    } else {
        None
    };

    let acroform_id = match acroform_ref {
        Some(aid) => aid,
        None => {
            let aid = doc.alloc_object_id();
            let mut af_dict = PdfDictionary::new();
            af_dict.insert("Fields", PdfObject::Array(PdfArray::new()));
            af_dict.insert("SigFlags", PdfObject::Integer(3));
            doc.objects.insert(aid, PdfObject::Dictionary(af_dict));
            if let Some(PdfObject::Dictionary(cat)) = doc.objects.get_mut(&root_ref) {
                cat.insert("AcroForm", PdfObject::Reference(aid));
            }
            aid
        }
    };

    if let Some(PdfObject::Dictionary(af)) = doc.objects.get_mut(&acroform_id) {
        af.insert("SigFlags", PdfObject::Integer(3));
        if !af.contains_key("Fields") {
            af.insert("Fields", PdfObject::Array(PdfArray::new()));
        }
        if let Some(fields_arr) = af.get_mut("Fields").and_then(|o| o.as_array_mut()) {
            fields_arr.push(PdfObject::Reference(annot_id));
        }
    }

    Ok(field_name)
}

/// Reads every `/Sig` dictionary and checks `/Contents` against `/ByteRange`.
pub fn verify_document_signatures(doc: &PdfDocument) -> Vec<VerifiedSignature> {
    let objects = objects_for_verification(doc);
    let mut signatures = Vec::new();

    for (id, obj) in &objects {
        let Some(dict) = obj.as_dict() else {
            continue;
        };
        if dict.get("Type").and_then(|o| o.as_name()) != Some("Sig") {
            continue;
        }

        let signer_name = dict_text(dict, "Name").unwrap_or_else(|| "Unknown Signer".to_string());
        let reason = dict_text(dict, "Reason").unwrap_or_default();
        let location = dict_text(dict, "Location").unwrap_or_default();
        let date = dict_text(dict, "M").unwrap_or_default();
        let sub_filter = dict
            .get("SubFilter")
            .and_then(|o| o.as_name())
            .unwrap_or("")
            .to_string();

        let contents = dict
            .get("Contents")
            .and_then(|o| o.as_string())
            .map(|value| value.bytes.clone())
            .unwrap_or_default();
        let contents_hex = contents
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect();

        let mut byte_range = Vec::new();
        let mut ranges_ok = true;
        if let Some(arr) = dict.get("ByteRange").and_then(|o| o.as_array()) {
            for item in arr {
                match item.as_integer() {
                    Some(value) if value >= 0 => match usize::try_from(value) {
                        Ok(parsed) => byte_range.push(parsed),
                        Err(_) => ranges_ok = false,
                    },
                    _ => ranges_ok = false,
                }
            }
        } else {
            ranges_ok = false;
        }

        let byte_range_valid = if !ranges_ok {
            false
        } else if sub_filter == SUBFILTER_NAME {
            attestation_matches(doc.raw_data(), &byte_range, &contents)
        } else if sub_filter == "adbe.pkcs7.detached" {
            crate::security::cms::cms_byte_range_valid(doc.raw_data(), &byte_range, &contents)
        } else {
            false
        };

        let mut field_name = format!("Signature_{}", id.number);
        let mut rect = [0.0, 0.0, 0.0, 0.0];
        let page_num = 1;
        for aobj in objects.values() {
            let Some(adict) = aobj.as_dict() else {
                continue;
            };
            if adict.get("V").and_then(|o| o.as_reference()) != Some(*id) {
                continue;
            }
            if let Some(name) = dict_text(adict, "T") {
                field_name = name;
            }
            if let Some(rarr) = adict.get("Rect").and_then(|o| o.as_array()) {
                if rarr.len() == 4 {
                    rect = [
                        rarr[0].as_real().unwrap_or(0.0),
                        rarr[1].as_real().unwrap_or(0.0),
                        rarr[2].as_real().unwrap_or(0.0),
                        rarr[3].as_real().unwrap_or(0.0),
                    ];
                }
            }
            break;
        }

        signatures.push(VerifiedSignature {
            field_name,
            signer_name,
            reason,
            location,
            date,
            sub_filter,
            byte_range,
            contents_hex,
            byte_range_valid,
            rect,
            page_number: page_num,
        });
    }

    signatures
}

/// Rewrites the newest `PDFEngine.sha256` attestation so its digest matches `file`.
///
/// A full save changes offsets, so the previous digest cannot stay valid.
/// Documents without this subfilter are returned unchanged.
pub(crate) fn seal_saved_bytes(file: &mut Vec<u8>) -> PdfResult<()> {
    let Some(sub_at) = find_attestation(file) else {
        return Ok(());
    };
    // String encryption replaces the clear 32-byte digest with ciphertext.
    // Resealing that ciphertext belongs to the encryption writer, not here.
    if !clear_digest_hole(file, sub_at) {
        if file
            .windows(b"/Encrypt".len())
            .any(|window| window == b"/Encrypt")
        {
            return Ok(());
        }
        return Err(PdfError::CryptographyError(
            "Integrity attestation /Contents must be a 32-byte hex string.".into(),
        ));
    }
    let obj_start = object_start(file, sub_at);
    let bracket = find_byte_range_bracket(file, obj_start, sub_at)?;
    let array_end = file[bracket..]
        .iter()
        .position(|byte| *byte == b']')
        .map(|rel| bracket + rel + 1)
        .ok_or_else(|| {
            PdfError::CryptographyError("Integrity attestation /ByteRange is not closed.".into())
        })?;
    let padded = padded_byte_range(0, 0, 0, 0);
    if &file[bracket..array_end] != padded.as_bytes() {
        let delta = padded.len() as i64 - (array_end - bracket) as i64;
        file.splice(bracket..array_end, padded.bytes());
        adjust_classic_xref(file, bracket, delta)?;
    }
    write_digest(file)
}

fn write_digest(file: &mut Vec<u8>) -> PdfResult<()> {
    let sub_at = find_attestation(file).ok_or_else(|| {
        PdfError::CryptographyError("Integrity attestation disappeared while sealing.".into())
    })?;
    let obj_start = object_start(file, sub_at);
    let contents_at = rfind(&file[obj_start..sub_at], b"/Contents <")
        .map(|rel| obj_start + rel)
        .ok_or_else(|| {
            PdfError::CryptographyError("Integrity attestation is missing /Contents.".into())
        })?;
    let hex_start = contents_at + b"/Contents <".len();
    let hex_len = file[hex_start..sub_at]
        .iter()
        .position(|byte| *byte == b'>')
        .ok_or_else(|| {
            PdfError::CryptographyError("Integrity attestation /Contents is not closed.".into())
        })?;
    if hex_len != 64
        || !file[hex_start..hex_start + hex_len]
            .iter()
            .all(u8::is_ascii_hexdigit)
    {
        return Err(PdfError::CryptographyError(
            "Integrity attestation /Contents must be a 32-byte hex string.".into(),
        ));
    }
    let hex_end = hex_start + hex_len;
    let bracket = find_byte_range_bracket(file, obj_start, hex_start)?;
    let padded = padded_byte_range(0, 0, 0, 0);
    let array_end = bracket + padded.len();
    if array_end > file.len() || &file[bracket..array_end] != padded.as_bytes() {
        return Err(PdfError::CryptographyError(
            "Integrity attestation /ByteRange could not be reserved at a fixed width.".into(),
        ));
    }

    let len1 = hex_start;
    let start2 = hex_end;
    let len2 = file.len() - start2;
    if len1 > 9_999_999_999 || start2 > 9_999_999_999 || len2 > 9_999_999_999 {
        return Err(PdfError::CryptographyError(
            "Integrity attestation does not support files whose offsets exceed 10 digits.".into(),
        ));
    }
    let rendered = padded_byte_range(0, len1 as u64, start2 as u64, len2 as u64);
    file[bracket..array_end].copy_from_slice(rendered.as_bytes());

    let mut covered = Vec::with_capacity(len1 + len2);
    covered.extend_from_slice(&file[..len1]);
    covered.extend_from_slice(&file[start2..]);
    let digest = sha256(&covered);
    let encoded: String = digest.iter().map(|byte| format!("{:02X}", byte)).collect();
    file[hex_start..hex_end].copy_from_slice(encoded.as_bytes());
    Ok(())
}

fn attestation_matches(file: &[u8], byte_range: &[usize], contents: &[u8]) -> bool {
    if byte_range.len() != 4 || contents.len() != 32 {
        return false;
    }
    let start1 = byte_range[0];
    let len1 = byte_range[1];
    let start2 = byte_range[2];
    let len2 = byte_range[3];
    if start1 != 0 || len1 > file.len() || start2 > file.len() {
        return false;
    }
    let Some(end1) = start1.checked_add(len1) else {
        return false;
    };
    let Some(end2) = start2.checked_add(len2) else {
        return false;
    };
    if end1 != len1 || start2 != len1 + 64 || end2 != file.len() {
        return false;
    }
    if file.get(len1.wrapping_sub(1)) != Some(&b'<') || file.get(start2) != Some(&b'>') {
        return false;
    }
    let mut covered = Vec::with_capacity(len1 + len2);
    covered.extend_from_slice(&file[..len1]);
    covered.extend_from_slice(&file[start2..]);
    sha256(&covered).as_slice() == contents
}

fn objects_for_verification(doc: &PdfDocument) -> BTreeMap<ObjectId, PdfObject> {
    let mut objects = BTreeMap::new();
    for (&id, entry) in &doc.xref.entries {
        let XRefEntry::InUse { offset, .. } = entry else {
            continue;
        };
        let mut parser = Parser::at_offset(doc.raw_data(), *offset as usize);
        if let Ok((parsed_id, obj)) = parser.parse_indirect_object() {
            if parsed_id == id {
                objects.insert(id, obj);
            }
        }
    }
    for (id, obj) in &doc.objects {
        objects.entry(*id).or_insert_with(|| obj.clone());
    }
    objects
}

fn dict_text(dict: &PdfDictionary, key: &str) -> Option<String> {
    dict.get(key)
        .and_then(|obj| obj.as_string())
        .map(|value| String::from_utf8_lossy(&value.bytes).into_owned())
}

fn find_attestation(file: &[u8]) -> Option<usize> {
    find_marked_signature(file, SUBFILTER_MARK)
}

/// Last `mark` whose following 32 bytes contain `/Type /Sig`.
pub(crate) fn find_marked_signature(file: &[u8], mark: &[u8]) -> Option<usize> {
    let mut search_from = 0;
    let mut found = None;
    while search_from + mark.len() <= file.len() {
        let Some(rel) = file[search_from..]
            .windows(mark.len())
            .position(|window| window == mark)
        else {
            break;
        };
        let at = search_from + rel;
        let after = at + mark.len();
        let window_end = (after + 32).min(file.len());
        if file[after..window_end]
            .windows(TYPE_SIG_MARK.len())
            .any(|window| window == TYPE_SIG_MARK)
        {
            found = Some(at);
        }
        search_from = after;
    }
    found
}

/// Replaces a short `/ByteRange [ 0 0 0 0 ]` with the fixed 10-digit zero form.
pub(crate) fn widen_byte_range_placeholder(file: &mut Vec<u8>, mark: &[u8]) -> PdfResult<()> {
    let sub_at = find_marked_signature(file, mark).ok_or_else(|| {
        PdfError::CryptographyError(
            "Signature marker disappeared while reserving ByteRange.".into(),
        )
    })?;
    let obj_start = object_start(file, sub_at);
    let bracket = find_byte_range_bracket(file, obj_start, sub_at)?;
    let array_end = file[bracket..]
        .iter()
        .position(|byte| *byte == b']')
        .map(|rel| bracket + rel + 1)
        .ok_or_else(|| PdfError::CryptographyError("Signature /ByteRange is not closed.".into()))?;
    let padded = padded_byte_range(0, 0, 0, 0);
    if &file[bracket..array_end] != padded.as_bytes() {
        let delta = padded.len() as i64 - (array_end - bracket) as i64;
        file.splice(bracket..array_end, padded.bytes());
        adjust_classic_xref(file, bracket, delta)?;
    }
    Ok(())
}

/// Writes the real 10-digit `/ByteRange` over the reserved zeros.
///
/// Returns the Contents hex span. The hex digits stay excluded from the range.
pub(crate) fn write_covered_byte_range(file: &mut [u8], mark: &[u8]) -> PdfResult<(usize, usize)> {
    let (hex_start, hex_end, bracket, array_end) = contents_geometry(file, mark)?;
    let padded = padded_byte_range(0, 0, 0, 0);
    if array_end > file.len() || &file[bracket..array_end] != padded.as_bytes() {
        return Err(PdfError::CryptographyError(
            "Signature /ByteRange could not be reserved at a fixed width.".into(),
        ));
    }
    let len1 = hex_start;
    let start2 = hex_end;
    let len2 = file.len() - start2;
    if len1 > 9_999_999_999 || start2 > 9_999_999_999 || len2 > 9_999_999_999 {
        return Err(PdfError::CryptographyError(
            "Signature does not support files whose offsets exceed 10 digits.".into(),
        ));
    }
    let rendered = padded_byte_range(0, len1 as u64, start2 as u64, len2 as u64);
    file[bracket..array_end].copy_from_slice(rendered.as_bytes());
    Ok((hex_start, hex_end))
}

/// Locates the Contents hex digits of the marked signature.
pub(crate) fn contents_hex_span(file: &[u8], mark: &[u8]) -> PdfResult<(usize, usize)> {
    let (hex_start, hex_end, _, _) = contents_geometry(file, mark)?;
    Ok((hex_start, hex_end))
}

fn contents_geometry(file: &[u8], mark: &[u8]) -> PdfResult<(usize, usize, usize, usize)> {
    let sub_at = find_marked_signature(file, mark).ok_or_else(|| {
        PdfError::CryptographyError("Signature marker disappeared while sealing.".into())
    })?;
    let obj_start = object_start(file, sub_at);
    let contents_at = rfind(&file[obj_start..sub_at], b"/Contents <")
        .map(|rel| obj_start + rel)
        .ok_or_else(|| PdfError::CryptographyError("Signature is missing /Contents.".into()))?;
    let hex_start = contents_at + b"/Contents <".len();
    let hex_len = file[hex_start..sub_at]
        .iter()
        .position(|byte| *byte == b'>')
        .ok_or_else(|| PdfError::CryptographyError("Signature /Contents is not closed.".into()))?;
    if hex_len == 0 || hex_len % 2 != 0 {
        return Err(PdfError::CryptographyError(
            "Signature /Contents hex length is not an even positive width.".into(),
        ));
    }
    let hex_end = hex_start + hex_len;
    let bracket = find_byte_range_bracket(file, obj_start, hex_start)?;
    let padded_len = padded_byte_range(0, 0, 0, 0).len();
    Ok((hex_start, hex_end, bracket, bracket + padded_len))
}

/// Writes `payload` as uppercase hex and zero-pads the rest of the hole.
pub(crate) fn write_contents_hex(
    file: &mut [u8],
    hex_start: usize,
    hex_end: usize,
    payload: &[u8],
) -> PdfResult<()> {
    let capacity = (hex_end - hex_start) / 2;
    if payload.len() > capacity {
        return Err(PdfError::CryptographyError(
            "CMS does not fit the reserved contents hole.".into(),
        ));
    }
    let mut raw = vec![0u8; capacity];
    raw[..payload.len()].copy_from_slice(payload);
    let encoded: String = raw.iter().map(|byte| format!("{:02X}", byte)).collect();
    if encoded.len() != hex_end - hex_start {
        return Err(PdfError::CryptographyError(
            "CMS does not fit the reserved contents hole.".into(),
        ));
    }
    file[hex_start..hex_end].copy_from_slice(encoded.as_bytes());
    Ok(())
}

fn clear_digest_hole(file: &[u8], sub_at: usize) -> bool {
    let obj_start = object_start(file, sub_at);
    let Some(rel) = rfind(&file[obj_start..sub_at], b"/Contents <") else {
        return false;
    };
    let hex_start = obj_start + rel + b"/Contents <".len();
    let Some(hex_len) = file[hex_start..sub_at]
        .iter()
        .position(|byte| *byte == b'>')
    else {
        return false;
    };
    hex_len == 64
        && file[hex_start..hex_start + hex_len]
            .iter()
            .all(u8::is_ascii_hexdigit)
}

pub(crate) fn object_start(file: &[u8], position: usize) -> usize {
    rfind(&file[..position], b"endobj")
        .map(|at| at + b"endobj".len())
        .unwrap_or(0)
}

pub(crate) fn find_byte_range_bracket(file: &[u8], start: usize, end: usize) -> PdfResult<usize> {
    let key = rfind(&file[start..end], b"/ByteRange ").ok_or_else(|| {
        PdfError::CryptographyError("Integrity attestation is missing /ByteRange.".into())
    })?;
    let after_key = start + key + b"/ByteRange ".len();
    file[after_key..end]
        .iter()
        .position(|byte| *byte == b'[')
        .map(|rel| after_key + rel)
        .ok_or_else(|| {
            PdfError::CryptographyError("Integrity attestation /ByteRange has no array.".into())
        })
}

pub(crate) fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// Shifts classic xref offsets and `startxref` after an insertion at `pivot`.
pub(crate) fn adjust_classic_xref(file: &mut Vec<u8>, pivot: usize, delta: i64) -> PdfResult<()> {
    if delta == 0 {
        return Ok(());
    }
    let xref_at = rfind(file, b"\nxref\n").map(|at| at + 1).ok_or_else(|| {
        PdfError::CryptographyError(
            "Sealed file is missing a classic cross-reference table.".into(),
        )
    })?;
    let mut pos = xref_at + b"xref\n".len();
    while pos < file.len() && !file[pos..].starts_with(b"trailer") {
        let (next, line) = read_line(file, pos)?;
        let (_, count) = parse_subsection_header(line)?;
        if count > file.len() {
            return Err(PdfError::CryptographyError(
                "Cross-reference subsection is larger than the file.".into(),
            ));
        }
        pos = next;
        for _ in 0..count {
            if pos + 20 > file.len() {
                return Err(PdfError::CryptographyError(
                    "Cross-reference entry runs past the end of the file.".into(),
                ));
            }
            let flag = file[pos + 17];
            let newline = file[pos + 19];
            if newline != b'\n' || (flag != b'n' && flag != b'f') {
                return Err(PdfError::CryptographyError(
                    "Cross-reference entry is not a 20-byte classic record.".into(),
                ));
            }
            if flag == b'n' {
                let mut digits = [0u8; 10];
                digits.copy_from_slice(&file[pos..pos + 10]);
                let offset = parse_u64(&digits)?;
                if offset > pivot as u64 {
                    let updated = offset as i64 + delta;
                    if !(0..=9_999_999_999).contains(&updated) {
                        return Err(PdfError::CryptographyError(
                            "Cross-reference offset does not fit in 10 digits after sealing."
                                .into(),
                        ));
                    }
                    let rendered = format!("{:010}", updated as u64);
                    file[pos..pos + 10].copy_from_slice(rendered.as_bytes());
                }
            }
            pos += 20;
        }
    }
    if pos >= file.len() || !file[pos..].starts_with(b"trailer") {
        return Err(PdfError::CryptographyError(
            "Cross-reference table does not end at trailer.".into(),
        ));
    }

    let startxref_at = rfind(file, b"\nstartxref\n")
        .ok_or_else(|| PdfError::CryptographyError("Sealed file is missing startxref.".into()))?;
    let num_start = startxref_at + 1 + b"startxref\n".len();
    let num_len = file[num_start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| PdfError::CryptographyError("startxref offset is not terminated.".into()))?;
    let old = parse_u64(&file[num_start..num_start + num_len])?;
    let new = old as i64 + delta;
    if new < 0 {
        return Err(PdfError::CryptographyError(
            "startxref became negative while sealing.".into(),
        ));
    }
    file.splice(num_start..num_start + num_len, new.to_string().bytes());
    Ok(())
}

fn read_line<'a>(file: &'a [u8], pos: usize) -> PdfResult<(usize, &'a [u8])> {
    let rel = file[pos..]
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| {
            PdfError::CryptographyError("Cross-reference table ended before trailer.".into())
        })?;
    Ok((pos + rel + 1, &file[pos..pos + rel]))
}

fn parse_subsection_header(line: &[u8]) -> PdfResult<(u64, usize)> {
    let text = std::str::from_utf8(line)
        .map_err(|_| PdfError::CryptographyError("Cross-reference header is not ASCII.".into()))?;
    let mut parts = text.split_whitespace();
    let start = parts.next().ok_or_else(|| {
        PdfError::CryptographyError("Cross-reference header is missing its start.".into())
    })?;
    let count = parts.next().ok_or_else(|| {
        PdfError::CryptographyError("Cross-reference header is missing its count.".into())
    })?;
    let start = start.parse::<u64>().map_err(|_| {
        PdfError::CryptographyError("Cross-reference header start is not an integer.".into())
    })?;
    let count = count.parse::<usize>().map_err(|_| {
        PdfError::CryptographyError("Cross-reference header count is not an integer.".into())
    })?;
    Ok((start, count))
}

fn parse_u64(bytes: &[u8]) -> PdfResult<u64> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| PdfError::CryptographyError("Offset is not ASCII.".into()))?;
    text.trim()
        .parse::<u64>()
        .map_err(|_| PdfError::CryptographyError(format!("Offset '{text}' is not an integer.")))
}

fn utc_timestamps() -> (String, String) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let (year, month, day, hour, minute, second) = civil_utc(seconds);
    let iso = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z");
    let pdf = format!("D:{year:04}{month:02}{day:02}{hour:02}{minute:02}{second:02}Z");
    (iso, pdf)
}

/// Converts a Unix timestamp to a UTC civil date. Howard Hinnant's algorithm.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_page_doc() -> PdfDocument {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        let off1 = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let off2 = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        let off3 = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n",
        );
        let off4 = pdf.len();
        pdf.extend_from_slice(
            b"4 0 obj\n<< /Length 20 >>\nstream\nq\n(Page 1) Tj\nQ\nendstream\nendobj\n",
        );
        let xref_offset = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n");
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off3).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off4).as_bytes());
        pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
        pdf.extend_from_slice(format!("startxref\n{}\n%%EOF\n", xref_offset).as_bytes());
        PdfDocument::load(&pdf).expect("Load test PDF")
    }

    fn covered_digest(file: &[u8], byte_range: &[usize]) -> [u8; 32] {
        let mut covered = Vec::new();
        covered.extend_from_slice(&file[byte_range[0]..byte_range[0] + byte_range[1]]);
        covered.extend_from_slice(&file[byte_range[2]..byte_range[2] + byte_range[3]]);
        sha256(&covered)
    }

    #[test]
    fn test_sign_and_verify_signature_roundtrip() {
        let mut doc = create_test_page_doc();
        let config = DigitalSignatureConfig {
            signer_name: "Lic. Roberto Garduño".to_string(),
            reason: "Contrato Comercial Aprobado".to_string(),
            location: "CDMX".to_string(),
            rect: [100.0, 100.0, 300.0, 160.0],
            page_number: 1,
            visual_badge: true,
            contact_info: Some("rgarduno@company.com".to_string()),
        };

        let sig = sign_document(&mut doc, &config).unwrap();
        assert_eq!(sig.signer_name, "Lic. Roberto Garduño");
        assert_eq!(sig.reason, "Contrato Comercial Aprobado");
        assert_eq!(sig.sub_filter, "PDFEngine.sha256");
        assert_ne!(sig.byte_range, vec![0, 1024, 2048, 4096]);
        assert!(sig.byte_range_valid);
        assert_eq!(sig.contents_hex.len(), 64);
        assert!(sig.date.starts_with("D:"));
        assert!(!sig.date.contains("20261002152000"));

        let file = doc.raw_data();
        assert!(file
            .windows(b"/PDFEngine.sha256".len())
            .any(|w| w == b"/PDFEngine.sha256"));
        assert!(file
            .windows(b"/PDFEngine.Approval".len())
            .any(|w| w == b"/PDFEngine.Approval"));
        assert!(!file
            .windows(b"adbe.pkcs7.detached".len())
            .any(|w| w == b"adbe.pkcs7.detached"));
        assert!(!file
            .windows(b"Adobe.PPKLite".len())
            .any(|w| w == b"Adobe.PPKLite"));
        assert!(file
            .windows(b"rgarduno@company.com".len())
            .any(|w| w == b"rgarduno@company.com"));

        let digest = covered_digest(file, &sig.byte_range);
        let digest_hex = digest
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect::<String>();
        assert_eq!(sig.contents_hex, digest_hex);
        assert_eq!(
            sig.byte_range[0] + sig.byte_range[1] + 64 + sig.byte_range[3],
            file.len()
        );

        let list = verify_document_signatures(&doc);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].signer_name, "Lic. Roberto Garduño");
        assert_eq!(list[0].location, "CDMX");
        assert_eq!(list[0].rect, [100.0, 100.0, 300.0, 160.0]);
        assert!(list[0].byte_range_valid);

        let mut tampered = file.to_vec();
        tampered[10] ^= 0xFF;
        let tampered_doc = PdfDocument::load(&tampered).unwrap();
        let tampered_sigs = verify_document_signatures(&tampered_doc);
        assert_eq!(tampered_sigs.len(), 1);
        assert!(!tampered_sigs[0].byte_range_valid);

        let exported = doc.save_to_vec().unwrap();
        let exported_doc = PdfDocument::load(&exported).unwrap();
        let exported_sigs = verify_document_signatures(&exported_doc);
        assert_eq!(exported_sigs.len(), 1);
        assert!(exported_sigs[0].byte_range_valid);
        let exported_digest = covered_digest(exported_doc.raw_data(), &exported_sigs[0].byte_range);
        let exported_hex = exported_digest
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect::<String>();
        assert_eq!(exported_sigs[0].contents_hex, exported_hex);
    }

    #[test]
    fn test_reason_text_cannot_spoof_the_attestation_marker() {
        let mut doc = create_test_page_doc();
        let mut config = DigitalSignatureConfig::default();
        config.reason = "note /SubFilter /PDFEngine.sha256 trailing".to_string();
        let sig = sign_document(&mut doc, &config).unwrap();
        assert!(sig.byte_range_valid);
        assert_eq!(sig.sub_filter, "PDFEngine.sha256");
    }

    #[test]
    fn test_civil_utc_unix_epoch() {
        assert_eq!(civil_utc(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil_utc(86_400), (1970, 1, 2, 0, 0, 0));
    }
}
