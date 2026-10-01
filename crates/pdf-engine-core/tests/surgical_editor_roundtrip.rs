use pdf_engine_core::editor::SurgicalEditor;
use pdf_engine_core::fonts::FontMetrics;
use pdf_engine_core::layout::LayoutReconstructor;
use pdf_engine_core::stream::{
    build_ast_from_operations, serialize_ast, ContentStreamTokenizer,
};

#[test]
fn test_surgical_paragraph_replacement_and_reflow() {
    let raw_stream = b"
0.9 0.9 0.9 rg
0 0 500 50 re
f
q
1 0 0 1 0 0 cm
/Im1 Do
Q
BT
/F1 12 Tf
50 700 Tm
(The original agreement shall remain in full force) Tj
0 -14 Td
(and effect unless modified in writing.) Tj
ET
BT
/F1 9 Tf
50 50 Tm
(Page 1 of 1) Tj
ET
";

    // 1. Parse into Content AST
    let mut tokenizer = ContentStreamTokenizer::new(raw_stream);
    let ops = tokenizer.tokenize_all().expect("Failed to tokenize");
    let mut ast = build_ast_from_operations(ops);

    // Verify initial AST has 4 top-level nodes:
    // 0: color rg
    // 1: rect re
    // 2: fill f
    // 3: graphics group q ... Q (/Im1 Do)
    // 4: TextBlock (agreement)
    // 5: TextBlock (footer)
    assert_eq!(ast.nodes.len(), 6);

    // 2. Reconstruct layout
    let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
    let reconstructor = LayoutReconstructor::new(&ast).with_font("F1", metrics.clone());
    let paragraphs = reconstructor.reconstruct();

    assert_eq!(paragraphs.len(), 2);
    assert!(paragraphs[0].text().contains("The original agreement"));
    assert!(paragraphs[1].text().contains("Page 1 of 1"));

    // 3. Surgically edit paragraph 0 in place
    let replacement_text = "The amended agreement has been executed and confirmed by all parties.";
    SurgicalEditor::edit_paragraph(&mut ast, &paragraphs[0], replacement_text, &metrics)
        .expect("Surgical edit failed");

    // 4. Verify AST structure after mutation
    // Node count must still be exactly 6 (non-text nodes and footer untouched)
    assert_eq!(ast.nodes.len(), 6);

    // 5. Serialize AST and verify lossless preservation
    let serialized = serialize_ast(&ast);
    let output_str = String::from_utf8_lossy(&serialized);

    // Non-text vector graphics must be 100% intact
    assert!(output_str.contains("0 0 500 50 re\n"));
    assert!(output_str.contains("f\n"));
    assert!(output_str.contains("/Im1 Do\n"));

    // Footer text block must be 100% intact
    assert!(output_str.contains("(Page 1 of 1) Tj\n"));

    // Old agreement text must NOT exist
    assert!(!output_str.contains("The original agreement"));

    // New replacement text must exist in the stream
    assert!(output_str.contains("The amended agreement"));

    // 6. Re-run LayoutReconstructor on mutated AST to verify that the new paragraph parses correctly
    let reconstructor_after = LayoutReconstructor::new(&ast).with_font("F1", metrics);
    let paragraphs_after = reconstructor_after.reconstruct();

    assert_eq!(paragraphs_after.len(), 2);
    assert!(paragraphs_after[0].text().contains("The amended agreement"));
    assert!(paragraphs_after[1].text().contains("Page 1 of 1"));
}
