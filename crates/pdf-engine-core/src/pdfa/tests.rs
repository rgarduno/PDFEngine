//! Structural checks for the engine's own PDF/A conversion.
//!
//! These tests do not claim a veraPDF result, an Acrobat preflight, or a
//! court acceptance. They check the face, the profile, and the object graph
//! this crate writes.

use crate::annots::{add_shape, ShapeKind, ShapeStyle};
use crate::cos::{decode_stream, ObjectId, PdfDictionary, PdfDocument, PdfName, PdfObject, XRefEntry};
use crate::error::PdfError;

use super::face::{build_face, font_checksum, glyph_outline, table_length};
use super::icc::srgb_profile;
use super::{convert_to_pdfa, validate_pdfa, PdfALevel};

fn one_page() -> PdfDocument {
    let mut doc = PdfDocument::empty();
    let pages_id = doc.pages_id().expect("pages");
    let page_id = doc.alloc_object_id();
    let mut page = PdfDictionary::new();
    page.insert("Type", PdfName::new("Page"));
    page.insert("Parent", pages_id);
    page.insert(
        "MediaBox",
        PdfObject::Array(vec![
            PdfObject::Integer(0),
            PdfObject::Integer(0),
            PdfObject::Integer(612),
            PdfObject::Integer(792),
        ]),
    );
    doc.set_object(page_id, PdfObject::Dictionary(page));

    let mut pages = match doc.get_object(pages_id).expect("pages object") {
        PdfObject::Dictionary(dict) => dict,
        _ => panic!("pages root"),
    };
    pages.insert("Kids", PdfObject::Array(vec![PdfObject::Reference(page_id)]));
    pages.insert("Count", 1i64);
    doc.set_object(pages_id, PdfObject::Dictionary(pages));
    doc
}

fn put_font(doc: &mut PdfDocument, font: PdfDictionary) -> ObjectId {
    let id = doc.alloc_object_id();
    doc.set_object(id, PdfObject::Dictionary(font));
    id
}

fn simple_font(base: &str, encoding: PdfObject) -> PdfDictionary {
    let mut font = PdfDictionary::new();
    font.insert("Type", PdfName::new("Font"));
    font.insert("Subtype", PdfName::new("Type1"));
    font.insert("BaseFont", PdfName::new(base));
    font.insert("Encoding", encoding);
    font
}

fn rect_array() -> PdfObject {
    PdfObject::Array(vec![
        PdfObject::Integer(10),
        PdfObject::Integer(10),
        PdfObject::Integer(110),
        PdfObject::Integer(40),
    ])
}

fn put_annot(subtype: &str) -> PdfDictionary {
    let mut annot = PdfDictionary::new();
    annot.insert("Type", PdfName::new("Annot"));
    annot.insert("Subtype", PdfName::new(subtype));
    annot.insert("Rect", rect_array());
    annot
}

fn error_text(error: PdfError) -> String {
    error.to_string()
}

fn walk_numbers(object: &PdfObject, hit: &mut bool, target: f64) {
    match object {
        PdfObject::Real(value) if (*value - target).abs() < 0.001 => *hit = true,
        PdfObject::Dictionary(dict) => {
            for (_, value) in dict.iter() {
                walk_numbers(value, hit, target);
            }
        }
        PdfObject::Stream(stream) => {
            for (_, value) in stream.dict.iter() {
                walk_numbers(value, hit, target);
            }
        }
        PdfObject::Array(items) => {
            for value in items {
                walk_numbers(value, hit, target);
            }
        }
        _ => {}
    }
}

fn contains_real(doc: &PdfDocument, target: f64) -> bool {
    let mut hit = false;
    for object in doc.objects.values() {
        walk_numbers(object, &mut hit, target);
    }
    hit
}

fn assert_forced_opaque(object: &PdfObject) {
    match object {
        PdfObject::Dictionary(dict) => check_opaque_dict(dict),
        PdfObject::Stream(stream) => check_opaque_dict(&stream.dict),
        PdfObject::Array(items) => {
            for value in items {
                assert_forced_opaque(value);
            }
        }
        _ => {}
    }
}

fn check_opaque_dict(dict: &PdfDictionary) {
    for key in ["ca", "CA"] {
        if let Some(value) = dict.get(key).and_then(|item| item.as_f64()) {
            assert!(
                (value - 1.0).abs() <= 0.001,
                "{key} stayed at {value}"
            );
        }
    }
    if dict.contains_key("BM") {
        assert_eq!(dict.get("BM").and_then(|item| item.as_name()), Some("Normal"));
    }
    assert!(dict.get("SMask").is_none());
    for (_, value) in dict.iter() {
        assert_forced_opaque(value);
    }
}

fn true_type_widths(doc: &PdfDocument) -> Vec<i64> {
    for object in doc.objects.values() {
        let Some(dict) = object.as_dict() else {
            continue;
        };
        if dict.get("Subtype").and_then(|item| item.as_name()) != Some("TrueType") {
            continue;
        }
        let Some(array) = dict.get("Widths").and_then(|item| item.as_array()) else {
            continue;
        };
        return array.iter().filter_map(|item| item.as_i64()).collect();
    }
    panic!("embedded face missing");
}

fn font_program(doc: &mut PdfDocument) -> Vec<u8> {
    let ids: Vec<ObjectId> = doc
        .xref
        .entries
        .iter()
        .filter(|(_, entry)| !matches!(entry, XRefEntry::Free { .. }))
        .map(|(id, _)| *id)
        .filter(|id| id.number != 0)
        .collect();
    for id in ids {
        let object = doc.get_object(id).expect("object");
        let PdfObject::Stream(stream) = object else {
            continue;
        };
        if stream.dict.get("Length1").is_none() {
            continue;
        }
        if stream.dict.get("Filter").and_then(|item| item.as_name()) != Some("FlateDecode") {
            continue;
        }
        return decode_stream("FlateDecode", None, &stream.content, &doc.limits).expect("font");
    }
    panic!("FontFile2 missing");
}

fn link_streams(doc: &PdfDocument) -> Vec<String> {
    let mut found = Vec::new();
    for object in doc.objects.values() {
        let Some(dict) = object.as_dict() else {
            continue;
        };
        if dict.get("Subtype").and_then(|item| item.as_name()) != Some("Link") {
            continue;
        }
        let Some(id) = dict
            .get("AP")
            .and_then(|item| item.as_dict())
            .and_then(|appearance| appearance.get("N"))
            .and_then(|item| item.as_reference())
        else {
            continue;
        };
        if let Some(PdfObject::Stream(stream)) = doc.objects.get(&id) {
            found.push(String::from_utf8_lossy(&stream.content).into_owned());
        }
    }
    found
}

#[test]
fn face_checksum_outlines_and_profile() {
    let uniform = [600i32; 224];
    let face = build_face(&uniform, 0);
    assert_eq!(font_checksum(&face.bytes), 0xB1B0AFBA);
    assert_eq!(table_length(&face.bytes, b"OS/2"), Some(86));
    assert_eq!(table_length(&face.bytes, b"loca"), Some(904));
    assert_eq!(table_length(&face.bytes, b"hmtx"), Some(900));
    assert!(face.base_font.starts_with("AAAAAA+"));

    let letter_a = glyph_outline(&face.bytes, 34).expect("A");
    assert!(letter_a.contours > 0);
    assert!(letter_a.xmax - letter_a.xmin > 10);
    assert!(letter_a.ymin <= letter_a.ymax);
    assert_eq!(letter_a.points, letter_a.contours as usize * 4);

    let letter_h = glyph_outline(&face.bytes, 41).expect("H");
    assert!(letter_h.contours > 0);
    assert!(letter_h.xmax - letter_h.xmin > 10);
    assert!(letter_h.ymin <= letter_h.ymax);
    assert_eq!(letter_h.points, letter_h.contours as usize * 4);

    let space = glyph_outline(&face.bytes, 1).expect("space");
    assert_eq!(space.contours, 0);
    assert_eq!(space.points, 0);
    let missing = glyph_outline(&face.bytes, 0).expect("notdef");
    assert_eq!(missing.contours, 1);
    assert_eq!(missing.points, 4);

    let mut mixed = [600i32; 224];
    mixed[0] = 278;
    mixed[1] = 833;
    let other = build_face(&mixed, 1);
    assert_eq!(font_checksum(&other.bytes), 0xB1B0AFBA);
    assert!(other.base_font.starts_with("AAAAAB+"));

    let profile = srgb_profile();
    assert!(profile.len() >= 40);
    assert_eq!(&profile[36..40], b"acsp");
    let declared = u32::from_be_bytes(profile[0..4].try_into().expect("size"));
    assert_eq!(declared as usize, profile.len());
    assert_eq!(profile.len(), 468);
}

#[test]
fn courier_width_is_kept_and_archive_markers_round_trip() {
    let mut doc = one_page();
    let mut font = simple_font("Courier", PdfObject::Name(PdfName::new("WinAnsiEncoding")));
    font.insert("FirstChar", 65i64);
    font.insert("LastChar", 65i64);
    font.insert("Widths", PdfObject::Array(vec![PdfObject::Integer(777)]));
    put_font(&mut doc, font);

    convert_to_pdfa(&mut doc, PdfALevel::A1b).expect("archive");
    let widths = true_type_widths(&doc);
    assert_eq!(widths.len(), 224);
    assert_eq!(widths[33], 777);
    assert!(widths.iter().enumerate().all(|(index, width)| index == 33 || *width == 600));
    assert!(validate_pdfa(&mut doc, PdfALevel::A1b).expect("issues").is_empty());

    let bytes = doc.save_to_vec().expect("save");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("%PDF-1.4"));
    assert!(text.contains("pdfaid:part"));
    assert!(text.contains(">1</pdfaid:part>"));
    assert!(text.contains("GTS_PDFA1"));
    assert!(text.contains("FontFile2"));
    assert!(text.contains("acsp"));
    assert!(text.contains("/ID"));
    assert!(text.contains("EngineFace"));

    let mut reloaded = PdfDocument::load(&bytes).expect("reload");
    let program = font_program(&mut reloaded);
    assert_eq!(font_checksum(&program), 0xB1B0AFBA);
    let letter_a = glyph_outline(&program, 34).expect("reloaded A");
    assert!(letter_a.contours > 0);
    assert!(validate_pdfa(&mut reloaded, PdfALevel::A1b).expect("reloaded issues").is_empty());
}

#[test]
fn encrypted_document_is_refused_unchanged() {
    let mut doc = one_page();
    let before = doc.catalog().expect("catalog");
    doc.xref.trailer.insert("Encrypt", 4i64);
    let error = convert_to_pdfa(&mut doc, PdfALevel::A1b).expect_err("encrypt");
    assert!(error_text(error).contains("An encrypted document cannot be archived."));
    assert_eq!(doc.catalog().expect("catalog"), before);
    assert!(!before.contains_key("Metadata"));
}

#[test]
fn composite_font_without_a_program_is_refused() {
    let mut doc = one_page();
    let before = doc.catalog().expect("catalog");
    let mut descendant = PdfDictionary::new();
    descendant.insert("Type", PdfName::new("Font"));
    descendant.insert("Subtype", PdfName::new("CIDFontType2"));
    descendant.insert("BaseFont", PdfName::new("Identity"));
    let mut font = PdfDictionary::new();
    font.insert("Type", PdfName::new("Font"));
    font.insert("Subtype", PdfName::new("Type0"));
    font.insert("BaseFont", PdfName::new("Identity-H"));
    font.insert("Encoding", PdfObject::Name(PdfName::new("Identity-H")));
    font.insert(
        "DescendantFonts",
        PdfObject::Array(vec![PdfObject::Dictionary(descendant)]),
    );
    put_font(&mut doc, font);

    let error = convert_to_pdfa(&mut doc, PdfALevel::A2b).expect_err("composite");
    assert!(error_text(error).contains("A composite font cannot be embedded for archive."));
    assert_eq!(doc.catalog().expect("catalog"), before);

    let mut standalone = one_page();
    let before_cid = standalone.catalog().expect("catalog");
    let mut cid = PdfDictionary::new();
    cid.insert("Type", PdfName::new("Font"));
    cid.insert("Subtype", PdfName::new("CIDFontType2"));
    cid.insert("BaseFont", PdfName::new("Identity"));
    put_font(&mut standalone, cid);
    let error = convert_to_pdfa(&mut standalone, PdfALevel::A1b).expect_err("cid");
    assert!(error_text(error).contains("A composite font cannot be embedded for archive."));
    assert_eq!(standalone.catalog().expect("catalog"), before_cid);
}

#[test]
fn custom_encoding_is_refused() {
    let mut doc = one_page();
    let before = doc.catalog().expect("catalog");
    let mut differences = PdfDictionary::new();
    differences.insert("Type", PdfName::new("Encoding"));
    differences.insert(
        "Differences",
        PdfObject::Array(vec![
            PdfObject::Integer(65),
            PdfObject::Name(PdfName::new("A")),
        ]),
    );
    put_font(
        &mut doc,
        simple_font("Helvetica", PdfObject::Dictionary(differences)),
    );
    let error = convert_to_pdfa(&mut doc, PdfALevel::A1b).expect_err("differences");
    assert!(error_text(error).contains("A custom font encoding cannot be archived."));
    assert_eq!(doc.catalog().expect("catalog"), before);

    let mut mac = one_page();
    let before_mac = mac.catalog().expect("catalog");
    put_font(
        &mut mac,
        simple_font("Helvetica", PdfObject::Name(PdfName::new("MacRomanEncoding"))),
    );
    let error = convert_to_pdfa(&mut mac, PdfALevel::A1b).expect_err("mac");
    assert!(error_text(error).contains("A custom font encoding cannot be archived."));
    assert_eq!(mac.catalog().expect("catalog"), before_mac);
}

#[test]
fn part_one_forces_opacity_and_part_two_keeps_it() {
    let mut first = one_page();
    add_shape(
        &mut first,
        0,
        ShapeKind::Square,
        &[[10.0, 10.0], [110.0, 80.0]],
        ShapeStyle {
            stroke: [0.0, 0.0, 0.0],
            fill: None,
            line_width: 1.5,
            opacity: 0.4,
            line_ending: None,
        },
    )
    .expect("shape");
    let state_id = first.alloc_object_id();
    let mut state = PdfDictionary::new();
    state.insert("Type", PdfName::new("ExtGState"));
    state.insert("ca", PdfObject::Real(0.4));
    state.insert("CA", PdfObject::Real(0.4));
    state.insert("BM", PdfName::new("Multiply"));
    first.set_object(state_id, PdfObject::Dictionary(state));
    assert!(contains_real(&first, 0.4));

    let mut second = first.clone();
    convert_to_pdfa(&mut first, PdfALevel::A1b).expect("a1");
    for object in first.objects.values() {
        assert_forced_opaque(object);
    }
    let a1 = first.save_to_vec().expect("a1 save");
    assert!(a1.windows(8).any(|window| window == b"%PDF-1.4"));

    convert_to_pdfa(&mut second, PdfALevel::A2b).expect("a2");
    let a2 = second.save_to_vec().expect("a2 save");
    let text = String::from_utf8_lossy(&a2);
    assert!(text.contains("%PDF-1.7"));
    assert!(text.contains("0.4"));
    assert!(text.contains(">2</pdfaid:part>"));
}

#[test]
fn missing_appearance_is_refused_and_a_link_receives_one() {
    let mut highlight = one_page();
    let before = highlight.catalog().expect("catalog");
    let annot = put_annot("Highlight");
    let id = highlight.alloc_object_id();
    highlight.set_object(id, PdfObject::Dictionary(annot));
    let error = convert_to_pdfa(&mut highlight, PdfALevel::A1b).expect_err("highlight");
    assert!(error_text(error).contains("An annotation has no appearance."));
    assert_eq!(highlight.catalog().expect("catalog"), before);

    let mut links = one_page();
    let plain = put_annot("Link");
    let plain_id = links.alloc_object_id();
    links.set_object(plain_id, PdfObject::Dictionary(plain));

    let mut stroked = put_annot("Link");
    let mut border = PdfDictionary::new();
    border.insert("W", PdfObject::Real(1.0));
    stroked.insert("BS", PdfObject::Dictionary(border));
    stroked.insert(
        "C",
        PdfObject::Array(vec![
            PdfObject::Real(1.0),
            PdfObject::Real(0.0),
            PdfObject::Real(0.0),
        ]),
    );
    let stroked_id = links.alloc_object_id();
    links.set_object(stroked_id, PdfObject::Dictionary(stroked));

    convert_to_pdfa(&mut links, PdfALevel::A2b).expect("links");
    let streams = link_streams(&links);
    assert!(streams.iter().any(|content| content == "q\nQ\n"));
    assert!(streams.iter().any(|content| content.contains(" re\n") && content.contains(" RG\n") && content.contains("\nS\n")));
}
