//! Comprehensive unit tests for PDF Redaction and Content Sanitization.

use super::*;
use crate::cos::object::{ObjectId, PdfObject};
use crate::cos::PdfDocument;
use crate::fonts::FontMetrics;
use crate::images::extract_page_images;
use crate::layout::geometry::Rect;
use crate::stream::parser::{build_ast_from_operations, serialize_ast, ContentStreamTokenizer};

#[test]
fn test_pattern_detection_email() {
    let sample = "Contact us at contact@company.org or support@domain.co.uk for inquiries.";
    let matches = find_emails(sample);
    assert_eq!(matches.len(), 2);
    assert_eq!(&sample[matches[0].0..matches[0].1], "contact@company.org");
    assert_eq!(&sample[matches[1].0..matches[1].1], "support@domain.co.uk");
}

#[test]
fn test_pattern_detection_ssn() {
    let sample = "Customer record SSN: 123-45-6789 and secondary 987-65-4321 confirmed.";
    let matches = find_ssn(sample);
    assert_eq!(matches.len(), 2);
    assert_eq!(&sample[matches[0].0..matches[0].1], "123-45-6789");
    assert_eq!(&sample[matches[1].0..matches[1].1], "987-65-4321");
}

#[test]
fn test_pattern_detection_credit_card() {
    // Standard valid Luhn card: 4532-0150-1234-5678 (4532015012345678)
    // 4*2=8, 5, 3*2=6, 2, 0*2=0, 1, 5*2=1, 0, 1*2=2, 2, 3*2=6, 4, 5*2=1, 6, 7*2=5, 8 => sum=55 (wait, let's construct true Luhn)
    // Let's test 49927398716 or 4532 0150 1234 5678 or 0000 0000 0000 0000 (all 0 is sum 0 % 10 == 0)
    let sample = "Paid with card: 0000-0000-0000-0000 on terminal.";
    let matches = find_credit_cards(sample);
    assert_eq!(matches.len(), 1);
    assert_eq!(&sample[matches[0].0..matches[0].1], "0000-0000-0000-0000");
}

#[test]
fn test_pattern_detection_rfc_and_curp() {
    let sample = "Contribuyente RFC: GARM850101XYZ y CURP: GARM850101HDFRRN01 validado.";
    let rfc_matches = find_rfc(sample);
    assert_eq!(rfc_matches.len(), 1);
    assert_eq!(&sample[rfc_matches[0].0..rfc_matches[0].1], "GARM850101XYZ");

    let curp_matches = find_curp(sample);
    assert_eq!(curp_matches.len(), 1);
    assert_eq!(
        &sample[curp_matches[0].0..curp_matches[0].1],
        "GARM850101HDFRRN01"
    );
}

#[test]
fn test_pattern_detection_substring() {
    let sample = "Project Manhattan is Top Secret. All top secret files must be sealed.";
    let matches = find_substring(sample, "Top Secret", false);
    assert_eq!(matches.len(), 2);
}

#[test]
fn test_surgical_ast_redaction_text_purged() {
    // Construct a content stream containing sensitive text:
    // (Notice width: font_size=12, char advance approx 6pt)
    // "Secret: 123-45-6789 OK"
    // At x=100, y=500:
    // "Secret: " (8 chars ~ 48pt) -> x from 100 to 148
    // "123-45-6789" (11 chars ~ 66pt) -> x from 148 to 214
    // " OK" (3 chars ~ 18pt) -> x from 214 to 232
    let stream = b"BT\n/F1 12 Tf\n1 0 0 1 100 500 Tm\n(Secret: 123-45-6789 OK) Tj\nET\n";
    let mut tokenizer = ContentStreamTokenizer::new(stream);
    let ops = tokenizer.tokenize_all().unwrap();
    let mut ast = build_ast_from_operations(ops);

    let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);

    // Define a redaction rectangle precisely over "123-45-6789"
    // glyphs for "123-45-6789" start around x=148 and end around x=214
    let redact_rect = Rect::new(145.0, 495.0, 216.0, 515.0);
    let redaction = RedactionRect::new(redact_rect).with_overlay_text("[REDACTADO]", None);

    let summary = apply_redaction_to_ast(&mut ast, &[redaction], &metrics).unwrap();

    assert!(summary.purged_glyphs_count >= 10);
    assert_eq!(summary.blackout_boxes_count, 1);

    // Serialize mutated stream
    let output_bytes = serialize_ast(&ast);
    let output_str = String::from_utf8_lossy(&output_bytes);

    // CRITICAL ASSERTION: The sensitive string MUST NOT exist in output bytes!
    assert!(
        !output_str.contains("123-45-6789"),
        "Sensitive text leaked in output bytes!"
    );

    // Non-redacted text MUST be preserved!
    assert!(output_str.contains("(Secret: ) Tj") || output_str.contains("Secret:"));
    assert!(output_str.contains("( OK) Tj") || output_str.contains("OK"));

    // Blackout box vector operators MUST be present
    assert!(output_str.contains("re\nf"));
    assert!(output_str.contains("([REDACTADO]) Tj"));
}

fn create_test_pdf_with_email() -> Vec<u8> {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

    // Object 1: Catalog
    let offset1 = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // Object 2: Pages
    let offset2 = pdf.len();
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

    // Object 3: Page 1 with Annotations
    let offset3 = pdf.len();
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [5 0 R] >>\nendobj\n",
    );

    // Object 4: Contents
    let offset4 = pdf.len();
    let stream_content = b"BT\n/F1 12 Tf\n1 0 0 1 72 700 Tm\n(Client Email: confidential@corp.com Verified) Tj\nET\n";
    pdf.extend_from_slice(
        format!("4 0 obj\n<< /Length {} >>\nstream\n", stream_content.len()).as_bytes(),
    );
    pdf.extend_from_slice(stream_content);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");

    // Object 5: Link Annotation
    let offset5 = pdf.len();
    pdf.extend_from_slice(
        b"5 0 obj\n<< /Type /Annot /Subtype /Link /Rect [140 695 270 715] >>\nendobj\n",
    );

    // Object 6: Info
    let offset6 = pdf.len();
    pdf.extend_from_slice(b"6 0 obj\n<< /Author (Secret Agent) /Title (Classified) >>\nendobj\n");

    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n0 7\n");
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset1).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset2).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset3).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset4).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset5).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset6).as_bytes());

    pdf.extend_from_slice(b"trailer\n<< /Size 7 /Root 1 0 R /Info 6 0 R >>\nstartxref\n");
    pdf.extend_from_slice(format!("{}\n%%EOF", xref_offset).as_bytes());

    pdf
}

#[test]
fn test_full_document_redaction_workflow() {
    let pdf_bytes = create_test_pdf_with_email();
    let mut doc = crate::cos::PdfDocument::load(&pdf_bytes).unwrap();
    let page_id = doc.get_pages().unwrap()[0];

    // Run Pattern Redaction targeting Email
    let mut config = RedactionConfig::default();
    config.scrub_metadata = true;
    config.prune_annotations = true;

    let summaries =
        redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &config).unwrap();

    assert_eq!(summaries.len(), 1);
    assert!(summaries[0].purged_glyphs_count > 0);
    assert_eq!(summaries[0].pruned_annotations_count, 1);

    // Verify content stream bytes
    let updated_bytes = doc.get_page_content_bytes(page_id).unwrap();
    let updated_str = String::from_utf8_lossy(&updated_bytes);
    assert!(!updated_str.contains("confidential@corp.com"));
    assert!(updated_str.contains("Client Email:"));
    assert!(updated_str.contains("Verified"));

    // Verify annotation was pruned from page dictionary
    let updated_page = doc.get_object(page_id).unwrap().as_dict().unwrap().clone();
    if let Some(crate::cos::PdfObject::Array(annots)) = updated_page.get("Annots") {
        assert_eq!(annots.len(), 0);
    }

    // Verify metadata was scrubbed
    let info_id = doc
        .xref
        .trailer
        .get("Info")
        .and_then(|i| i.as_reference())
        .unwrap();
    let updated_info = doc.get_object(info_id).unwrap().as_dict().unwrap().clone();
    assert!(!updated_info.contains_key("Author"));
    assert!(!updated_info.contains_key("Title"));
    assert_eq!(
        updated_info
            .get("Producer")
            .and_then(|p| p.as_string())
            .map(|s| s.to_string_lossy()),
        Some("PDFEngine Sanitizer".to_string())
    );
}

#[test]
fn redaction_strips_alternate_text_on_the_rewritten_page() {
    let stream = b"/Span << /ActualText (hidden-actual) /Alt (hidden-alt) /E (hidden-expansion) /MCID 7 >> BDC\nBT\n/F1 12 Tf\n1 0 0 1 72 700 Tm\n(Visible clause) Tj\nET\nEMC\n";
    let mut tokenizer = ContentStreamTokenizer::new(stream);
    let ops = tokenizer.tokenize_all().unwrap();
    let mut ast = build_ast_from_operations(ops);
    let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
    let redaction = RedactionRect::new(Rect::new(70.0, 690.0, 220.0, 720.0));

    apply_redaction_to_ast(&mut ast, &[redaction], &metrics).unwrap();

    let serialized = serialize_ast(&ast);
    let output = String::from_utf8_lossy(&serialized);
    assert!(!output.contains("hidden-actual"));
    assert!(!output.contains("hidden-alt"));
    assert!(!output.contains("hidden-expansion"));
    assert!(!output.contains("ActualText"));
    assert!(output.contains("MCID"));
}

#[test]
fn redaction_drops_page_metadata_and_keeps_document_info() {
    use crate::cos::object::{PdfDictionary, PdfObject};
    use crate::cos::PdfDocument;

    let mut doc = PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    let page_id = doc.get_pages().unwrap()[0];
    let meta_id = doc.alloc_object_id();
    let mut meta = PdfDictionary::new();
    meta.insert("Type", PdfObject::Name("Metadata".into()));
    doc.set_object(meta_id, PdfObject::Dictionary(meta));

    let mut page = match doc.get_object(page_id).unwrap() {
        PdfObject::Dictionary(dict) => dict,
        _ => panic!("page object"),
    };
    page.insert("Metadata", PdfObject::Reference(meta_id));
    doc.set_object(page_id, PdfObject::Dictionary(page));

    let config = RedactionConfig::default();
    assert!(!config.scrub_metadata);
    redact_document_rectangles(
        &mut doc,
        0,
        &[Rect::new(70.0, 690.0, 400.0, 720.0)],
        &config,
    )
    .unwrap();

    let page = doc.get_object(page_id).unwrap().as_dict().unwrap().clone();
    assert!(!page.contains_key("Metadata"));

    let info_id = doc
        .xref
        .trailer
        .get("Info")
        .and_then(|item| item.as_reference())
        .unwrap();
    let info = doc.get_object(info_id).unwrap().as_dict().unwrap().clone();
    assert!(info.contains_key("Author"));
}

const EMAIL: &str = "confidential@corp.com";

fn attachment_names(doc: &mut crate::cos::PdfDocument) -> Vec<String> {
    let mut names: Vec<String> = crate::ops::list_attachments(doc)
        .unwrap()
        .into_iter()
        .map(|info| info.name)
        .collect();
    names.sort();
    names
}

fn single_page_pdf(stream_content: &[u8], flate: bool) -> Vec<u8> {
    let stored = if flate {
        crate::cos::encode_flate(stream_content, crate::cos::Compression::default()).unwrap()
    } else {
        stream_content.to_vec()
    };
    let filter = if flate { " /Filter /FlateDecode" } else { "" };

    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n");

    let offset1 = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    let offset2 = pdf.len();
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

    let offset3 = pdf.len();
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n",
    );

    let offset4 = pdf.len();
    pdf.extend_from_slice(
        format!("4 0 obj\n<< /Length {}{filter} >>\nstream\n", stored.len()).as_bytes(),
    );
    pdf.extend_from_slice(&stored);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");

    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n0 5\n");
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset1).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset2).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset3).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset4).as_bytes());
    pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n");
    pdf.extend_from_slice(format!("{xref_offset}\n%%EOF").as_bytes());
    pdf
}

fn octal_literal(text: &str) -> String {
    let mut escaped = String::new();
    for byte in text.bytes() {
        escaped.push('\\');
        escaped.push_str(&format!("{byte:03o}"));
    }
    format!("BT\n/F1 12 Tf\n1 0 0 1 72 700 Tm\n({escaped}) Tj\nET\n")
}

fn bytes_contain(haystack: &[u8], needle: &str) -> bool {
    let needle = needle.as_bytes();
    haystack.len() >= needle.len() && haystack.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn redaction_removes_embedded_file_that_repeats_the_email() {
    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    crate::ops::add_attachment(&mut doc, "note.txt", EMAIL.as_bytes(), Some("text/plain"), None)
        .unwrap();
    crate::ops::add_attachment(
        &mut doc,
        "other.txt",
        b"other@corp.com",
        Some("text/plain"),
        None,
    )
    .unwrap();

    redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &RedactionConfig::default())
        .unwrap();

    assert_eq!(attachment_names(&mut doc), vec!["other.txt".to_string()]);
    let kept = crate::ops::extract_attachment(&mut doc, "other.txt").unwrap();
    assert_eq!(kept.data, b"other@corp.com");
    let saved = doc.save_to_vec().unwrap();
    assert!(!bytes_contain(&saved, EMAIL));
}

#[test]
fn redaction_removes_embedded_file_named_or_described_with_the_email() {
    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    crate::ops::add_attachment(
        &mut doc,
        "confidential@corp.com.txt",
        b"hello",
        Some("text/plain"),
        None,
    )
    .unwrap();
    crate::ops::add_attachment(
        &mut doc,
        "memo.txt",
        b"plain notes",
        Some("text/plain"),
        Some("reach confidential@corp.com"),
    )
    .unwrap();
    crate::ops::add_attachment(
        &mut doc,
        "public.txt",
        b"public notice",
        Some("text/plain"),
        None,
    )
    .unwrap();

    redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &RedactionConfig::default())
        .unwrap();

    assert_eq!(attachment_names(&mut doc), vec!["public.txt".to_string()]);
}

#[test]
fn redaction_keeps_embedded_file_when_the_page_has_no_match() {
    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    crate::ops::add_attachment(
        &mut doc,
        "absent.txt",
        b"absent-token-zz",
        Some("text/plain"),
        None,
    )
    .unwrap();

    let pattern = RedactionPattern::Text {
        query: "absent-token-zz".to_string(),
        case_sensitive: true,
    };
    let summaries =
        redact_document_pattern(&mut doc, &pattern, None, &RedactionConfig::default()).unwrap();

    assert!(summaries.is_empty());
    assert_eq!(attachment_names(&mut doc), vec!["absent.txt".to_string()]);
}

#[test]
fn redaction_ignores_embedded_file_for_a_short_match() {
    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    crate::ops::add_attachment(
        &mut doc,
        "clip.txt",
        b"fi confidential@corp.com",
        Some("text/plain"),
        None,
    )
    .unwrap();

    let pattern = RedactionPattern::Text {
        query: "fi".to_string(),
        case_sensitive: false,
    };
    redact_document_pattern(&mut doc, &pattern, None, &RedactionConfig::default()).unwrap();

    assert_eq!(attachment_names(&mut doc), vec!["clip.txt".to_string()]);
    let kept = crate::ops::extract_attachment(&mut doc, "clip.txt").unwrap();
    assert_eq!(kept.data, b"fi confidential@corp.com");
}

#[test]
fn redaction_removes_embedded_file_when_text_case_does_not_match() {
    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    crate::ops::add_attachment(
        &mut doc,
        "copy.txt",
        b"Confidential copy",
        Some("text/plain"),
        None,
    )
    .unwrap();
    crate::ops::add_attachment(
        &mut doc,
        "public.txt",
        b"public notice",
        Some("text/plain"),
        None,
    )
    .unwrap();

    let pattern = RedactionPattern::Text {
        query: "CONFIDENTIAL".to_string(),
        case_sensitive: false,
    };
    redact_document_pattern(&mut doc, &pattern, None, &RedactionConfig::default()).unwrap();

    assert_eq!(attachment_names(&mut doc), vec!["public.txt".to_string()]);
}

#[test]
fn redaction_keeps_a_neighbor_word_in_another_file() {
    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    crate::ops::add_attachment(&mut doc, "leak.txt", EMAIL.as_bytes(), Some("text/plain"), None)
        .unwrap();
    crate::ops::add_attachment(&mut doc, "neighbor.txt", b"Verified", Some("text/plain"), None)
        .unwrap();

    let mut config = RedactionConfig::default();
    config.padding = 80.0;
    redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &config).unwrap();

    assert_eq!(
        attachment_names(&mut doc),
        vec!["neighbor.txt".to_string()]
    );
}

#[test]
fn region_redaction_removes_embedded_file_with_the_excised_text() {
    let line = "Client Email: confidential@corp.com Verified";
    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    crate::ops::add_attachment(&mut doc, "line.txt", line.as_bytes(), Some("text/plain"), None)
        .unwrap();
    crate::ops::add_attachment(
        &mut doc,
        "other.txt",
        b"other@corp.com",
        Some("text/plain"),
        None,
    )
    .unwrap();

    redact_document_rectangles(
        &mut doc,
        0,
        &[Rect::new(70.0, 690.0, 400.0, 720.0)],
        &RedactionConfig::default(),
    )
    .unwrap();

    assert_eq!(attachment_names(&mut doc), vec!["other.txt".to_string()]);
}

#[test]
fn redaction_removes_embedded_pdf_whose_page_repeats_the_email() {
    let nested = single_page_pdf(octal_literal(EMAIL).as_bytes(), true);
    assert!(
        !bytes_contain(&nested, EMAIL),
        "the fixture must hide the email from a raw byte search"
    );

    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    crate::ops::add_attachment(
        &mut doc,
        "nested.pdf",
        &nested,
        Some("application/pdf"),
        None,
    )
    .unwrap();
    crate::ops::add_attachment(
        &mut doc,
        "other.txt",
        b"other@corp.com",
        Some("text/plain"),
        None,
    )
    .unwrap();

    redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &RedactionConfig::default())
        .unwrap();

    assert_eq!(attachment_names(&mut doc), vec!["other.txt".to_string()]);
}

#[test]
fn redaction_keeps_embedded_pdf_that_only_hides_the_email_inside() {
    let secret = single_page_pdf(octal_literal(EMAIL).as_bytes(), true);
    assert!(!bytes_contain(&secret, EMAIL));
    let hello = single_page_pdf(
        b"BT\n/F1 12 Tf\n1 0 0 1 72 700 Tm\n(Hello only) Tj\nET\n",
        false,
    );
    let mut wrapper = crate::cos::PdfDocument::load(&hello).unwrap();
    crate::ops::add_attachment(
        &mut wrapper,
        "secret.pdf",
        &secret,
        Some("application/pdf"),
        None,
    )
    .unwrap();
    let wrapper_bytes = wrapper.save_to_vec().unwrap();
    assert!(!bytes_contain(&wrapper_bytes, EMAIL));

    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    crate::ops::add_attachment(
        &mut doc,
        "wrapped.pdf",
        &wrapper_bytes,
        Some("application/pdf"),
        None,
    )
    .unwrap();

    redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &RedactionConfig::default())
        .unwrap();

    assert_eq!(attachment_names(&mut doc), vec!["wrapped.pdf".to_string()]);
}

fn field_named(doc: &mut crate::cos::PdfDocument, name: &str) -> crate::forms::FormField {
    crate::forms::extract_document_forms(doc)
        .unwrap()
        .into_iter()
        .find(|field| field.name == name)
        .unwrap_or_else(|| panic!("missing field {name}"))
}

fn text_field(name: &str, value: Option<&str>, y: f64) -> crate::forms::FormFieldCreateOptions {
    crate::forms::FormFieldCreateOptions {
        name: name.to_string(),
        value: value.map(str::to_string),
        rect: Rect::new(72.0, y, 320.0, y + 18.0),
        ..crate::forms::FormFieldCreateOptions::default()
    }
}

fn page_annot_ids(doc: &mut crate::cos::PdfDocument, page_id: crate::cos::object::ObjectId) -> Vec<crate::cos::object::ObjectId> {
    let page = doc.get_object(page_id).unwrap().as_dict().unwrap().clone();
    match page.get("Annots") {
        Some(crate::cos::object::PdfObject::Array(items)) => {
            items.iter().filter_map(|item| item.as_reference()).collect()
        }
        _ => Vec::new(),
    }
}

#[test]
fn redaction_clears_form_value_and_appearance_that_repeat_the_email() {
    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    let page_id = doc.get_pages().unwrap()[0];
    let email = crate::forms::create_form_field(&mut doc, 1, &text_field("email", Some(EMAIL), 520.0)).unwrap();
    crate::forms::create_form_field(&mut doc, 1, &text_field("note", Some("quarterly totals"), 480.0)).unwrap();
    crate::forms::create_form_field(
        &mut doc,
        1,
        &crate::forms::FormFieldCreateOptions {
            name: "agree".to_string(),
            field_type: crate::forms::FormFieldType::Checkbox,
            value: None,
            rect: Rect::new(72.0, 440.0, 96.0, 458.0),
            ..crate::forms::FormFieldCreateOptions::default()
        },
    )
    .unwrap();
    crate::forms::create_form_field(
        &mut doc,
        1,
        &crate::forms::FormFieldCreateOptions {
            name: "country".to_string(),
            field_type: crate::forms::FormFieldType::Choice,
            value: Some("Mexico".to_string()),
            options: Some(vec!["Mexico".to_string(), "Canada".to_string()]),
            rect: Rect::new(72.0, 400.0, 240.0, 418.0),
            ..crate::forms::FormFieldCreateOptions::default()
        },
    )
    .unwrap();

    redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &RedactionConfig::default())
        .unwrap();

    let cleared = field_named(&mut doc, "email");
    assert_eq!(cleared.value, "");
    assert_eq!(cleared.id, email.id);
    let cleared_dict = doc.get_object(cleared.id).unwrap().as_dict().unwrap().clone();
    assert!(!cleared_dict.contains_key("V"));
    assert!(!cleared_dict.contains_key("AP"));
    assert!(doc.objects.contains_key(&cleared.id));
    assert!(page_annot_ids(&mut doc, page_id).contains(&cleared.id));

    assert_eq!(field_named(&mut doc, "note").value, "quarterly totals");
    let agree = field_named(&mut doc, "agree");
    assert_eq!(agree.value, "Off");
    let agree_dict = doc.get_object(agree.id).unwrap().as_dict().unwrap().clone();
    assert!(agree_dict.contains_key("AS"));
    let country = field_named(&mut doc, "country");
    assert_eq!(country.value, "Mexico");
    assert_eq!(country.options, vec!["Mexico".to_string(), "Canada".to_string()]);

    let saved = doc.save_to_vec().unwrap();
    assert!(!bytes_contain(&saved, EMAIL));
}

#[test]
fn redaction_keeps_form_value_when_the_page_has_no_match() {
    let hello = single_page_pdf(
        b"BT\n/F1 12 Tf\n1 0 0 1 72 700 Tm\n(Hello only) Tj\nET\n",
        false,
    );
    let mut doc = crate::cos::PdfDocument::load(&hello).unwrap();
    crate::forms::create_form_field(&mut doc, 1, &text_field("email", Some(EMAIL), 520.0)).unwrap();

    let summaries =
        redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &RedactionConfig::default())
            .unwrap();

    assert!(summaries.is_empty());
    assert_eq!(field_named(&mut doc, "email").value, EMAIL);
    let saved = doc.save_to_vec().unwrap();
    assert!(bytes_contain(&saved, EMAIL));
}

#[test]
fn redaction_keeps_form_value_for_a_short_match() {
    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    crate::forms::create_form_field(
        &mut doc,
        1,
        &text_field("clip", Some("fi confidential@corp.com"), 520.0),
    )
    .unwrap();

    let pattern = RedactionPattern::Text {
        query: "fi".to_string(),
        case_sensitive: false,
    };
    redact_document_pattern(&mut doc, &pattern, None, &RedactionConfig::default()).unwrap();

    assert_eq!(field_named(&mut doc, "clip").value, "fi confidential@corp.com");
}

#[test]
fn redaction_clears_form_value_when_text_case_does_not_match() {
    let page = single_page_pdf(
        b"BT\n/F1 12 Tf\n1 0 0 1 72 700 Tm\n(Reach SECRET team) Tj\nET\n",
        false,
    );
    let mut doc = crate::cos::PdfDocument::load(&page).unwrap();
    crate::forms::create_form_field(&mut doc, 1, &text_field("code", Some("SECRET"), 520.0)).unwrap();
    crate::forms::create_form_field(&mut doc, 1, &text_field("notes", Some("notes"), 480.0)).unwrap();

    let pattern = RedactionPattern::Text {
        query: "secret".to_string(),
        case_sensitive: false,
    };
    redact_document_pattern(&mut doc, &pattern, None, &RedactionConfig::default()).unwrap();

    assert_eq!(field_named(&mut doc, "code").value, "");
    assert_eq!(field_named(&mut doc, "notes").value, "notes");
}

#[test]
fn redaction_keeps_a_neighbor_word_in_a_form_field() {
    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    crate::forms::create_form_field(&mut doc, 1, &text_field("email", Some(EMAIL), 400.0)).unwrap();
    crate::forms::create_form_field(&mut doc, 1, &text_field("status", Some("Verified"), 360.0))
        .unwrap();

    let mut config = RedactionConfig::default();
    config.padding = 80.0;
    redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &config).unwrap();

    assert_eq!(field_named(&mut doc, "email").value, "");
    assert_eq!(field_named(&mut doc, "status").value, "Verified");
}

#[test]
fn region_redaction_clears_form_value_with_the_excised_text() {
    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    let line = "Client Email: confidential@corp.com Verified";
    crate::forms::create_form_field(&mut doc, 1, &text_field("line", Some(line), 400.0)).unwrap();
    crate::forms::create_form_field(&mut doc, 1, &text_field("other", Some("other@corp.com"), 360.0))
        .unwrap();

    redact_document_rectangles(
        &mut doc,
        0,
        &[Rect::new(70.0, 690.0, 400.0, 720.0)],
        &RedactionConfig::default(),
    )
    .unwrap();

    assert_eq!(field_named(&mut doc, "line").value, "");
    assert_eq!(field_named(&mut doc, "other").value, "other@corp.com");
}

#[test]
fn redaction_clears_a_widget_appearance_when_the_value_does_not_repeat() {
    use crate::cos::object::{PdfDictionary, PdfName, PdfObject, PdfStream, PdfString};

    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    let page_id = doc.get_pages().unwrap()[0];
    let acro_id = crate::forms::get_or_create_acroform(&mut doc).unwrap();

    let compressed =
        crate::cos::encode_flate(octal_literal(EMAIL).as_bytes(), crate::cos::Compression::default())
            .unwrap();
    assert!(!bytes_contain(&compressed, EMAIL));
    let stream_id = doc.alloc_object_id();
    let mut stream_dict = PdfDictionary::new();
    stream_dict.insert("Filter", PdfObject::Name(PdfName::new("FlateDecode")));
    stream_dict.insert("Length", PdfObject::Integer(compressed.len() as i64));
    doc.set_object(
        stream_id,
        PdfObject::Stream(PdfStream::new(stream_dict, compressed)),
    );

    let kid_id = doc.alloc_object_id();
    let mut kid = PdfDictionary::new();
    kid.insert("Subtype", PdfObject::Name(PdfName::new("Widget")));
    kid.insert("Rect", PdfObject::Array(vec![
        PdfObject::Real(72.0),
        PdfObject::Real(400.0),
        PdfObject::Real(320.0),
        PdfObject::Real(418.0),
    ]));
    kid.insert("P", PdfObject::Reference(page_id));
    let mut appearance = PdfDictionary::new();
    appearance.insert("N", PdfObject::Reference(stream_id));
    kid.insert("AP", PdfObject::Dictionary(appearance));
    doc.set_object(kid_id, PdfObject::Dictionary(kid));

    let parent_id = doc.alloc_object_id();
    let mut parent = PdfDictionary::new();
    parent.insert("T", PdfObject::String(PdfString::literal(b"Email".to_vec())));
    parent.insert("FT", PdfObject::Name(PdfName::new("Tx")));
    parent.insert("V", PdfObject::String(PdfString::literal(b"ok".to_vec())));
    parent.insert("Kids", PdfObject::Array(vec![PdfObject::Reference(kid_id)]));
    doc.set_object(parent_id, PdfObject::Dictionary(parent));

    let mut acro = match doc.get_object(acro_id).unwrap() {
        PdfObject::Dictionary(dict) => dict,
        _ => panic!("acroform"),
    };
    let mut fields = match acro.remove("Fields") {
        Some(PdfObject::Array(items)) => items,
        _ => Vec::new(),
    };
    fields.push(PdfObject::Reference(parent_id));
    acro.insert("Fields", PdfObject::Array(fields));
    doc.set_object(acro_id, PdfObject::Dictionary(acro));

    let mut page = match doc.get_object(page_id).unwrap() {
        PdfObject::Dictionary(dict) => dict,
        _ => panic!("page"),
    };
    let mut annots = match page.remove("Annots") {
        Some(PdfObject::Array(items)) => items,
        _ => Vec::new(),
    };
    annots.push(PdfObject::Reference(kid_id));
    page.insert("Annots", PdfObject::Array(annots));
    doc.set_object(page_id, PdfObject::Dictionary(page));

    assert_eq!(field_named(&mut doc, "Email").value, "ok");

    redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &RedactionConfig::default())
        .unwrap();

    assert_eq!(field_named(&mut doc, "Email").value, "");
    assert!(doc.objects.contains_key(&parent_id));
    assert!(doc.objects.contains_key(&kid_id));
    assert!(!doc.objects.contains_key(&stream_id));
    assert!(!doc.xref.entries.contains_key(&stream_id));
}

#[test]
fn redaction_drops_an_indirect_form_value_that_repeats_the_email() {
    use crate::cos::object::{PdfObject, PdfString};

    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    let created =
        crate::forms::create_form_field(&mut doc, 1, &text_field("email", None, 520.0)).unwrap();
    let value_id = doc.alloc_object_id();
    doc.set_object(
        value_id,
        PdfObject::String(PdfString::literal(EMAIL.as_bytes().to_vec())),
    );
    let mut field = match doc.get_object(created.id).unwrap() {
        PdfObject::Dictionary(dict) => dict,
        _ => panic!("field"),
    };
    field.insert("V", PdfObject::Reference(value_id));
    doc.set_object(created.id, PdfObject::Dictionary(field));
    assert_eq!(field_named(&mut doc, "email").value, "");
    assert!(bytes_contain(&doc.save_to_vec().unwrap(), EMAIL));

    redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &RedactionConfig::default())
        .unwrap();

    assert!(!doc.objects.contains_key(&value_id));
    assert!(!doc.xref.entries.contains_key(&value_id));
    assert!(!bytes_contain(&doc.save_to_vec().unwrap(), EMAIL));
}

#[test]
fn redaction_keeps_the_field_name_default_and_option_list() {
    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    crate::forms::create_form_field(&mut doc, 1, &text_field(EMAIL, Some("kept"), 520.0)).unwrap();
    crate::forms::create_form_field(
        &mut doc,
        1,
        &crate::forms::FormFieldCreateOptions {
            name: "backup".to_string(),
            value: Some("quarterly totals".to_string()),
            default_value: Some(EMAIL.to_string()),
            rect: Rect::new(72.0, 480.0, 320.0, 498.0),
            ..crate::forms::FormFieldCreateOptions::default()
        },
    )
    .unwrap();
    crate::forms::create_form_field(
        &mut doc,
        1,
        &crate::forms::FormFieldCreateOptions {
            name: "country".to_string(),
            field_type: crate::forms::FormFieldType::Choice,
            value: Some("Mexico".to_string()),
            options: Some(vec![EMAIL.to_string(), "Mexico".to_string()]),
            rect: Rect::new(72.0, 440.0, 320.0, 458.0),
            ..crate::forms::FormFieldCreateOptions::default()
        },
    )
    .unwrap();

    redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &RedactionConfig::default())
        .unwrap();

    let named = field_named(&mut doc, EMAIL);
    assert_eq!(named.value, "kept");
    let backup = field_named(&mut doc, "backup");
    assert_eq!(backup.value, "quarterly totals");
    assert_eq!(backup.default_value.as_deref(), Some(EMAIL));
    let country = field_named(&mut doc, "country");
    assert_eq!(country.value, "Mexico");
    assert!(country.options.iter().any(|option| option == EMAIL));
}

#[test]
fn redaction_keeps_an_appearance_whose_filter_cannot_be_decoded() {
    use crate::cos::object::{PdfDictionary, PdfName, PdfObject, PdfStream};

    let mut doc = crate::cos::PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    let created =
        crate::forms::create_form_field(&mut doc, 1, &text_field("note", Some("kept-note"), 400.0))
            .unwrap();
    let raw = format!("BT /F1 12 Tf ({EMAIL}) Tj ET").into_bytes();
    let stream_id = doc.alloc_object_id();
    let mut stream_dict = PdfDictionary::new();
    stream_dict.insert("Filter", PdfObject::Name(PdfName::new("LZWDecode")));
    stream_dict.insert("Length", PdfObject::Integer(raw.len() as i64));
    doc.set_object(stream_id, PdfObject::Stream(PdfStream::new(stream_dict, raw)));

    let mut field = match doc.get_object(created.id).unwrap() {
        PdfObject::Dictionary(dict) => dict,
        _ => panic!("field"),
    };
    let mut appearance = PdfDictionary::new();
    appearance.insert("N", PdfObject::Reference(stream_id));
    field.insert("AP", PdfObject::Dictionary(appearance));
    doc.set_object(created.id, PdfObject::Dictionary(field));

    redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &RedactionConfig::default())
        .unwrap();

    assert_eq!(field_named(&mut doc, "note").value, "kept-note");
    let kept = doc.get_object(created.id).unwrap().as_dict().unwrap().clone();
    assert!(kept.contains_key("AP"));
    assert!(doc.objects.contains_key(&stream_id));
    assert!(doc.xref.entries.contains_key(&stream_id));
    assert!(bytes_contain(&doc.save_to_vec().unwrap(), EMAIL));
}

const RGB_4X2: &str = "/Type /XObject /Subtype /Image /Width 4 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8";
const GRAY_4X2: &str =
    "/Type /XObject /Subtype /Image /Width 4 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8";

fn rgb_4x2_samples() -> Vec<u8> {
    vec![
        10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200,
        210, 220, 230, 240,
    ]
}

fn assemble_pdf(bodies: &[Vec<u8>]) -> Vec<u8> {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets = Vec::with_capacity(bodies.len());
    for (index, body) in bodies.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(body);
        if !body.ends_with(b"\n") {
            pdf.push(b'\n');
        }
        pdf.extend_from_slice(b"endobj\n");
    }
    let xref_offset = pdf.len();
    let size = bodies.len() + 1;
    pdf.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n")
            .as_bytes(),
    );
    pdf
}

fn stream_body(dict_tail: &str, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    let gap = if dict_tail.is_empty() { "" } else { " " };
    body.extend_from_slice(
        format!("<< {dict_tail}{gap}/Length {} >>\nstream\n", data.len()).as_bytes(),
    );
    body.extend_from_slice(data);
    body.extend_from_slice(b"endstream\n");
    body
}

fn page_body(image_id: u32, contents_id: u32) -> Vec<u8> {
    format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im1 {image_id} 0 R >> >> /Contents {contents_id} 0 R >>\n"
    )
    .into_bytes()
}

fn one_image_pdf(content: &str, image_dict: &str, samples: &[u8]) -> Vec<u8> {
    assemble_pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>\n".to_vec(),
        b"<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>\n".to_vec(),
        page_body(5, 4),
        stream_body("", content.as_bytes()),
        stream_body(image_dict, samples),
    ])
}

fn two_page_image_pdf(content: &str, image_dict: &str, samples: &[u8]) -> Vec<u8> {
    assemble_pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>\n".to_vec(),
        b"<< /Type /Pages /Kids [ 3 0 R 4 0 R ] /Count 2 >>\n".to_vec(),
        page_body(7, 5),
        page_body(7, 6),
        stream_body("", content.as_bytes()),
        stream_body("", content.as_bytes()),
        stream_body(image_dict, samples),
    ])
}

fn image_paint(width: &str, height: &str, x: &str, y: &str) -> String {
    format!("q\n{width} 0 0 {height} {x} {y} cm\n/Im1 Do\nQ\n")
}

fn slice_in(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|window| window == needle)
}

fn image_stream(doc: &mut PdfDocument, id: ObjectId) -> crate::cos::object::PdfStream {
    match doc.get_object(id).unwrap() {
        PdfObject::Stream(stream) => stream,
        _ => panic!("image object is not a stream"),
    }
}

fn decoded_samples(doc: &mut PdfDocument, id: ObjectId) -> Vec<u8> {
    let stream = image_stream(doc, id);
    let filter = stream
        .dict
        .get("Filter")
        .and_then(|item| item.as_name())
        .unwrap_or("")
        .trim_start_matches('/')
        .to_string();
    crate::cos::decode_stream(&filter, None, &stream.content, &doc.limits).unwrap()
}

fn reload(doc: &mut PdfDocument) -> PdfDocument {
    PdfDocument::load(&doc.save_to_vec().unwrap()).unwrap()
}

fn redact_box(doc: &mut PdfDocument, page: usize, rect: Rect) {
    redact_page(
        doc,
        page,
        &[RedactionRect::new(rect)],
        &RedactionConfig::default(),
    )
    .unwrap();
}

fn assert_rgb_left_cleared(samples: &[u8]) {
    assert_eq!(
        samples,
        &[
            0, 0, 0, 0, 0, 0, 70, 80, 90, 100, 110, 120, 0, 0, 0, 0, 0, 0, 190, 200, 210, 220,
            230, 240,
        ]
    );
}

#[test]
fn redaction_destroys_image_samples_inside_a_rectangle() {
    let raw = rgb_4x2_samples();
    let mut doc = PdfDocument::load(&one_image_pdf(
        &image_paint("4", "2", "100", "200"),
        RGB_4X2,
        &raw,
    ))
    .unwrap();
    redact_box(&mut doc, 0, Rect::new(100.0, 200.0, 102.0, 202.0));
    let saved = doc.save_to_vec().unwrap();
    assert!(!slice_in(&saved, &raw));
    let mut doc = PdfDocument::load(&saved).unwrap();
    let stream = image_stream(&mut doc, ObjectId::new(5));
    assert_eq!(
        stream.dict.get("Filter").and_then(|item| item.as_name()),
        Some("FlateDecode")
    );
    assert_eq!(
        stream.dict.get("ColorSpace").and_then(|item| item.as_name()),
        Some("DeviceRGB")
    );
    assert_rgb_left_cleared(&decoded_samples(&mut doc, ObjectId::new(5)));
    let page = doc.get_pages().unwrap()[0];
    let images = extract_page_images(&mut doc, page).unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].resource_name, "Im1");
    assert_eq!(images[0].object_id, ObjectId::new(5));
    assert_eq!(images[0].width_px, 4);
    assert_eq!(images[0].height_px, 2);
}

#[test]
fn redaction_leaves_an_image_that_misses_the_rectangle() {
    let raw = rgb_4x2_samples();
    let mut doc = PdfDocument::load(&one_image_pdf(
        &image_paint("4", "2", "100", "200"),
        RGB_4X2,
        &raw,
    ))
    .unwrap();
    let before = image_stream(&mut doc, ObjectId::new(5)).content;
    redact_box(&mut doc, 0, Rect::new(0.0, 0.0, 10.0, 10.0));
    assert_eq!(image_stream(&mut doc, ObjectId::new(5)).content, before);
}

#[test]
fn redaction_leaves_image_samples_when_no_rectangle_is_applied() {
    let raw = rgb_4x2_samples();
    let mut doc = PdfDocument::load(&one_image_pdf(
        &image_paint("4", "2", "100", "200"),
        RGB_4X2,
        &raw,
    ))
    .unwrap();
    let before = image_stream(&mut doc, ObjectId::new(5)).content;
    redact_document_pattern(
        &mut doc,
        &RedactionPattern::Email,
        None,
        &RedactionConfig::default(),
    )
    .unwrap();
    redact_page(&mut doc, 0, &[], &RedactionConfig::default()).unwrap();
    assert_eq!(image_stream(&mut doc, ObjectId::new(5)).content, before);
}

#[test]
fn redaction_leaves_an_image_whose_filter_cannot_be_decoded() {
    let marker = b"KEEP-LZW-IMAGE-BYTES";
    let mut doc = PdfDocument::load(&one_image_pdf(
        &image_paint("4", "2", "100", "200"),
        &format!("{RGB_4X2} /Filter /LZWDecode"),
        marker,
    ))
    .unwrap();
    redact_box(&mut doc, 0, Rect::new(100.0, 200.0, 102.0, 202.0));
    let stream = image_stream(&mut doc, ObjectId::new(5));
    assert_eq!(stream.content, marker);
    assert_eq!(
        stream.dict.get("Filter").and_then(|item| item.as_name()),
        Some("LZWDecode")
    );
}

#[test]
fn redaction_leaves_a_filter_array_and_a_non_identity_decode() {
    let marker = b"KEEP-ARRAY-FILTER";
    let mut array_doc = PdfDocument::load(&one_image_pdf(
        &image_paint("4", "2", "100", "200"),
        &format!("{RGB_4X2} /Filter [/FlateDecode]"),
        marker,
    ))
    .unwrap();
    redact_box(
        &mut array_doc,
        0,
        Rect::new(100.0, 200.0, 102.0, 202.0),
    );
    let stream = image_stream(&mut array_doc, ObjectId::new(5));
    assert_eq!(stream.content, marker);
    assert!(matches!(stream.dict.get("Filter"), Some(PdfObject::Array(_))));

    let raw = rgb_4x2_samples();
    let mut decode_doc = PdfDocument::load(&one_image_pdf(
        &image_paint("4", "2", "100", "200"),
        &format!("{RGB_4X2} /Decode [1 0 1 0 1 0]"),
        &raw,
    ))
    .unwrap();
    redact_box(
        &mut decode_doc,
        0,
        Rect::new(100.0, 200.0, 102.0, 202.0),
    );
    let stream = image_stream(&mut decode_doc, ObjectId::new(5));
    assert_eq!(stream.content, raw);
    assert!(stream.dict.contains_key("Decode"));
}

#[test]
fn redaction_splits_a_shared_image_when_one_placement_meets_the_rectangle() {
    let raw = rgb_4x2_samples();
    let content = format!(
        "{}{}",
        image_paint("4", "2", "100", "200"),
        image_paint("4", "2", "400", "200")
    );
    let mut doc = PdfDocument::load(&one_image_pdf(&content, RGB_4X2, &raw)).unwrap();
    redact_box(&mut doc, 0, Rect::new(100.0, 200.0, 102.0, 202.0));
    let page = doc.get_pages().unwrap()[0];
    let images = extract_page_images(&mut doc, page).unwrap();
    let left = images
        .iter()
        .find(|image| image.bbox.min_x < 150.0 && image.bbox.width() > 0.0)
        .unwrap();
    let right = images
        .iter()
        .find(|image| image.bbox.min_x > 300.0 && image.bbox.width() > 0.0)
        .unwrap();
    assert_ne!(left.object_id, ObjectId::new(5));
    assert_eq!(right.object_id, ObjectId::new(5));
    assert_eq!(image_stream(&mut doc, ObjectId::new(5)).content, raw);
    assert_rgb_left_cleared(&decoded_samples(&mut doc, left.object_id));
    assert!(doc.objects.contains_key(&ObjectId::new(5)));
}

#[test]
fn redaction_drops_a_shared_image_when_every_placement_is_retargeted() {
    let raw = rgb_4x2_samples();
    let content = format!(
        "{}{}",
        image_paint("4", "2", "100", "200"),
        image_paint("4", "2", "300", "200")
    );
    let mut doc = PdfDocument::load(&one_image_pdf(&content, RGB_4X2, &raw)).unwrap();
    redact_box(&mut doc, 0, Rect::new(90.0, 190.0, 320.0, 210.0));
    assert!(!doc.objects.contains_key(&ObjectId::new(5)));
    assert!(!doc.xref.entries.contains_key(&ObjectId::new(5)));
    let page = doc.get_pages().unwrap()[0];
    let images = extract_page_images(&mut doc, page).unwrap();
    assert!(images.iter().all(|image| image.object_id != ObjectId::new(5)));
    let painted: Vec<_> = images
        .iter()
        .filter(|image| image.bbox.width() > 0.0 || image.bbox.height() > 0.0)
        .collect();
    assert_eq!(painted.len(), 2);
    let saved = doc.save_to_vec().unwrap();
    assert!(!slice_in(&saved, &raw));
}

#[test]
fn redaction_on_one_page_keeps_the_image_for_the_other_page() {
    let raw = rgb_4x2_samples();
    let mut doc = PdfDocument::load(&two_page_image_pdf(
        &image_paint("4", "2", "100", "200"),
        RGB_4X2,
        &raw,
    ))
    .unwrap();
    redact_document_rectangles(
        &mut doc,
        0,
        &[Rect::new(90.0, 190.0, 110.0, 210.0)],
        &RedactionConfig::default(),
    )
    .unwrap();
    let pages = doc.get_pages().unwrap();
    let first = extract_page_images(&mut doc, pages[0]).unwrap();
    assert_eq!(first.len(), 1);
    assert!(first.iter().all(|image| image.object_id != ObjectId::new(7)));
    assert!(first[0].bbox.width() > 0.0);
    assert!(decoded_samples(&mut doc, first[0].object_id)
        .iter()
        .all(|byte| *byte == 0));
    let second = extract_page_images(&mut doc, pages[1]).unwrap();
    assert!(second.iter().any(|image| image.object_id == ObjectId::new(7)));
    assert_eq!(image_stream(&mut doc, ObjectId::new(7)).content, raw);
}

#[test]
fn redaction_blacks_out_scan_pixels_under_invisible_text() {
    let samples = vec![255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0];
    let content = "\
q
93 0 0 15 70.5 698.5 cm
/Im1 Do
Q
BT
/F1 12 Tf
3 Tr
1 0 0 1 72 700 Tm
(secret@corp.com) Tj
ET
";
    let mut doc = PdfDocument::load(&one_image_pdf(
        content,
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8",
        &samples,
    ))
    .unwrap();
    redact_document_pattern(
        &mut doc,
        &RedactionPattern::Email,
        None,
        &RedactionConfig::default(),
    )
    .unwrap();
    let saved = doc.save_to_vec().unwrap();
    assert!(!bytes_contain(&saved, "secret@corp.com"));
    let mut doc = PdfDocument::load(&saved).unwrap();
    let stream = image_stream(&mut doc, ObjectId::new(5));
    assert_eq!(
        stream.dict.get("Filter").and_then(|item| item.as_name()),
        Some("FlateDecode")
    );
    assert_eq!(decoded_samples(&mut doc, ObjectId::new(5)), vec![0; 12]);
}

#[test]
fn redaction_rewrites_a_jpeg_as_flate_and_keeps_gray_outside_samples() {
    let mut pixels = vec![0u8; 8 * 8 * 3];
    for iy in 0..8 {
        for ix in 0..8 {
            let offset = (iy * 8 + ix) * 3;
            if ix == 0 {
                pixels[offset] = 255;
            } else if ix == 7 {
                pixels[offset + 1] = 255;
            } else {
                pixels[offset + 2] = 40;
            }
        }
    }
    let mut cursor = std::io::Cursor::new(Vec::new());
    jpeg_encoder::Encoder::new(&mut cursor, 90)
        .encode(&pixels, 8, 8, jpeg_encoder::ColorType::Rgb)
        .unwrap();
    let jpeg = cursor.into_inner();
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(jpeg.clone()));
    let before = decoder.decode().unwrap();
    let info = decoder.info().unwrap();
    assert_eq!((info.width, info.height), (8, 8));
    let right = before[21..24].to_vec();
    assert_ne!(right, vec![0, 0, 0]);

    let mut doc = PdfDocument::load(&one_image_pdf(
        &image_paint("8", "8", "100", "200"),
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode",
        &jpeg,
    ))
    .unwrap();
    redact_box(&mut doc, 0, Rect::new(100.0, 200.0, 102.0, 208.0));
    let saved = doc.save_to_vec().unwrap();
    assert!(!slice_in(&saved, &jpeg));
    let mut doc = PdfDocument::load(&saved).unwrap();
    let stream = image_stream(&mut doc, ObjectId::new(5));
    assert_eq!(
        stream.dict.get("Filter").and_then(|item| item.as_name()),
        Some("FlateDecode")
    );
    assert_eq!(
        stream.dict.get("ColorSpace").and_then(|item| item.as_name()),
        Some("DeviceRGB")
    );
    let samples = decoded_samples(&mut doc, ObjectId::new(5));
    assert_eq!(&samples[0..3], &[0, 0, 0]);
    assert_eq!(&samples[21..24], right.as_slice());

    let gray = vec![10u8, 40, 70, 100, 130, 160, 190, 220];
    let mut doc = PdfDocument::load(&one_image_pdf(
        &image_paint("4", "2", "100", "200"),
        GRAY_4X2,
        &gray,
    ))
    .unwrap();
    redact_box(&mut doc, 0, Rect::new(100.0, 200.0, 102.0, 202.0));
    let mut doc = reload(&mut doc);
    let stream = image_stream(&mut doc, ObjectId::new(5));
    assert_eq!(
        stream.dict.get("ColorSpace").and_then(|item| item.as_name()),
        Some("DeviceGray")
    );
    assert_eq!(
        decoded_samples(&mut doc, ObjectId::new(5)),
        vec![0, 0, 70, 100, 0, 0, 190, 220]
    );
}
