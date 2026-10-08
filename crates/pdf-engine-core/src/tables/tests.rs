//! Unit and integration tests for table detection and multi-format export.

use crate::cos::object::PdfObject;
use crate::error::PdfError;
use crate::layout::geometry::Rect;
use crate::layout::line::TextLine;
use crate::layout::paragraph::ParagraphBlock;
use crate::layout::span::TextSpan;
use crate::stream::ast::{ContentAst, ContentNode, NodeId, Operation};
use crate::tables::detector::{
    detect_borderless_tables, detect_lattice_tables, MAX_LATTICE_CELL_PRODUCT,
    MAX_MERGED_RULE_LINES, MAX_RAW_VECTOR_SEGMENTS,
};
use crate::tables::export::{export_to_csv, export_to_html, export_to_json, export_to_markdown};
use crate::tables::types::{DetectedTable, TableCell};

#[test]
fn test_csv_rfc4180_export() {
    let mut table = DetectedTable {
        table_idx: 0,
        page_number: 1,
        bbox: Rect::new(50.0, 500.0, 500.0, 700.0),
        row_count: 3,
        col_count: 3,
        cells: vec![
            TableCell::new(0, 0, Rect::new(50.0, 650.0, 200.0, 700.0), "Product Name"),
            TableCell::new(0, 1, Rect::new(200.0, 650.0, 350.0, 700.0), "Description"),
            TableCell::new(0, 2, Rect::new(350.0, 650.0, 500.0, 700.0), "Price, USD"),
            TableCell::new(1, 0, Rect::new(50.0, 600.0, 200.0, 650.0), "Widget A"),
            TableCell::new(
                1,
                1,
                Rect::new(200.0, 600.0, 350.0, 650.0),
                "High \"performance\" model",
            ),
            TableCell::new(1, 2, Rect::new(350.0, 600.0, 500.0, 650.0), "$100.00"),
            TableCell::new(2, 0, Rect::new(50.0, 550.0, 200.0, 600.0), "Widget B"),
            TableCell::new(
                2,
                1,
                Rect::new(200.0, 550.0, 350.0, 600.0),
                "Standard, reliable",
            ),
            TableCell::new(2, 2, Rect::new(350.0, 550.0, 500.0, 600.0), "$50.00"),
        ],
        headers: Vec::new(),
        rows: Vec::new(),
    };

    table.rebuild_matrix();

    let csv = export_to_csv(&table);
    assert!(csv.contains("Product Name,Description,\"Price, USD\""));
    assert!(csv.contains("Widget A,\"High \"\"performance\"\" model\",$100.00"));
    assert!(csv.contains("Widget B,\"Standard, reliable\",$50.00"));
}

#[test]
fn test_markdown_and_html_export() {
    let mut table = DetectedTable {
        table_idx: 0,
        page_number: 1,
        bbox: Rect::new(50.0, 500.0, 400.0, 650.0),
        row_count: 2,
        col_count: 2,
        cells: vec![
            TableCell::new(0, 0, Rect::new(50.0, 600.0, 200.0, 650.0), "Col | A"),
            TableCell::new(0, 1, Rect::new(200.0, 600.0, 400.0, 650.0), "Col B"),
            TableCell::new(1, 0, Rect::new(50.0, 550.0, 200.0, 600.0), "Val 1 <script>"),
            TableCell::new(1, 1, Rect::new(200.0, 550.0, 400.0, 600.0), "Val 2 & info"),
        ],
        headers: Vec::new(),
        rows: Vec::new(),
    };

    table.rebuild_matrix();

    let md = export_to_markdown(&table);
    assert!(md.contains("| Col \\| A | Col B |"));
    assert!(md.contains("| :--- | :--- |"));
    assert!(md.contains("| Val 1 <script> | Val 2 & info |"));

    let html = export_to_html(&table);
    assert!(html.contains("<th>Col | A</th>"));
    assert!(html.contains("<td>Val 1 &lt;script&gt;</td>"));
    assert!(html.contains("<td>Val 2 &amp; info</td>"));
}

#[test]
fn test_html_export_escapes_every_markup_character() {
    let mut table = DetectedTable {
        table_idx: 0,
        page_number: 1,
        bbox: Rect::new(50.0, 500.0, 200.0, 560.0),
        row_count: 1,
        col_count: 1,
        cells: vec![TableCell::new(
            0,
            0,
            Rect::new(50.0, 500.0, 200.0, 560.0),
            "a&b<c>d\"e'f",
        )],
        headers: Vec::new(),
        rows: Vec::new(),
    };
    table.rebuild_matrix();

    let html = export_to_html(&table);
    assert!(html.contains("a&amp;b&lt;c&gt;d&quot;e&#39;f"));
    assert!(!html.contains("a&b<c>"));
    assert!(!html.contains("<script"));
}

#[test]
fn test_json_export_structure() {
    let mut table = DetectedTable {
        table_idx: 1,
        page_number: 2,
        bbox: Rect::new(100.0, 200.0, 400.0, 500.0),
        row_count: 2,
        col_count: 2,
        cells: vec![
            TableCell::new(0, 0, Rect::new(100.0, 350.0, 250.0, 500.0), "Header 1"),
            TableCell::new(0, 1, Rect::new(250.0, 350.0, 400.0, 500.0), "Header 2"),
            TableCell::new(1, 0, Rect::new(100.0, 200.0, 250.0, 350.0), "Row 1 Col 1"),
            TableCell::new(1, 1, Rect::new(250.0, 200.0, 400.0, 350.0), "Row 1 Col 2"),
        ],
        headers: Vec::new(),
        rows: Vec::new(),
    };

    table.rebuild_matrix();

    let json = export_to_json(&table);
    assert!(json.contains("\"table_index\": 1"));
    assert!(json.contains("\"page_number\": 2"));
    assert!(json.contains("\"row_count\": 2"));
    assert!(json.contains("\"col_count\": 2"));
    assert!(json.contains("\"headers\": [\"Header 1\", \"Header 2\"]"));
    assert!(json.contains("[\"Row 1 Col 1\", \"Row 1 Col 2\"]"));
}

fn add_horiz_line(nodes: &mut Vec<ContentNode>, y: f64, x1: f64, x2: f64) {
    nodes.push(ContentNode::Instruction {
        id: NodeId(nodes.len()),
        operation: Operation::new("m", vec![PdfObject::Real(x1), PdfObject::Real(y)]),
    });
    nodes.push(ContentNode::Instruction {
        id: NodeId(nodes.len()),
        operation: Operation::new("l", vec![PdfObject::Real(x2), PdfObject::Real(y)]),
    });
    nodes.push(ContentNode::Instruction {
        id: NodeId(nodes.len()),
        operation: Operation::new("S", vec![]),
    });
}

fn add_vert_line(nodes: &mut Vec<ContentNode>, x: f64, y1: f64, y2: f64) {
    nodes.push(ContentNode::Instruction {
        id: NodeId(nodes.len()),
        operation: Operation::new("m", vec![PdfObject::Real(x), PdfObject::Real(y1)]),
    });
    nodes.push(ContentNode::Instruction {
        id: NodeId(nodes.len()),
        operation: Operation::new("l", vec![PdfObject::Real(x), PdfObject::Real(y2)]),
    });
    nodes.push(ContentNode::Instruction {
        id: NodeId(nodes.len()),
        operation: Operation::new("S", vec![]),
    });
}

#[test]
fn test_lattice_grid_detection() {
    let mut ast = ContentAst::new();

    // 2 rows x 2 cols grid lines:
    // X boundaries: 100.0, 200.0, 300.0
    // Y boundaries: 600.0, 650.0, 700.0
    // Horizontal lines
    add_horiz_line(&mut ast.nodes, 700.0, 100.0, 300.0);
    add_horiz_line(&mut ast.nodes, 650.0, 100.0, 300.0);
    add_horiz_line(&mut ast.nodes, 600.0, 100.0, 300.0);

    // Vertical lines
    add_vert_line(&mut ast.nodes, 100.0, 600.0, 700.0);
    add_vert_line(&mut ast.nodes, 200.0, 600.0, 700.0);
    add_vert_line(&mut ast.nodes, 300.0, 600.0, 700.0);

    // Create synthetic paragraph blocks with text inside the cells
    let span1 = TextSpan {
        text: "Item".to_string(),
        font_name: "F1".to_string(),
        font_size: 10.0,
        rendered_size: 10.0,
        baseline_y: 670.0,
        bbox: Rect::new(110.0, 665.0, 140.0, 680.0),
        glyphs: vec![],
    };
    let line1 = TextLine {
        text: "Item".to_string(),
        bbox: span1.bbox,
        baseline_y: 670.0,
        start_x: 110.0,
        end_x: 140.0,
        spans: vec![span1],
    };
    let para1 = ParagraphBlock::new(0, vec![line1], vec![NodeId(1)]).unwrap();

    let span2 = TextSpan {
        text: "Cost".to_string(),
        font_name: "F1".to_string(),
        font_size: 10.0,
        rendered_size: 10.0,
        baseline_y: 670.0,
        bbox: Rect::new(210.0, 665.0, 240.0, 680.0),
        glyphs: vec![],
    };
    let line2 = TextLine {
        text: "Cost".to_string(),
        bbox: span2.bbox,
        baseline_y: 670.0,
        start_x: 210.0,
        end_x: 240.0,
        spans: vec![span2],
    };
    let para2 = ParagraphBlock::new(1, vec![line2], vec![NodeId(2)]).unwrap();

    let tables = detect_lattice_tables(&ast, &[para1, para2], 1).expect("small lattice");
    assert_eq!(tables.len(), 1);
    let table = &tables[0];
    assert_eq!(table.row_count, 2);
    assert_eq!(table.col_count, 2);
    assert_eq!(table.headers, vec!["Item", "Cost"]);
}

#[test]
fn test_borderless_whitespace_table_detection() {
    // Create multi-column lines without vector lines
    let span_a1 = TextSpan {
        text: "Date".to_string(),
        font_name: "F1".to_string(),
        font_size: 10.0,
        rendered_size: 10.0,
        baseline_y: 700.0,
        bbox: Rect::new(72.0, 695.0, 120.0, 710.0),
        glyphs: vec![],
    };
    let span_a2 = TextSpan {
        text: "Amount".to_string(),
        font_name: "F1".to_string(),
        font_size: 10.0,
        rendered_size: 10.0,
        baseline_y: 700.0,
        bbox: Rect::new(250.0, 695.0, 300.0, 710.0),
        glyphs: vec![],
    };
    let line1 = TextLine {
        text: "Date Amount".to_string(),
        bbox: Rect::new(72.0, 695.0, 300.0, 710.0),
        baseline_y: 700.0,
        start_x: 72.0,
        end_x: 300.0,
        spans: vec![span_a1, span_a2],
    };

    let span_b1 = TextSpan {
        text: "2026-01-01".to_string(),
        font_name: "F1".to_string(),
        font_size: 10.0,
        rendered_size: 10.0,
        baseline_y: 680.0,
        bbox: Rect::new(72.0, 675.0, 150.0, 690.0),
        glyphs: vec![],
    };
    let span_b2 = TextSpan {
        text: "$250.00".to_string(),
        font_name: "F1".to_string(),
        font_size: 10.0,
        rendered_size: 10.0,
        baseline_y: 680.0,
        bbox: Rect::new(250.0, 675.0, 310.0, 690.0),
        glyphs: vec![],
    };
    let line2 = TextLine {
        text: "2026-01-01 $250.00".to_string(),
        bbox: Rect::new(72.0, 675.0, 310.0, 690.0),
        baseline_y: 680.0,
        start_x: 72.0,
        end_x: 310.0,
        spans: vec![span_b1, span_b2],
    };

    let span_c1 = TextSpan {
        text: "2026-01-02".to_string(),
        font_name: "F1".to_string(),
        font_size: 10.0,
        rendered_size: 10.0,
        baseline_y: 660.0,
        bbox: Rect::new(72.0, 655.0, 150.0, 670.0),
        glyphs: vec![],
    };
    let span_c2 = TextSpan {
        text: "$500.00".to_string(),
        font_name: "F1".to_string(),
        font_size: 10.0,
        rendered_size: 10.0,
        baseline_y: 660.0,
        bbox: Rect::new(250.0, 655.0, 310.0, 670.0),
        glyphs: vec![],
    };
    let line3 = TextLine {
        text: "2026-01-02 $500.00".to_string(),
        bbox: Rect::new(72.0, 655.0, 310.0, 670.0),
        baseline_y: 660.0,
        start_x: 72.0,
        end_x: 310.0,
        spans: vec![span_c1, span_c2],
    };

    let p1 = ParagraphBlock::new(0, vec![line1], vec![NodeId(1)]).unwrap();
    let p2 = ParagraphBlock::new(1, vec![line2], vec![NodeId(2)]).unwrap();
    let p3 = ParagraphBlock::new(2, vec![line3], vec![NodeId(3)]).unwrap();

    let tables = detect_borderless_tables(&[p1, p2, p3], 1);
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].col_count, 2);
    assert_eq!(tables[0].row_count, 3);
}

fn assert_segment_limit(result: Result<Vec<DetectedTable>, PdfError>) {
    match result {
        Err(PdfError::SecurityLimitExceeded(_)) => {}
        other => panic!("expected a segment limit, got {other:?}"),
    }
}

#[test]
fn test_lattice_stops_before_the_raw_segment_budget() {
    let mut ast = ContentAst::new();
    for i in 0..=MAX_RAW_VECTOR_SEGMENTS {
        let y = i as f64 * 4.0;
        add_horiz_line(&mut ast.nodes, y, 0.0, 40.0);
    }

    assert_segment_limit(detect_lattice_tables(&ast, &[], 1));
}

#[test]
fn test_lattice_stops_before_the_merged_line_search() {
    let mut ast = ContentAst::new();
    for i in 0..=MAX_MERGED_RULE_LINES {
        let y = 800.0 - (i as f64 * 4.0);
        add_horiz_line(&mut ast.nodes, y, 40.0, 240.0);
    }
    add_vert_line(&mut ast.nodes, 40.0, 0.0, 800.0);
    add_vert_line(&mut ast.nodes, 240.0, 0.0, 800.0);

    assert_segment_limit(detect_lattice_tables(&ast, &[], 1));
}

#[test]
fn test_lattice_stops_before_the_cell_product_search() {
    let mut ast = ContentAst::new();
    let axis: usize = 100;
    assert!(axis < MAX_MERGED_RULE_LINES);
    assert!(axis * axis > MAX_LATTICE_CELL_PRODUCT);

    for i in 0..axis {
        let y = 2000.0 - (i as f64 * 12.0);
        add_horiz_line(&mut ast.nodes, y, 0.0, axis as f64 * 20.0);
        let x = i as f64 * 20.0;
        add_vert_line(&mut ast.nodes, x, 0.0, 2000.0);
    }

    assert_segment_limit(detect_lattice_tables(&ast, &[], 1));
}
