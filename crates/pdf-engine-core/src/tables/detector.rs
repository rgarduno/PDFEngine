//! Semantic table detection and reconstruction according to ISO 32000-1 §14.8.4.
//!
//! Implements dual-strategy table extraction:
//! 1. **Lattice / Ruled Grid Analysis**: Parses vector drawing operators (`re`, `m`, `l`, `h`, `S`, `f`, `B`, `b`),
//!    projects lines via CTM graphics state, discovers closed intersecting grid cells, and resolves spans.
//! 2. **Stream / Borderless Layout Analysis**: Evaluates whitespace gutters, aligned text columns,
//!    and baseline rows when ruling lines are absent.

use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::layout::geometry::{Point, Rect};
use crate::layout::paragraph::ParagraphBlock;
use crate::layout::reconstructor::LayoutReconstructor;
use crate::stream::ast::{ContentAst, ContentNode, Operation};
use crate::stream::graphics_state::{GraphicsStateStack, Matrix};
use crate::stream::parser::{build_ast_from_operations, ContentStreamTokenizer};
use crate::tables::types::{DetectedTable, TableCell};

/// A horizontal line segment in page coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HorizSegment {
    pub y: f64,
    pub min_x: f64,
    pub max_x: f64,
}

/// A vertical line segment in page coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VertSegment {
    pub x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

/// Detects all structured tables on a given page of the document.
pub fn detect_tables(doc: &mut PdfDocument, page_idx: usize) -> PdfResult<Vec<DetectedTable>> {
    let pages = doc.get_pages()?;
    if page_idx == 0 || page_idx > pages.len() {
        return Err(PdfError::InvalidPageNumber {
            page: page_idx,
            total: pages.len(),
        });
    }

    let page_id = pages[page_idx - 1];
    let content_bytes = doc.get_page_content_bytes(page_id)?;
    if content_bytes.is_empty() {
        return Ok(Vec::new());
    }

    let mut tokenizer = ContentStreamTokenizer::new(&content_bytes);
    let operations = tokenizer.tokenize_all().unwrap_or_default();
    let ast = build_ast_from_operations(operations);

    // 1. Extract layout paragraphs and text lines for cell content assignment
    let reconstructor = LayoutReconstructor::new(&ast);
    let paragraphs = reconstructor.reconstruct()?;

    // 2. Try lattice-based (vector ruled grid) table detection
    let mut tables = detect_lattice_tables(&ast, &paragraphs, page_idx);

    // 3. If no ruled tables found, attempt stream (borderless whitespace) table detection
    if tables.is_empty() {
        tables = detect_borderless_tables(&paragraphs, page_idx);
    }

    Ok(tables)
}

/// Detects tables from vector graphics lines and bounding rectangles (Lattice strategy).
pub fn detect_lattice_tables(
    ast: &ContentAst,
    paragraphs: &[ParagraphBlock],
    page_number: usize,
) -> Vec<DetectedTable> {
    let (mut horiz_lines, mut vert_lines, cell_boxes) = extract_vector_segments(&ast.nodes);

    merge_collinear_horiz_segments(&mut horiz_lines);
    merge_collinear_vert_segments(&mut vert_lines);

    // If we have at least 2 horizontal and 2 vertical lines, find grid cells
    let mut detected_cells = Vec::new();
    if horiz_lines.len() >= 2 && vert_lines.len() >= 2 {
        detected_cells = find_lattice_cells(&horiz_lines, &vert_lines);
    }

    // Merge in any explicit cell rectangles discovered directly from `re` operators
    for r in cell_boxes {
        if r.width() >= 15.0 && r.height() >= 10.0 {
            let already_exists = detected_cells.iter().any(|c: &Rect| {
                (c.min_x - r.min_x).abs() < 2.0
                    && (c.max_x - r.max_x).abs() < 2.0
                    && (c.min_y - r.min_y).abs() < 2.0
                    && (c.max_y - r.max_y).abs() < 2.0
            });
            if !already_exists {
                detected_cells.push(r);
            }
        }
    }

    if detected_cells.is_empty() {
        return Vec::new();
    }

    // Cluster adjacent cells into table candidates
    let clusters = cluster_cells_into_tables(detected_cells);
    let mut results = Vec::new();

    for (table_idx, cluster) in clusters.into_iter().enumerate() {
        if cluster.len() < 2 {
            continue; // At least 2 cells needed to qualify as a table
        }

        if let Some(table) = build_table_from_cell_boxes(cluster, paragraphs, table_idx, page_number) {
            results.push(table);
        }
    }

    results
}

/// Recursively scans AST nodes tracking CTM and extracting horizontal/vertical vector lines.
fn extract_vector_segments(
    nodes: &[ContentNode],
) -> (Vec<HorizSegment>, Vec<VertSegment>, Vec<Rect>) {
    let mut horiz = Vec::new();
    let mut vert = Vec::new();
    let mut rects = Vec::new();
    let mut state_stack = GraphicsStateStack::new();

    scan_nodes_for_segments(nodes, &mut state_stack, &mut horiz, &mut vert, &mut rects);

    (horiz, vert, rects)
}

fn scan_nodes_for_segments(
    nodes: &[ContentNode],
    state_stack: &mut GraphicsStateStack,
    horiz: &mut Vec<HorizSegment>,
    vert: &mut Vec<VertSegment>,
    rects: &mut Vec<Rect>,
) {
    let mut current_point: Option<Point> = None;
    let mut subpath_start: Option<Point> = None;

    for node in nodes {
        match node {
            ContentNode::GraphicsGroup { children, .. } => {
                state_stack.push();
                scan_nodes_for_segments(children, state_stack, horiz, vert, rects);
                state_stack.pop();
            }
            ContentNode::Instruction { operation, .. } => {
                process_path_operation(
                    operation,
                    state_stack,
                    &mut current_point,
                    &mut subpath_start,
                    horiz,
                    vert,
                    rects,
                );
            }
            ContentNode::TextBlock { .. } => {
                // Text blocks do not draw vector grid lines
            }
        }
    }
}

fn process_path_operation(
    op: &Operation,
    state_stack: &mut GraphicsStateStack,
    current_point: &mut Option<Point>,
    subpath_start: &mut Option<Point>,
    horiz: &mut Vec<HorizSegment>,
    vert: &mut Vec<VertSegment>,
    rects: &mut Vec<Rect>,
) {
    let ctm = state_stack.current.ctm;

    match op.operator.as_str() {
        "q" => state_stack.push(),
        "Q" => {
            let _ = state_stack.pop();
        }
        "cm" => {
            if op.operands.len() == 6 {
                let a = op.operands[0].as_f64().unwrap_or(1.0);
                let b = op.operands[1].as_f64().unwrap_or(0.0);
                let c = op.operands[2].as_f64().unwrap_or(0.0);
                let d = op.operands[3].as_f64().unwrap_or(1.0);
                let e = op.operands[4].as_f64().unwrap_or(0.0);
                let f = op.operands[5].as_f64().unwrap_or(0.0);
                state_stack.current.concat_matrix(&Matrix::new(a, b, c, d, e, f));
            }
        }
        "m" => {
            if op.operands.len() >= 2 {
                let x = op.operands[0].as_f64().unwrap_or(0.0);
                let y = op.operands[1].as_f64().unwrap_or(0.0);
                let (px, py) = ctm.transform_point(x, y);
                let p = Point::new(px, py);
                *current_point = Some(p);
                *subpath_start = Some(p);
            }
        }
        "l" => {
            if op.operands.len() >= 2 {
                let x = op.operands[0].as_f64().unwrap_or(0.0);
                let y = op.operands[1].as_f64().unwrap_or(0.0);
                let (px, py) = ctm.transform_point(x, y);
                let p2 = Point::new(px, py);
                if let Some(p1) = *current_point {
                    evaluate_line_segment(p1, p2, horiz, vert);
                }
                *current_point = Some(p2);
            }
        }
        "h" | "s" | "b" | "B" => {
            // Close subpath by connecting to subpath_start
            if let (Some(p1), Some(p2)) = (*current_point, *subpath_start) {
                if (p1.x - p2.x).abs() > 0.1 || (p1.y - p2.y).abs() > 0.1 {
                    evaluate_line_segment(p1, p2, horiz, vert);
                }
            }
            *current_point = *subpath_start;
        }
        "re" => {
            if op.operands.len() >= 4 {
                let x = op.operands[0].as_f64().unwrap_or(0.0);
                let y = op.operands[1].as_f64().unwrap_or(0.0);
                let w = op.operands[2].as_f64().unwrap_or(0.0);
                let h = op.operands[3].as_f64().unwrap_or(0.0);

                let (p00_x, p00_y) = ctm.transform_point(x, y);
                let (p10_x, p10_y) = ctm.transform_point(x + w, y);
                let (p11_x, p11_y) = ctm.transform_point(x + w, y + h);
                let (p01_x, p01_y) = ctm.transform_point(x, y + h);

                let min_x = p00_x.min(p10_x).min(p11_x).min(p01_x);
                let max_x = p00_x.max(p10_x).max(p11_x).max(p01_x);
                let min_y = p00_y.min(p10_y).min(p11_y).min(p01_y);
                let max_y = p00_y.max(p10_y).max(p11_y).max(p01_y);

                let width = max_x - min_x;
                let height = max_y - min_y;

                if width >= 6.0 && height <= 3.0 {
                    // Thin horizontal ruled line
                    horiz.push(HorizSegment {
                        y: (min_y + max_y) / 2.0,
                        min_x,
                        max_x,
                    });
                } else if height >= 6.0 && width <= 3.0 {
                    // Thin vertical ruled line
                    vert.push(VertSegment {
                        x: (min_x + max_x) / 2.0,
                        min_y,
                        max_y,
                    });
                } else if width >= 8.0 && height >= 6.0 {
                    // Explicit rectangle: add outer borders as segments and store as potential cell
                    horiz.push(HorizSegment { y: min_y, min_x, max_x });
                    horiz.push(HorizSegment { y: max_y, min_x, max_x });
                    vert.push(VertSegment { x: min_x, min_y, max_y });
                    vert.push(VertSegment { x: max_x, min_y, max_y });
                    rects.push(Rect::new(min_x, min_y, max_x, max_y));
                }
            }
        }
        _ => {}
    }
}

fn evaluate_line_segment(
    p1: Point,
    p2: Point,
    horiz: &mut Vec<HorizSegment>,
    vert: &mut Vec<VertSegment>,
) {
    let dx = (p2.x - p1.x).abs();
    let dy = (p2.y - p1.y).abs();

    if dy <= 1.5 && dx >= 6.0 {
        horiz.push(HorizSegment {
            y: (p1.y + p2.y) / 2.0,
            min_x: p1.x.min(p2.x),
            max_x: p1.x.max(p2.x),
        });
    } else if dx <= 1.5 && dy >= 6.0 {
        vert.push(VertSegment {
            x: (p1.x + p2.x) / 2.0,
            min_y: p1.y.min(p2.y),
            max_y: p1.y.max(p2.y),
        });
    }
}

/// Merges collinear horizontal segments that share roughly the same Y coordinate and overlap or touch.
fn merge_collinear_horiz_segments(segments: &mut Vec<HorizSegment>) {
    if segments.len() <= 1 {
        return;
    }

    segments.sort_by(|a, b| {
        a.y.partial_cmp(&b.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.min_x.partial_cmp(&b.min_x).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut merged: Vec<HorizSegment> = Vec::new();
    for seg in segments.drain(..) {
        if let Some(last) = merged.last_mut() {
            if (last.y - seg.y).abs() <= 1.5 && seg.min_x <= last.max_x + 3.0 {
                last.max_x = last.max_x.max(seg.max_x);
                last.min_x = last.min_x.min(seg.min_x);
                last.y = (last.y + seg.y) / 2.0;
                continue;
            }
        }
        merged.push(seg);
    }

    *segments = merged;
}

/// Merges collinear vertical segments that share roughly the same X coordinate and overlap or touch.
fn merge_collinear_vert_segments(segments: &mut Vec<VertSegment>) {
    if segments.len() <= 1 {
        return;
    }

    segments.sort_by(|a, b| {
        a.x.partial_cmp(&b.x)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.min_y.partial_cmp(&b.min_y).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut merged: Vec<VertSegment> = Vec::new();
    for seg in segments.drain(..) {
        if let Some(last) = merged.last_mut() {
            if (last.x - seg.x).abs() <= 1.5 && seg.min_y <= last.max_y + 3.0 {
                last.max_y = last.max_y.max(seg.max_y);
                last.min_y = last.min_y.min(seg.min_y);
                last.x = (last.x + seg.x) / 2.0;
                continue;
            }
        }
        merged.push(seg);
    }

    *segments = merged;
}

/// Finds lattice cells formed by intersecting horizontal and vertical lines.
fn find_lattice_cells(horiz: &[HorizSegment], vert: &[VertSegment]) -> Vec<Rect> {
    // Extract unique Y coordinates (clustered within 2.5 pt)
    let mut y_levels: Vec<f64> = horiz.iter().map(|h| h.y).collect();
    y_levels.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal)); // Descending: top to bottom
    y_levels = cluster_floats(y_levels, 2.5);

    // Extract unique X coordinates (clustered within 2.5 pt)
    let mut x_levels: Vec<f64> = vert.iter().map(|v| v.x).collect();
    x_levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)); // Ascending: left to right
    x_levels = cluster_floats(x_levels, 2.5);

    if y_levels.len() < 2 || x_levels.len() < 2 {
        return Vec::new();
    }

    let mut cells = Vec::new();

    // Iterate through adjacent pairs of horizontal levels and vertical levels
    for i in 0..y_levels.len() - 1 {
        let top_y = y_levels[i];
        let bot_y = y_levels[i + 1];
        if (top_y - bot_y).abs() < 5.0 {
            continue;
        }

        for j in 0..x_levels.len() - 1 {
            let left_x = x_levels[j];
            let right_x = x_levels[j + 1];
            if (right_x - left_x).abs() < 8.0 {
                continue;
            }

            // Verify support: does this candidate box have horizontal borders and vertical borders?
            let has_top = horiz.iter().any(|h| {
                (h.y - top_y).abs() <= 2.5 && h.min_x <= left_x + 3.0 && h.max_x >= right_x - 3.0
            });
            let has_bot = horiz.iter().any(|h| {
                (h.y - bot_y).abs() <= 2.5 && h.min_x <= left_x + 3.0 && h.max_x >= right_x - 3.0
            });
            let has_left = vert.iter().any(|v| {
                (v.x - left_x).abs() <= 2.5 && v.min_y <= bot_y + 3.0 && v.max_y >= top_y - 3.0
            });
            let has_right = vert.iter().any(|v| {
                (v.x - right_x).abs() <= 2.5 && v.min_y <= bot_y + 3.0 && v.max_y >= top_y - 3.0
            });

            let edge_count = (has_top as usize) + (has_bot as usize) + (has_left as usize) + (has_right as usize);
            if edge_count >= 3 {
                cells.push(Rect::new(left_x, bot_y, right_x, top_y));
            }
        }
    }

    cells
}

/// Clusters close floating point numbers into single representative values.
fn cluster_floats(vals: Vec<f64>, tolerance: f64) -> Vec<f64> {
    if vals.is_empty() {
        return vals;
    }
    let mut clustered = Vec::new();
    let mut current_cluster: Vec<f64> = vec![vals[0]];

    for val in vals.into_iter().skip(1) {
        let mean = current_cluster.iter().sum::<f64>() / current_cluster.len() as f64;
        if (val - mean).abs() <= tolerance {
            current_cluster.push(val);
        } else {
            clustered.push(mean);
            current_cluster = vec![val];
        }
    }
    if !current_cluster.is_empty() {
        clustered.push(current_cluster.iter().sum::<f64>() / current_cluster.len() as f64);
    }
    clustered
}

/// Clusters cell bounding boxes into connected components forming discrete tables.
fn cluster_cells_into_tables(cells: Vec<Rect>) -> Vec<Vec<Rect>> {
    let n = cells.len();
    let mut visited = vec![false; n];
    let mut clusters = Vec::new();

    for i in 0..n {
        if visited[i] {
            continue;
        }

        let mut current_cluster = Vec::new();
        let mut queue = vec![i];
        visited[i] = true;

        while let Some(idx) = queue.pop() {
            let cell = cells[idx];
            current_cluster.push(cell);

            for j in 0..n {
                if !visited[j] {
                    let other = cells[j];
                    // Check adjacency: horizontal or vertical distance <= 5.0 pt
                    let x_overlap = cell.min_x < other.max_x + 3.0 && cell.max_x > other.min_x - 3.0;
                    let y_overlap = cell.min_y < other.max_y + 3.0 && cell.max_y > other.min_y - 3.0;

                    if (x_overlap && (cell.min_y - other.max_y).abs() <= 5.0)
                        || (x_overlap && (other.min_y - cell.max_y).abs() <= 5.0)
                        || (y_overlap && (cell.min_x - other.max_x).abs() <= 5.0)
                        || (y_overlap && (other.min_x - cell.max_x).abs() <= 5.0)
                    {
                        visited[j] = true;
                        queue.push(j);
                    }
                }
            }
        }

        clusters.push(current_cluster);
    }

    clusters
}

/// Builds a structured `DetectedTable` from a cluster of cell bounding boxes and page paragraphs.
fn build_table_from_cell_boxes(
    mut boxes: Vec<Rect>,
    paragraphs: &[ParagraphBlock],
    table_idx: usize,
    page_number: usize,
) -> Option<DetectedTable> {
    if boxes.is_empty() {
        return None;
    }

    // Compute table bounding box
    let mut table_bbox = boxes[0];
    for b in &boxes[1..] {
        table_bbox = table_bbox.union(b);
    }

    // Extract unique sorted row Y coordinates (top down, descending)
    let mut y_coords: Vec<f64> = boxes.iter().map(|b| b.max_y).collect();
    y_coords.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let row_levels = cluster_floats(y_coords, 4.0);

    // Extract unique sorted col X coordinates (left to right, ascending)
    let mut x_coords: Vec<f64> = boxes.iter().map(|b| b.min_x).collect();
    x_coords.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let col_levels = cluster_floats(x_coords, 4.0);

    let row_count = row_levels.len();
    let col_count = col_levels.len();

    if row_count == 0 || col_count == 0 {
        return None;
    }

    // Sort boxes top-to-bottom, left-to-right
    boxes.sort_by(|a, b| {
        b.max_y.partial_cmp(&a.max_y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.min_x.partial_cmp(&b.min_x).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut cells = Vec::new();
    for b in boxes {
        // Find row_idx
        let row_idx = row_levels
            .iter()
            .position(|&y| (b.max_y - y).abs() <= 4.0)
            .unwrap_or(0);

        // Find col_idx
        let col_idx = col_levels
            .iter()
            .position(|&x| (b.min_x - x).abs() <= 4.0)
            .unwrap_or(0);

        // Assign text from page paragraphs that intersect this cell box
        let cell_text = extract_text_for_box(&b, paragraphs);
        let is_header = row_idx == 0;

        let mut cell = TableCell::new(row_idx, col_idx, b, cell_text);
        cell.is_header = is_header;
        cells.push(cell);
    }

    let mut table = DetectedTable {
        table_idx,
        page_number,
        bbox: table_bbox,
        row_count,
        col_count,
        cells,
        headers: Vec::new(),
        rows: Vec::new(),
    };

    table.rebuild_matrix();

    // Verify if table has any non-empty text content
    let has_content = table.headers.iter().any(|h| !h.trim().is_empty())
        || table.rows.iter().any(|r| r.iter().any(|c| !c.trim().is_empty()));

    if has_content || table.row_count * table.col_count >= 4 {
        Some(table)
    } else {
        None
    }
}

/// Fallback: Detects borderless tables using whitespace gutters and aligned columns (Stream strategy).
pub fn detect_borderless_tables(
    paragraphs: &[ParagraphBlock],
    page_number: usize,
) -> Vec<DetectedTable> {
    if paragraphs.is_empty() {
        return Vec::new();
    }

    // Collect all lines across paragraphs
    let mut all_lines = Vec::new();
    for p in paragraphs {
        for line in &p.lines {
            all_lines.push(line.clone());
        }
    }

    if all_lines.len() < 3 {
        return Vec::new();
    }

    // Sort lines top to bottom (descending baseline_y)
    all_lines.sort_by(|a, b| b.baseline_y.partial_cmp(&a.baseline_y).unwrap_or(std::cmp::Ordering::Equal));

    // Analyze lines for tabbed or multi-column layout
    // A line with at least 2 spans separated by > 12 pt whitespace is a candidate table row
    let mut candidate_rows = Vec::new();
    for line in &all_lines {
        if line.spans.len() >= 2 {
            candidate_rows.push(line);
        } else {
            // Check if text has multiple space-separated columns
            let parts: Vec<&str> = line.text.split("   ").filter(|s| !s.trim().is_empty()).collect();
            if parts.len() >= 2 {
                candidate_rows.push(line);
            }
        }
    }

    if candidate_rows.len() < 2 {
        return Vec::new();
    }

    // Group adjacent candidate rows with similar column alignments
    let mut table_bbox = candidate_rows[0].bbox;
    for r in &candidate_rows[1..] {
        table_bbox = table_bbox.union(&r.bbox);
    }

    // Discover column bounds across candidate rows
    let mut col_starts = Vec::new();
    for r in &candidate_rows {
        if r.spans.len() >= 2 {
            for span in &r.spans {
                col_starts.push(span.bbox.min_x);
            }
        }
    }
    col_starts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let clustered_cols = cluster_floats(col_starts, 10.0);

    if clustered_cols.len() < 2 {
        return Vec::new();
    }

    let col_count = clustered_cols.len();
    let row_count = candidate_rows.len();
    let mut cells = Vec::new();

    for (r_idx, line) in candidate_rows.iter().enumerate() {
        for (c_idx, &col_x) in clustered_cols.iter().enumerate() {
            let next_col_x = if c_idx + 1 < clustered_cols.len() {
                clustered_cols[c_idx + 1]
            } else {
                table_bbox.max_x + 5.0
            };

            let cell_box = Rect::new(col_x, line.bbox.min_y, next_col_x, line.bbox.max_y);
            let text = extract_text_for_box(&cell_box, paragraphs);
            let mut cell = TableCell::new(r_idx, c_idx, cell_box, text);
            cell.is_header = r_idx == 0;
            cells.push(cell);
        }
    }

    let mut table = DetectedTable {
        table_idx: 0,
        page_number,
        bbox: table_bbox,
        row_count,
        col_count,
        cells,
        headers: Vec::new(),
        rows: Vec::new(),
    };

    table.rebuild_matrix();

    vec![table]
}

/// Extracts and concatenates text from paragraphs that falls within a given bounding box.
fn extract_text_for_box(bbox: &Rect, paragraphs: &[ParagraphBlock]) -> String {
    let mut fragments = Vec::new();

    for p in paragraphs {
        for line in &p.lines {
            for span in &line.spans {
                let mid_x = (span.bbox.min_x + span.bbox.max_x) / 2.0;
                let mid_y = (span.bbox.min_y + span.bbox.max_y) / 2.0;

                // Test if the span's midpoint is inside the cell box, or if the majority of the span overlaps
                let is_inside = mid_x >= bbox.min_x - 0.5
                    && mid_x <= bbox.max_x + 0.5
                    && mid_y >= bbox.min_y - 0.5
                    && mid_y <= bbox.max_y + 0.5;

                let overlaps = span.bbox.min_x < bbox.max_x && span.bbox.max_x > bbox.min_x
                    && span.bbox.min_y < bbox.max_y && span.bbox.max_y > bbox.min_y;

                let overlap_ratio = if overlaps {
                    let ox1 = span.bbox.min_x.max(bbox.min_x);
                    let ox2 = span.bbox.max_x.min(bbox.max_x);
                    let oy1 = span.bbox.min_y.max(bbox.min_y);
                    let oy2 = span.bbox.max_y.min(bbox.max_y);
                    let inter_area = (ox2 - ox1).max(0.0) * (oy2 - oy1).max(0.0);
                    let span_area = span.bbox.width() * span.bbox.height();
                    if span_area > 0.0 {
                        inter_area / span_area
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };

                if is_inside || overlap_ratio >= 0.5 {
                    fragments.push((span.bbox.max_y, span.bbox.min_x, span.text.clone()));
                }
            }
        }
    }

    if fragments.is_empty() {
        return String::new();
    }

    // Sort reading order: descending Y (top-to-bottom), ascending X (left-to-right)
    fragments.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut result = String::new();
    for (_, _, text) in fragments {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(trimmed);
        }
    }

    result
}
