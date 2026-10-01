use pdf_engine_core::fonts::FontMetrics;
use pdf_engine_core::stream::{
    build_ast_from_operations, serialize_ast, ContentNode, ContentStreamTokenizer, GraphicsStateStack,
    Matrix,
};

#[test]
fn test_content_stream_ast_and_graphics_state_projection() {
    let raw_stream = b"
q
1 0 0 1 50 100 cm
BT
/F1 14 Tf
10 25 Td
(Invoice #1042) Tj
ET
Q
";

    // 1. Tokenize and build AST
    let mut tokenizer = ContentStreamTokenizer::new(raw_stream);
    let ops = tokenizer.tokenize_all().expect("Failed to tokenize content stream");
    let ast = build_ast_from_operations(ops);

    assert_eq!(ast.nodes.len(), 1);
    match &ast.nodes[0] {
        ContentNode::GraphicsGroup { children, .. } => {
            assert_eq!(children.len(), 2); // cm + TextBlock
        }
        _ => panic!("Expected top-level GraphicsGroup"),
    }

    // 2. Simulate graphics state projection
    let mut state_stack = GraphicsStateStack::new();

    // q
    state_stack.push();

    // 1 0 0 1 50 100 cm
    let cm = Matrix::translation(50.0, 100.0);
    state_stack.current.concat_matrix(&cm);

    // BT
    state_stack.current.begin_text();

    // /F1 14 Tf
    state_stack.current.text_state.font_size = 14.0;
    state_stack.current.text_state.font_name = "F1".to_string();

    // 10 25 Td
    state_stack.current.move_text_position(10.0, 25.0);

    // Baseline coordinate in page space:
    let (device_x, device_y) = state_stack.current.ctm.transform_point(
        state_stack.current.text_matrix.e,
        state_stack.current.text_matrix.f,
    );
    assert_eq!((device_x, device_y), (60.0, 125.0));

    // 3. Test typographic metrics advance
    let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
    let text = "Invoice #1042";
    let mut total_advance = 0.0;
    for b in text.bytes() {
        total_advance += metrics.compute_char_advance(b as u32, &state_stack.current.text_state);
    }
    // 13 characters * (500/1000 * 14) = 13 * 7 = 91.0
    assert_eq!(total_advance, 91.0);

    // 4. Test lossless AST roundtrip serialization
    let reserialized = serialize_ast(&ast);
    let text_repr = String::from_utf8_lossy(&reserialized);
    assert!(text_repr.contains("(Invoice #1042) Tj"));
    assert!(text_repr.contains("/F1 14 Tf"));
    assert!(text_repr.contains("q\n"));
    assert!(text_repr.contains("Q\n"));
}
