//! Security, Hardening, and Stress Test Suite (ISO 32000 Conformance).
//!
//! Validates engine resilience under adversarial conditions:
//! 1. Decompression bomb / zip bomb mitigation.
//! 2. Circular reference cycle detection and bounded recursion.
//! 3. High-density document stress with multi-paragraph layout reconstruction.
//! 4. Deep graphics state nesting (`q ... Q` stack depth).
//! 5. Complex typographic ligatures and Unicode edge cases.

use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;

use pdf_engine_core::cos::filters::decode_flate;
use pdf_engine_core::cos::{ObjectId, PdfDocument};
use pdf_engine_core::editor::SurgicalEditor;
use pdf_engine_core::error::PdfError;
use pdf_engine_core::fonts::FontMetrics;
use pdf_engine_core::layout::{LayoutReconstructor, TextAlignment};
use pdf_engine_core::security::SecurityLimits;
use pdf_engine_core::stream::{
    build_ast_from_operations, serialize_ast, ContentStreamTokenizer, GraphicsStateStack, Matrix,
};

#[test]
fn test_zip_bomb_bounded_memory_guard() {
    // Generate a 1 MB payload of repeating zeroes
    let uncompressed_payload = vec![0u8; 1024 * 1024];

    // Compress into a tiny zlib byte sequence (~1 KB)
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&uncompressed_payload).unwrap();
    let compressed_bytes = encoder.finish().unwrap();

    assert!(
        compressed_bytes.len() < 2048,
        "Zlib payload should be highly compressed"
    );

    // Configure strict security limits: max 64 KB decompressed
    let limits = SecurityLimits {
        max_stream_decompressed_bytes: 64 * 1024, // 64 KiB ceiling
        ..SecurityLimits::default()
    };

    // Attempting decompression must fail safely without OOM
    let result = decode_flate(&compressed_bytes, None, &limits);
    match result {
        Err(PdfError::SecurityLimitExceeded(msg)) => {
            assert!(
                msg.contains("exceeds maximum allowable limit"),
                "Expected size limit violation, got: {}",
                msg
            );
        }
        other => panic!("Expected SecurityLimitExceeded error, got: {:?}", other),
    }

    // Configure strict ratio limit: 10:1 ratio
    let ratio_limits = SecurityLimits {
        max_stream_decompressed_bytes: 10 * 1024 * 1024, // High ceiling
        max_decompression_ratio: 10,                     // Strict 10:1 ratio
        ..SecurityLimits::default()
    };

    let ratio_result = decode_flate(&compressed_bytes, None, &ratio_limits);
    match ratio_result {
        Err(PdfError::SecurityLimitExceeded(msg)) => {
            assert!(
                msg.contains("expansion ratio"),
                "Expected ratio violation, got: {}",
                msg
            );
        }
        other => panic!(
            "Expected SecurityLimitExceeded ratio error, got: {:?}",
            other
        ),
    }
}

#[test]
fn test_circular_object_reference_cycle_detection() {
    // Construct a malicious PDF where the /Pages tree contains an immediate cycle:
    // Object 2 (/Pages) references Kids [ 2 0 R ] (self-referencing loop)
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n");

    let off1 = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    let off2 = pdf.len();
    // Circular self-reference: Kids includes Object 2 itself
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [ 2 0 R ] /Count 1 >>\nendobj\n");

    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n0 3\n0000000000 65535 f \n");
    pdf.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());

    pdf.extend_from_slice(b"trailer\n<< /Size 3 /Root 1 0 R >>\n");
    pdf.extend_from_slice(format!("startxref\n{}\n%%EOF\n", xref_offset).as_bytes());

    let mut doc = PdfDocument::load(&pdf).expect("Document should parse initial structures");

    // Traversal must catch the cycle and abort cleanly without hanging or stack overflow
    let pages_result = doc.get_pages();
    match pages_result {
        Err(PdfError::CircularReference { id, .. }) => {
            assert_eq!(id, 2, "Should identify object 2 as the cyclic node");
        }
        other => panic!("Expected CircularReference error, got: {:?}", other),
    }
}

#[test]
fn test_deep_graphics_state_stack_stress() {
    // Stress test the graphics state stack with 128 nested `q` (save) operations
    let mut stream_ops = Vec::new();

    for _i in 0..128 {
        // q followed by an affine translation
        stream_ops.extend_from_slice(b"q\n1 0 0 1 10 10 cm\n");
    }

    // Balancing 128 `Q` (restore) operations
    for _ in 0..128 {
        stream_ops.extend_from_slice(b"Q\n");
    }

    let mut tokenizer = ContentStreamTokenizer::new(&stream_ops);
    let ops = tokenizer
        .tokenize_all()
        .expect("Tokenization should succeed");
    assert_eq!(ops.len(), 128 * 3);

    let mut state_stack = GraphicsStateStack::new();

    for op in &ops {
        match op.operator.as_str() {
            "q" => state_stack.push(),
            "Q" => {
                let _ = state_stack.pop();
            }
            "cm" => {
                state_stack
                    .current
                    .concat_matrix(&Matrix::translation(10.0, 10.0));
            }
            _ => {}
        }
    }

    // After 128 pops, state should be back to root identity
    assert_eq!(state_stack.current.ctm, Matrix::identity());
}

#[test]
fn test_multi_column_dense_document_layout_and_surgical_edits() {
    // Generate a high-density, multi-column document stream with 20 distinct paragraph blocks
    let mut stream_bytes = Vec::new();
    let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);

    for row in 0..20 {
        let y_pos = 2000.0 - (row as f64 * 80.0);
        stream_bytes.extend_from_slice(
            format!(
                "BT\n/F1 10 Tf\n50 {} Tm\n(Section Record #{} - Financial Accounting Specification) Tj\n0 -14 Td\n(Transaction verified and recorded in persistent ledger state.) Tj\nET\n",
                y_pos, row + 1
            ).as_bytes()
        );
    }

    let mut tokenizer = ContentStreamTokenizer::new(&stream_bytes);
    let ops = tokenizer.tokenize_all().expect("Tokenize dense stream");
    let mut ast = build_ast_from_operations(ops);

    let reconstructor = LayoutReconstructor::new(&ast).with_font("F1", metrics.clone());
    let paragraphs = reconstructor.reconstruct().unwrap();

    assert_eq!(
        paragraphs.len(),
        20,
        "Should reconstruct all 20 distinct multi-column paragraph blocks"
    );

    // Verify first paragraph
    assert!(paragraphs[0].text().contains("Section Record #1"));
    assert_eq!(paragraphs[0].alignment, TextAlignment::Left);

    // Surgically edit paragraph #5 and paragraph #15
    let target_5 = paragraphs[5].clone();
    SurgicalEditor::edit_paragraph(
        &mut ast,
        &target_5,
        "MODIFIED: Financial record #6 successfully audited and approved.",
        &metrics,
    )
    .expect("Surgical edit on paragraph #5 must succeed");

    let target_15 = paragraphs[15].clone();
    SurgicalEditor::edit_paragraph(
        &mut ast,
        &target_15,
        "MODIFIED: Column 2 financial settlement finalized with legal verification.",
        &metrics,
    )
    .expect("Surgical edit on paragraph #15 must succeed");

    // Reconstruct layout after mutations
    let re_reconstructor = LayoutReconstructor::new(&ast).with_font("F1", metrics.clone());
    let re_paragraphs = re_reconstructor.reconstruct().unwrap();

    assert_eq!(
        re_paragraphs.len(),
        20,
        "Block count must remain exactly 20"
    );
    assert!(re_paragraphs[5]
        .text()
        .contains("Financial record #6 successfully audited"));
    assert!(re_paragraphs[15]
        .text()
        .contains("Column 2 financial settlement finalized"));

    // Verify untouched paragraphs are 100% identical
    assert_eq!(re_paragraphs[0].text(), paragraphs[0].text());
    assert_eq!(re_paragraphs[1].text(), paragraphs[1].text());
    assert_eq!(re_paragraphs[19].text(), paragraphs[19].text());

    // Serialize mutated AST and verify roundtrip integrity
    let out_bytes = serialize_ast(&ast);
    assert!(!out_bytes.is_empty());
    assert!(out_bytes
        .windows(b"MODIFIED: Financial record #6".len())
        .any(|w| w == b"MODIFIED: Financial record #6"));
}

#[test]
fn test_corrupted_xref_error_handling() {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n");

    let _off1 = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog >>\nendobj\n");

    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n0 2\n0000000000 65535 f \n");
    // Purposely corrupted offset pointing beyond document bounds (9999999999)
    pdf.extend_from_slice(b"9999999999 00000 n \n");

    pdf.extend_from_slice(b"trailer\n<< /Size 2 /Root 1 0 R >>\n");
    pdf.extend_from_slice(format!("startxref\n{}\n%%EOF\n", xref_offset).as_bytes());

    let mut doc = PdfDocument::load(&pdf).expect("Document header and xref table should load");

    // Resolving the corrupted object ID must return a typed PdfError, never panic
    let result = doc.get_object(ObjectId::new(1));
    assert!(result.is_err(), "Expected error for out-of-bounds offset");
    match result {
        Err(PdfError::InvalidXRef { .. }) | Err(PdfError::ParseError { .. }) => {
            // Success: safely handled
        }
        other => panic!("Expected InvalidXRef or ParseError, got: {:?}", other),
    }
}

#[test]
fn test_font_encoder_and_glyph_fallback_editing() {
    use pdf_engine_core::fonts::FontEncoder;

    let encoder = FontEncoder::new();

    // 1. Test encoding standard Spanish / European characters in WinAnsi
    let spanish_text = "Señor López: Garantía de café & crédito (€50)";
    let encoded_bytes = encoder.encode_string(spanish_text);
    assert!(!encoded_bytes.is_empty());

    // Verify ñ (0xF1), ó (0xF3), í (0xED), é (0xE9), € (0x80)
    assert!(encoded_bytes.contains(&0xF1)); // ñ
    assert!(encoded_bytes.contains(&0xF3)); // ó
    assert!(encoded_bytes.contains(&0xED)); // í
    assert!(encoded_bytes.contains(&0xE9)); // é
    assert!(encoded_bytes.contains(&0x80)); // €

    // 2. Test fallback for unsupported symbols / scripts (e.g. Cyrillic/Greek when font is Latin-1)
    let mixed_text = "Report: Москва / Café";
    let fallback_bytes = encoder.encode_string(mixed_text);
    // Transliterate should replace missing Cyrillic with '?'
    let fallback_str = String::from_utf8_lossy(&fallback_bytes);
    assert!(fallback_str.contains("??????"));
    assert!(fallback_bytes.contains(&0xE9)); // 'é' still cleanly preserved!

    // 3. Test surgical editor with encoder
    let stream_source = b"BT\n/F1 12 Tf\n50 700 Tm\n(Original Draft Agreement) Tj\nET\n";
    let mut tokenizer = ContentStreamTokenizer::new(stream_source);
    let ops = tokenizer.tokenize_all().unwrap();
    let mut ast = build_ast_from_operations(ops);

    let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
    let reconstructor = LayoutReconstructor::new(&ast).with_font("F1", metrics.clone());
    let paragraphs = reconstructor.reconstruct().unwrap();
    assert_eq!(paragraphs.len(), 1);

    let edit_result = SurgicalEditor::edit_paragraph_with_encoder(
        &mut ast,
        &paragraphs[0],
        "Cláusula de confidencialidad para el año 2026.",
        &metrics,
        &encoder,
    );
    assert!(
        edit_result.is_ok(),
        "Surgical edit with FontEncoder must succeed"
    );

    let serialized = serialize_ast(&ast);
    assert!(!serialized.is_empty());
    // Verify that the serialized content stream contains valid PDF syntax
    assert!(serialized.starts_with(b"BT\n"));
    assert!(serialized.ends_with(b"ET\n"));
}
