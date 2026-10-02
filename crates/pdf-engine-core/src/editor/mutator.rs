//! Surgical in-place Content AST mutator.
//!
//! Replaces target text blocks with freshly re-flowed text operations
//! while preserving 100% of surrounding vector paths, images, and graphics states.

use crate::cos::object::{PdfName, PdfObject, PdfString};
use crate::error::{PdfError, PdfResult};
use crate::fonts::{FontEncoder, FontMetrics};
use crate::layout::paragraph::ParagraphBlock;
use crate::stream::ast::{ContentAst, ContentNode, NodeId, Operation};
use crate::stream::graphics_state::TextState;
use crate::editor::reflow::ReflowEngine;

/// Performs surgical in-place text mutations on a page's Content AST.
pub struct SurgicalEditor;

impl SurgicalEditor {
    /// Replaces the text of a paragraph block in-place with new content using default encoder.
    pub fn edit_paragraph(
        ast: &mut ContentAst,
        target_block: &ParagraphBlock,
        new_text: &str,
        metrics: &FontMetrics,
    ) -> PdfResult<()> {
        let encoder = FontEncoder::new();
        Self::edit_paragraph_with_encoder(ast, target_block, new_text, metrics, &encoder)
    }

    /// Replaces the text of a paragraph block in-place with character encoding and glyph fallback.
    pub fn edit_paragraph_with_encoder(
        ast: &mut ContentAst,
        target_block: &ParagraphBlock,
        new_text: &str,
        metrics: &FontMetrics,
        encoder: &FontEncoder,
    ) -> PdfResult<()> {
        if target_block.source_node_ids.is_empty() {
            return Err(PdfError::LayoutError(
                "Target paragraph has no associated AST source nodes".to_string(),
            ));
        }

        let primary_node_id = target_block.source_node_ids[0];

        // Determine font style from the original block's first line and span
        let first_line = target_block.lines.first().ok_or_else(|| {
            PdfError::LayoutError("Target paragraph contains no lines".to_string())
        })?;
        let first_span = first_line.spans.first().ok_or_else(|| {
            PdfError::LayoutError("Target paragraph contains no styled spans".to_string())
        })?;

        let font_name = first_span.font_name.clone();
        let font_size = first_span.font_size;
        let mut text_state = TextState::default();
        text_state.font_name = font_name.clone();
        text_state.font_size = font_size;

        // Perform reflow
        let max_width = target_block.bbox.width().max(50.0);
        let reflow_lines = ReflowEngine::reflow(
            new_text,
            max_width,
            metrics,
            &text_state,
            target_block.alignment,
            target_block.leading,
        );

        // Generate replacement TextBlock operations
        let new_operations = Self::synthesize_text_operations(
            &reflow_lines,
            target_block.bbox.min_x,
            first_line.baseline_y,
            &font_name,
            font_size,
            encoder,
        );

        let replacement_node = ContentNode::TextBlock {
            id: primary_node_id,
            operations: new_operations,
        };

        // Mutate AST in place
        let mut replaced = false;
        Self::mutate_nodes(
            &mut ast.nodes,
            primary_node_id,
            &target_block.source_node_ids[1..],
            replacement_node,
            &mut replaced,
        );

        if !replaced {
            return Err(PdfError::ContentStreamError(format!(
                "Failed to find AST node {:?} for paragraph replacement",
                primary_node_id
            )));
        }

        Ok(())
    }

    /// Synthesizes clean ISO 32000 text operations (`BT`, `Tf`, `Tm`, `Tj`, `ET`).
    fn synthesize_text_operations(
        lines: &[crate::editor::reflow::ReflowLine],
        origin_x: f64,
        top_baseline_y: f64,
        font_name: &str,
        font_size: f64,
        encoder: &FontEncoder,
    ) -> Vec<Operation> {
        let mut ops = Vec::new();

        // 1. Begin Text
        ops.push(Operation::new("BT", Vec::new()));

        // 2. Set Font and Size
        let font_name_clean = font_name.trim_start_matches('/');
        ops.push(Operation::new(
            "Tf",
            vec![
                PdfObject::Name(PdfName::new(font_name_clean)),
                PdfObject::Real(font_size),
            ],
        ));

        // 3. For each line: position and show string
        for line in lines {
            let x = origin_x + line.offset_x;
            let y = top_baseline_y + line.offset_y;

            // Set line word spacing if justified
            if line.word_spacing > 0.0 {
                ops.push(Operation::new(
                    "Tw",
                    vec![PdfObject::Real(line.word_spacing)],
                ));
            }

            // Set line baseline position using Tm: [1 0 0 1 x y] Tm
            ops.push(Operation::new(
                "Tm",
                vec![
                    PdfObject::Real(1.0),
                    PdfObject::Real(0.0),
                    PdfObject::Real(0.0),
                    PdfObject::Real(1.0),
                    PdfObject::Real(x),
                    PdfObject::Real(y),
                ],
            ));

            // Show line text encoded with font encoder
            let encoded_bytes = encoder.encode_string(&line.text);
            ops.push(Operation::new(
                "Tj",
                vec![PdfObject::String(PdfString::literal(encoded_bytes))],
            ));

            // Reset word spacing if it was altered
            if line.word_spacing > 0.0 {
                ops.push(Operation::new(
                    "Tw",
                    vec![PdfObject::Real(0.0)],
                ));
            }
        }

        // 4. End Text
        ops.push(Operation::new("ET", Vec::new()));

        ops
    }

    /// Recursively traverses node list to replace primary node and prune secondary nodes.
    fn mutate_nodes(
        nodes: &mut Vec<ContentNode>,
        primary_id: NodeId,
        prune_ids: &[NodeId],
        replacement: ContentNode,
        replaced: &mut bool,
    ) {
        let mut i = 0;
        while i < nodes.len() {
            let node = &mut nodes[i];
            match node {
                ContentNode::TextBlock { id, .. } => {
                    if *id == primary_id {
                        nodes[i] = replacement.clone();
                        *replaced = true;
                        i += 1;
                        continue;
                    } else if prune_ids.contains(id) {
                        // Prune secondary nodes merged into this paragraph
                        nodes.remove(i);
                        continue;
                    }
                }
                ContentNode::GraphicsGroup { children, .. } => {
                    Self::mutate_nodes(children, primary_id, prune_ids, replacement.clone(), replaced);
                }
                ContentNode::Instruction { id, .. } => {
                    if prune_ids.contains(id) {
                        nodes.remove(i);
                        continue;
                    }
                }
            }
            i += 1;
        }
    }
}
