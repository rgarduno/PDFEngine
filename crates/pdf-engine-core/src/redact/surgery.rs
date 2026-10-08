//! Removes glyphs that intersect a redaction rectangle and draws an opaque blackout.
//!
//! Marked-content `/ActualText`, `/Alt`, and `/E` entries on the rewritten page are
//! removed with the glyphs. Attachments, the structure tree, and form appearances
//! are left in place. This is not an ISO 32000-1 legal redaction.

use std::collections::{BTreeMap, HashSet};

use crate::cos::object::{PdfDictionary, PdfName, PdfObject, PdfString};
use crate::error::PdfResult;
use crate::fonts::FontMetrics;
use crate::layout::geometry::Rect;
use crate::layout::glyph::PositionedGlyph;
use crate::layout::reconstructor::LayoutReconstructor;
use crate::redact::patterns::find_matches;
use crate::redact::types::{RedactionPattern, RedactionRect, RedactionSummary};
use crate::stream::ast::{ContentAst, ContentNode, NodeId, Operation};

/// Locates all text matches for `pattern` on a page's AST and computes their tight bounding boxes.
pub fn find_pattern_boxes_on_page(
    ast: &ContentAst,
    pattern: &RedactionPattern,
    metrics: &FontMetrics,
    padding: f64,
) -> PdfResult<Vec<Rect>> {
    let reconstructor = LayoutReconstructor::new(ast).with_font("F1", metrics.clone());
    let paragraphs = reconstructor.reconstruct()?;
    let mut detected_boxes = Vec::new();

    for paragraph in paragraphs {
        for line in paragraph.lines {
            let line_text = &line.text;
            let matches = find_matches(pattern, line_text);

            if matches.is_empty() {
                continue;
            }

            // Flatten all glyphs in this line with their corresponding string byte slice
            let mut line_glyphs: Vec<(usize, usize, &PositionedGlyph)> = Vec::new();
            let mut cur_byte = 0;

            for span in &line.spans {
                for glyph in &span.glyphs {
                    let g_len = glyph.unicode.len();
                    line_glyphs.push((cur_byte, cur_byte + g_len, glyph));
                    cur_byte += g_len;
                }
            }

            for (start_byte, end_byte) in matches {
                let mut match_bbox: Option<Rect> = None;

                for (g_start, g_end, glyph) in &line_glyphs {
                    // Check if glyph overlaps with match interval
                    if *g_start < end_byte && *g_end > start_byte {
                        match_bbox = match match_bbox {
                            Some(b) => Some(b.union(&glyph.bbox)),
                            None => Some(glyph.bbox),
                        };
                    }
                }

                if let Some(bbox) = match_bbox {
                    // Apply safety padding
                    let padded = Rect::new(
                        bbox.min_x - padding,
                        bbox.min_y - padding,
                        bbox.max_x + padding,
                        bbox.max_y + padding,
                    );
                    detected_boxes.push(padded);
                }
            }
        }
    }

    Ok(detected_boxes)
}

const ALTERNATE_TEXT_KEYS: &[&str] = &["ActualText", "Alt", "E"];

/// Drops replacement-text keys from dictionaries carried in the page content stream.
fn strip_alternate_text(nodes: &mut [ContentNode]) {
    for node in nodes {
        match node {
            ContentNode::GraphicsGroup { children, .. } => strip_alternate_text(children),
            ContentNode::TextBlock { operations, .. } => {
                for operation in operations {
                    for operand in &mut operation.operands {
                        strip_alternate_text_object(operand);
                    }
                }
            }
            ContentNode::Instruction { operation, .. } => {
                for operand in &mut operation.operands {
                    strip_alternate_text_object(operand);
                }
            }
        }
    }
}

fn strip_alternate_text_object(object: &mut PdfObject) {
    match object {
        PdfObject::Dictionary(dict) => strip_alternate_text_dict(dict),
        PdfObject::Stream(stream) => strip_alternate_text_dict(&mut stream.dict),
        PdfObject::Array(items) => {
            for item in items {
                strip_alternate_text_object(item);
            }
        }
        _ => {}
    }
}

fn strip_alternate_text_dict(dict: &mut PdfDictionary) {
    for key in ALTERNATE_TEXT_KEYS {
        dict.remove(key);
    }
    for (_key, value) in dict.iter_mut() {
        strip_alternate_text_object(value);
    }
}

/// Surgically excises glyphs intersecting `redactions` from `ast` and appends blackout vector patches.
pub fn apply_redaction_to_ast(
    ast: &mut ContentAst,
    redactions: &[RedactionRect],
    metrics: &FontMetrics,
) -> PdfResult<RedactionSummary> {
    let mut summary = RedactionSummary::default();
    if redactions.is_empty() {
        return Ok(summary);
    }

    strip_alternate_text(&mut ast.nodes);

    // Step 1: Reconstruct layout to identify positioned glyphs and their AST nodes
    let reconstructor = LayoutReconstructor::new(ast).with_font("F1", metrics.clone());
    let paragraphs = reconstructor.reconstruct()?;

    // Map each NodeId to its full list of glyphs with original index
    let mut node_glyphs: BTreeMap<NodeId, Vec<(usize, PositionedGlyph, bool)>> = BTreeMap::new();
    let mut dirty_node_ids = HashSet::new();

    let mut global_glyph_idx = 0;
    for paragraph in paragraphs {
        for line in paragraph.lines {
            for span in line.spans {
                for glyph in span.glyphs {
                    let intersects_redaction =
                        redactions.iter().any(|r| r.rect.intersects(&glyph.bbox));

                    let is_redacted = intersects_redaction;
                    if is_redacted {
                        summary.purged_glyphs_count += 1;
                        dirty_node_ids.insert(glyph.ast_node_id);
                    }

                    node_glyphs.entry(glyph.ast_node_id).or_default().push((
                        global_glyph_idx,
                        glyph,
                        is_redacted,
                    ));

                    global_glyph_idx += 1;
                }
            }
        }
    }

    // Step 2: Surgically mutate dirty AST TextBlocks
    for (node_id, glyphs) in node_glyphs {
        if !dirty_node_ids.contains(&node_id) {
            continue;
        }

        let replacement_operations = synthesize_redacted_text_operations(&glyphs);
        let replacement_node = ContentNode::TextBlock {
            id: node_id,
            operations: replacement_operations,
        };

        if replace_ast_node(&mut ast.nodes, node_id, replacement_node) {
            summary.modified_blocks_count += 1;
        }
    }

    // Step 3: Append opaque vector blackout patches and optional overlay text
    for red in redactions {
        let patch_node = synthesize_blackout_patch(ast.alloc_id(), red);
        ast.nodes.push(patch_node);
        summary.blackout_boxes_count += 1;
        summary.applied_rects.push(red.rect);
    }

    Ok(summary)
}

/// Partitions non-redacted glyphs into contiguous text runs preserving original coordinates.
fn synthesize_redacted_text_operations(
    glyphs: &[(usize, PositionedGlyph, bool)],
) -> Vec<Operation> {
    // Collect non-redacted glyphs
    let kept_glyphs: Vec<&(usize, PositionedGlyph, bool)> = glyphs
        .iter()
        .filter(|(_, _, is_redacted)| !*is_redacted)
        .collect();

    if kept_glyphs.is_empty() {
        // Entire block was redacted: emit empty BT ... ET
        return vec![
            Operation::new("BT", Vec::new()),
            Operation::new("ET", Vec::new()),
        ];
    }

    // Partition kept glyphs into contiguous runs
    let mut runs: Vec<Vec<&PositionedGlyph>> = Vec::new();
    let mut cur_run: Vec<&PositionedGlyph> = Vec::new();
    let mut last_idx: Option<usize> = None;

    for (idx, glyph, _) in kept_glyphs {
        if cur_run.is_empty() {
            cur_run.push(glyph);
            last_idx = Some(*idx);
            continue;
        }

        let prev = cur_run.last().unwrap();
        let was_adjacent = last_idx.map(|l| *idx == l + 1).unwrap_or(false);
        let same_font = prev.font_name == glyph.font_name;
        let same_size = (prev.font_size - glyph.font_size).abs() < 0.1;
        let same_baseline = (prev.origin.y - glyph.origin.y).abs() < 0.5;

        if was_adjacent && same_font && same_size && same_baseline {
            cur_run.push(glyph);
            last_idx = Some(*idx);
        } else {
            runs.push(std::mem::take(&mut cur_run));
            cur_run.push(glyph);
            last_idx = Some(*idx);
        }
    }

    if !cur_run.is_empty() {
        runs.push(cur_run);
    }

    // Generate ISO 32000 text operations for runs
    let mut ops = Vec::new();
    ops.push(Operation::new("BT", Vec::new()));

    let mut current_font = String::new();
    let mut current_size = 0.0;

    for run in runs {
        if run.is_empty() {
            continue;
        }

        let first = run[0];
        let font_clean = first.font_name.trim_start_matches('/');

        // Set font if changed
        if font_clean != current_font || (first.font_size - current_size).abs() >= 0.1 {
            ops.push(Operation::new(
                "Tf",
                vec![
                    PdfObject::Name(PdfName::new(font_clean)),
                    PdfObject::Real(first.font_size),
                ],
            ));
            current_font = font_clean.to_string();
            current_size = first.font_size;
        }

        // Set exact text matrix origin: [1 0 0 1 x y] Tm
        ops.push(Operation::new(
            "Tm",
            vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
                PdfObject::Real(first.origin.x),
                PdfObject::Real(first.origin.y),
            ],
        ));

        // Emit encoded character bytes
        let raw_bytes: Vec<u8> = run.iter().map(|g| g.char_code as u8).collect();
        ops.push(Operation::new(
            "Tj",
            vec![PdfObject::String(PdfString::literal(raw_bytes))],
        ));
    }

    ops.push(Operation::new("ET", Vec::new()));
    ops
}

/// Synthesizes an opaque vector graphics node with blackout fill and optional overlay label.
fn synthesize_blackout_patch(group_id: NodeId, red: &RedactionRect) -> ContentNode {
    let mut ops = Vec::new();

    // 1. Save graphics state `q`
    ops.push(Operation::new("q", Vec::new()));

    // 2. Set fill color `r g b rg`
    ops.push(Operation::new(
        "rg",
        vec![
            PdfObject::Real(red.fill_color[0]),
            PdfObject::Real(red.fill_color[1]),
            PdfObject::Real(red.fill_color[2]),
        ],
    ));

    // 3. Draw and fill rectangle `x y w h re f`
    let w = red.rect.width();
    let h = red.rect.height();
    ops.push(Operation::new(
        "re",
        vec![
            PdfObject::Real(red.rect.min_x),
            PdfObject::Real(red.rect.min_y),
            PdfObject::Real(w),
            PdfObject::Real(h),
        ],
    ));
    ops.push(Operation::new("f", Vec::new()));

    // 4. Draw border if specified
    if let Some(border) = red.border_color {
        ops.push(Operation::new(
            "RG",
            vec![
                PdfObject::Real(border[0]),
                PdfObject::Real(border[1]),
                PdfObject::Real(border[2]),
            ],
        ));
        ops.push(Operation::new("w", vec![PdfObject::Real(1.0)]));
        ops.push(Operation::new(
            "re",
            vec![
                PdfObject::Real(red.rect.min_x),
                PdfObject::Real(red.rect.min_y),
                PdfObject::Real(w),
                PdfObject::Real(h),
            ],
        ));
        ops.push(Operation::new("s", Vec::new()));
    }

    // 5. Draw overlay text if specified
    if let Some(ref label) = red.overlay_text {
        if !label.is_empty() {
            let font_size = red.font_size.unwrap_or_else(|| (h * 0.65).clamp(6.0, 11.0));

            // Center calculation
            let approx_width = (label.len() as f64) * font_size * 0.52;
            let tx = (red.rect.center_x() - approx_width * 0.5).max(red.rect.min_x + 1.0);
            let ty = red.rect.min_y + (h - font_size) * 0.5 + font_size * 0.22;

            ops.push(Operation::new("BT", Vec::new()));
            ops.push(Operation::new(
                "Tf",
                vec![
                    PdfObject::Name(PdfName::new("Helvetica")),
                    PdfObject::Real(font_size),
                ],
            ));
            ops.push(Operation::new(
                "rg",
                vec![
                    PdfObject::Real(red.text_color[0]),
                    PdfObject::Real(red.text_color[1]),
                    PdfObject::Real(red.text_color[2]),
                ],
            ));
            ops.push(Operation::new(
                "Tm",
                vec![
                    PdfObject::Real(1.0),
                    PdfObject::Real(0.0),
                    PdfObject::Real(0.0),
                    PdfObject::Real(1.0),
                    PdfObject::Real(tx),
                    PdfObject::Real(ty),
                ],
            ));
            ops.push(Operation::new(
                "Tj",
                vec![PdfObject::String(PdfString::literal(label.as_bytes()))],
            ));
            ops.push(Operation::new("ET", Vec::new()));
        }
    }

    // 6. Restore graphics state `Q`
    ops.push(Operation::new("Q", Vec::new()));

    ContentNode::GraphicsGroup {
        id: group_id,
        children: ops
            .into_iter()
            .map(|op| ContentNode::Instruction {
                id: group_id,
                operation: op,
            })
            .collect(),
    }
}

/// Recursively replaces a node by `NodeId` within the AST node tree.
fn replace_ast_node(
    nodes: &mut [ContentNode],
    target_id: NodeId,
    replacement: ContentNode,
) -> bool {
    for node in nodes.iter_mut() {
        match node {
            ContentNode::TextBlock { id, .. } => {
                if *id == target_id {
                    *node = replacement;
                    return true;
                }
            }
            ContentNode::GraphicsGroup { children, .. } => {
                if replace_ast_node(children, target_id, replacement.clone()) {
                    return true;
                }
            }
            ContentNode::Instruction { .. } => {}
        }
    }
    false
}
