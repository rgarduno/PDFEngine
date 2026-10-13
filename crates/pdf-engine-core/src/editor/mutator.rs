//! Surgical in-place Content AST mutator.
//!
//! Replaces target text blocks with freshly re-flowed text operations
//! while preserving 100% of surrounding vector paths, images, and graphics states.

use crate::cos::object::{PdfName, PdfObject, PdfString};
use crate::editor::reflow::ReflowEngine;
use crate::editor::style::{ParagraphStyle, PreparedParagraphStyle};
use crate::error::{PdfError, PdfResult};
use crate::fonts::{FontEncoder, FontMetrics};
use crate::layout::paragraph::ParagraphBlock;
use crate::stream::ast::{ContentAst, ContentNode, NodeId, Operation};
use crate::stream::graphics_state::TextState;

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
        let (resource, size) = Self::span_source(target_block)?;
        let prepared = ParagraphStyle::default().prepare(
            &resource,
            size,
            target_block.leading,
            target_block.alignment,
            "",
            400,
            "normal",
        )?;
        Self::edit_paragraph_prepared_with_encoder(
            ast,
            target_block,
            new_text,
            metrics,
            encoder,
            &prepared,
        )
    }

    /// Replaces a paragraph using a resolved style. An empty style matches `edit_paragraph`.
    pub fn edit_paragraph_prepared(
        ast: &mut ContentAst,
        target_block: &ParagraphBlock,
        new_text: &str,
        metrics: &FontMetrics,
        prepared: &PreparedParagraphStyle,
    ) -> PdfResult<()> {
        let encoder = FontEncoder::new();
        Self::edit_paragraph_prepared_with_encoder(
            ast,
            target_block,
            new_text,
            metrics,
            &encoder,
            prepared,
        )
    }

    fn edit_paragraph_prepared_with_encoder(
        ast: &mut ContentAst,
        target_block: &ParagraphBlock,
        new_text: &str,
        metrics: &FontMetrics,
        encoder: &FontEncoder,
        prepared: &PreparedParagraphStyle,
    ) -> PdfResult<()> {
        if target_block.source_node_ids.is_empty() {
            return Err(PdfError::LayoutError(
                "Target paragraph has no associated AST source nodes".to_string(),
            ));
        }

        let primary_node_id = target_block.source_node_ids[0];
        let first_line = target_block.lines.first().ok_or_else(|| {
            PdfError::LayoutError("Target paragraph contains no lines".to_string())
        })?;
        if first_line.spans.is_empty() {
            return Err(PdfError::LayoutError(
                "Target paragraph contains no styled spans".to_string(),
            ));
        }

        let mut text_state = TextState::default();
        text_state.font_name = prepared.font_resource.clone();
        text_state.font_size = prepared.size;

        let max_width = target_block.bbox.width().max(50.0);
        let reflow_lines = ReflowEngine::reflow(
            new_text,
            max_width,
            metrics,
            &text_state,
            prepared.alignment,
            prepared.leading,
        );

        let new_operations = Self::synthesize_text_operations(
            &reflow_lines,
            target_block.bbox.min_x,
            first_line.baseline_y,
            &prepared.font_resource,
            prepared.size,
            prepared.color,
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

    fn span_source(target_block: &ParagraphBlock) -> PdfResult<(String, f64)> {
        if target_block.source_node_ids.is_empty() {
            return Err(PdfError::LayoutError(
                "Target paragraph has no associated AST source nodes".to_string(),
            ));
        }
        let first_line = target_block.lines.first().ok_or_else(|| {
            PdfError::LayoutError("Target paragraph contains no lines".to_string())
        })?;
        let first_span = first_line.spans.first().ok_or_else(|| {
            PdfError::LayoutError("Target paragraph contains no styled spans".to_string())
        })?;
        Ok((first_span.font_name.clone(), first_span.font_size))
    }

    /// Synthesizes ISO 32000 text operations. Color is wrapped outside `BT`/`ET`.
    fn synthesize_text_operations(
        lines: &[crate::editor::reflow::ReflowLine],
        origin_x: f64,
        top_baseline_y: f64,
        font_name: &str,
        font_size: f64,
        color: Option<[f64; 3]>,
        encoder: &FontEncoder,
    ) -> Vec<Operation> {
        let mut ops = Vec::new();

        if let Some([red, green, blue]) = color {
            ops.push(Operation::new("q", Vec::new()));
            ops.push(Operation::new(
                "rg",
                vec![
                    PdfObject::Real(red),
                    PdfObject::Real(green),
                    PdfObject::Real(blue),
                ],
            ));
        }

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
                ops.push(Operation::new("Tw", vec![PdfObject::Real(0.0)]));
            }
        }

        // 4. End Text
        ops.push(Operation::new("ET", Vec::new()));
        if color.is_some() {
            ops.push(Operation::new("Q", Vec::new()));
        }

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
                    Self::mutate_nodes(
                        children,
                        primary_id,
                        prune_ids,
                        replacement.clone(),
                        replaced,
                    );
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
