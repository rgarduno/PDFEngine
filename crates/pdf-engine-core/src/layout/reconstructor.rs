//! Semantic layout reconstruction algorithm.
//!
//! Evaluates page content ASTs, simulates graphics state matrices, extracts
//! positioned glyphs, and hierarchically clusters them into Spans -> Lines -> ParagraphBlocks.

use std::collections::BTreeMap;

use crate::error::{PdfError, PdfResult};
use crate::fonts::{FontMetrics, ToUnicodeMap};
use crate::layout::geometry::Point;
use crate::layout::glyph::PositionedGlyph;
use crate::layout::line::TextLine;
use crate::layout::paragraph::ParagraphBlock;
use crate::layout::span::TextSpan;
use crate::stream::ast::{ContentAst, ContentNode, NodeId, Operation};
use crate::stream::graphics_state::{GraphicsStateStack, Matrix};

/// Reconstructs high-level semantic paragraphs from a page's Content AST.
pub struct LayoutReconstructor<'a> {
    ast: &'a ContentAst,
    fonts: BTreeMap<String, FontMetrics>,
    cmaps: BTreeMap<String, ToUnicodeMap>,
}

impl<'a> LayoutReconstructor<'a> {
    /// Creates a new layout reconstructor for an AST.
    pub fn new(ast: &'a ContentAst) -> Self {
        Self {
            ast,
            fonts: BTreeMap::new(),
            cmaps: BTreeMap::new(),
        }
    }

    /// Registers a font metrics descriptor under a resource name (e.g. "F1").
    pub fn with_font(mut self, font_name: impl Into<String>, metrics: FontMetrics) -> Self {
        self.fonts.insert(font_name.into(), metrics);
        self
    }

    /// Registers a /ToUnicode CMap under a resource name (e.g. "F1").
    pub fn with_cmap(mut self, font_name: impl Into<String>, cmap: ToUnicodeMap) -> Self {
        self.cmaps.insert(font_name.into(), cmap);
        self
    }

    /// Reconstructs and returns all paragraph blocks from the AST.
    pub fn reconstruct(&self) -> PdfResult<Vec<ParagraphBlock>> {
        let mut state_stack = GraphicsStateStack::new();
        let mut glyphs = Vec::new();

        self.extract_glyphs_from_nodes(&self.ast.nodes, &mut state_stack, &mut glyphs);

        self.cluster_glyphs_into_paragraphs(glyphs)
    }

    /// Recursively visits AST nodes and simulates graphics state to extract glyphs.
    fn extract_glyphs_from_nodes(
        &self,
        nodes: &[ContentNode],
        state_stack: &mut GraphicsStateStack,
        glyphs: &mut Vec<PositionedGlyph>,
    ) {
        for node in nodes {
            match node {
                ContentNode::GraphicsGroup { children, .. } => {
                    state_stack.push();
                    self.extract_glyphs_from_nodes(children, state_stack, glyphs);
                    state_stack.pop();
                }
                ContentNode::TextBlock { id, operations } => {
                    state_stack.current.begin_text();
                    for op in operations {
                        self.process_text_operation(op, *id, state_stack, glyphs);
                    }
                }
                ContentNode::Instruction { operation, .. } => {
                    self.process_general_operation(operation, state_stack);
                }
            }
        }
    }

    /// Evaluates graphics state changes outside text blocks (e.g. `cm`).
    fn process_general_operation(&self, op: &Operation, state_stack: &mut GraphicsStateStack) {
        if op.operator == "cm" && op.operands.len() >= 6 {
            let a = op.operands[0].as_f64().unwrap_or(1.0);
            let b = op.operands[1].as_f64().unwrap_or(0.0);
            let c = op.operands[2].as_f64().unwrap_or(0.0);
            let d = op.operands[3].as_f64().unwrap_or(1.0);
            let e = op.operands[4].as_f64().unwrap_or(0.0);
            let f = op.operands[5].as_f64().unwrap_or(0.0);
            let m = Matrix::new(a, b, c, d, e, f);
            state_stack.current.concat_matrix(&m);
        }
    }

    /// Evaluates text state and text positioning/showing operators inside `BT ... ET`.
    fn process_text_operation(
        &self,
        op: &Operation,
        node_id: NodeId,
        state_stack: &mut GraphicsStateStack,
        glyphs: &mut Vec<PositionedGlyph>,
    ) {
        match op.operator.as_str() {
            "Tf" => {
                if op.operands.len() >= 2 {
                    if let Some(name) = op.operands[0].as_name() {
                        state_stack.current.text_state.font_name = name.trim_start_matches('/').to_string();
                    }
                    if let Some(size) = op.operands[1].as_f64() {
                        state_stack.current.text_state.font_size = size;
                    }
                }
            }
            "Tc" => {
                if let Some(tc) = op.operands.first().and_then(|o| o.as_f64()) {
                    state_stack.current.text_state.char_spacing = tc;
                }
            }
            "Tw" => {
                if let Some(tw) = op.operands.first().and_then(|o| o.as_f64()) {
                    state_stack.current.text_state.word_spacing = tw;
                }
            }
            "Tz" => {
                if let Some(tz) = op.operands.first().and_then(|o| o.as_f64()) {
                    state_stack.current.text_state.horizontal_scaling = tz;
                }
            }
            "TL" => {
                if let Some(tl) = op.operands.first().and_then(|o| o.as_f64()) {
                    state_stack.current.text_state.leading = tl;
                }
            }
            "Td" => {
                if op.operands.len() >= 2 {
                    let tx = op.operands[0].as_f64().unwrap_or(0.0);
                    let ty = op.operands[1].as_f64().unwrap_or(0.0);
                    state_stack.current.move_text_position(tx, ty);
                }
            }
            "TD" => {
                if op.operands.len() >= 2 {
                    let tx = op.operands[0].as_f64().unwrap_or(0.0);
                    let ty = op.operands[1].as_f64().unwrap_or(0.0);
                    state_stack.current.text_state.leading = -ty;
                    state_stack.current.move_text_position(tx, ty);
                }
            }
            "Tm" => {
                if op.operands.len() >= 6 {
                    let a = op.operands[0].as_f64().unwrap_or(1.0);
                    let b = op.operands[1].as_f64().unwrap_or(0.0);
                    let c = op.operands[2].as_f64().unwrap_or(0.0);
                    let d = op.operands[3].as_f64().unwrap_or(1.0);
                    let e = op.operands[4].as_f64().unwrap_or(0.0);
                    let f = op.operands[5].as_f64().unwrap_or(0.0);
                    state_stack.current.set_text_matrix(Matrix::new(a, b, c, d, e, f));
                } else if op.operands.len() >= 2 {
                    let e = op.operands[0].as_f64().unwrap_or(0.0);
                    let f = op.operands[1].as_f64().unwrap_or(0.0);
                    state_stack.current.set_text_matrix(Matrix::new(1.0, 0.0, 0.0, 1.0, e, f));
                }
            }
            "T*" => {
                let leading = state_stack.current.text_state.leading;
                state_stack.current.move_text_position(0.0, -leading);
            }
            "Tj" => {
                if let Some(crate::cos::PdfObject::String(s)) = op.operands.first() {
                    self.show_string_glyphs(&s.bytes, node_id, state_stack, glyphs);
                }
            }
            "TJ" => {
                if let Some(crate::cos::PdfObject::Array(items)) = op.operands.first() {
                    for item in items {
                        match item {
                            crate::cos::PdfObject::String(s) => {
                                self.show_string_glyphs(&s.bytes, node_id, state_stack, glyphs);
                            }
                            crate::cos::PdfObject::Integer(k) => {
                                let dx = FontMetrics::compute_kerning_displacement(*k as f64, &state_stack.current.text_state);
                                state_stack.current.advance_text(dx);
                            }
                            crate::cos::PdfObject::Real(k) => {
                                let dx = FontMetrics::compute_kerning_displacement(*k, &state_stack.current.text_state);
                                state_stack.current.advance_text(dx);
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Renders a byte sequence to individual positioned glyphs.
    fn show_string_glyphs(
        &self,
        bytes: &[u8],
        node_id: NodeId,
        state_stack: &mut GraphicsStateStack,
        glyphs: &mut Vec<PositionedGlyph>,
    ) {
        let font_name = state_stack.current.text_state.font_name.clone();
        let metrics = self.fonts.get(&font_name);
        let cmap = self.cmaps.get(&font_name);

        for &b in bytes {
            let char_code = b as u32;

            // Resolve Unicode
            let unicode = if let Some(cmap) = cmap {
                cmap.decode_code(char_code)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| (b as char).to_string())
            } else {
                (b as char).to_string()
            };

            // Calculate advance
            let advance = if let Some(m) = metrics {
                m.compute_char_advance(char_code, &state_stack.current.text_state)
            } else {
                // Default fallback: 500/1000 width
                (500.0 / 1000.0) * state_stack.current.text_state.font_size
            };

            // Transform origin to page device space
            let (origin_x, origin_y) = state_stack.current.ctm.transform_point(
                state_stack.current.text_matrix.e,
                state_stack.current.text_matrix.f,
            );

            let origin = Point::new(origin_x, origin_y);
            glyphs.push(PositionedGlyph::new(
                char_code,
                unicode,
                origin,
                advance,
                font_name.clone(),
                state_stack.current.text_state.font_size,
                node_id,
            ));

            // Advance text matrix
            state_stack.current.advance_text(advance);
        }
    }

    /// Hierarchically clusters flat glyph runs into structured paragraphs.
    fn cluster_glyphs_into_paragraphs(
        &self,
        glyphs: Vec<PositionedGlyph>,
    ) -> PdfResult<Vec<ParagraphBlock>> {
        if glyphs.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Group glyphs into spans (consecutive glyphs on same baseline with same style)
        let mut spans = Vec::new();
        let mut cur_span_glyphs = Vec::new();

        for glyph in glyphs {
            if cur_span_glyphs.is_empty() {
                cur_span_glyphs.push(glyph);
                continue;
            }

            let prev: &PositionedGlyph = cur_span_glyphs.last().ok_or_else(|| {
                PdfError::LayoutError("glyph span has no preceding glyph".into())
            })?;
            let same_baseline = (prev.origin.y - glyph.origin.y).abs() < 1.0;
            let same_font = prev.font_name == glyph.font_name;
            let same_size = (prev.font_size - glyph.font_size).abs() < 0.5;
            let same_node = prev.ast_node_id == glyph.ast_node_id;
            let horiz_dist = glyph.origin.x - (prev.origin.x + prev.advance);
            let not_huge_gap = horiz_dist < prev.font_size * 2.5;

            if same_baseline && same_font && same_size && same_node && not_huge_gap {
                cur_span_glyphs.push(glyph);
            } else {
                if let Some(span) = TextSpan::from_glyphs(std::mem::take(&mut cur_span_glyphs)) {
                    spans.push(span);
                }
                cur_span_glyphs.push(glyph);
            }
        }
        if let Some(span) = TextSpan::from_glyphs(cur_span_glyphs) {
            spans.push(span);
        }

        // 2. Group spans into lines (spans sharing the same baseline within vertical tolerance)
        let mut lines = Vec::new();
        // Sort spans by y descending (top to bottom reading order in PDF) and then x ascending
        spans.sort_by(|a, b| {
            b.baseline_y
                .partial_cmp(&a.baseline_y)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.bbox.min_x.partial_cmp(&b.bbox.min_x).unwrap_or(std::cmp::Ordering::Equal))
        });

        let mut cur_line_spans = Vec::new();
        for span in spans {
            if cur_line_spans.is_empty() {
                cur_line_spans.push(span);
                continue;
            }

            let line_base = cur_line_spans[0].baseline_y;
            let tolerance = cur_line_spans[0].font_size * 0.2;

            if (span.baseline_y - line_base).abs() <= tolerance {
                cur_line_spans.push(span);
            } else {
                if let Some(line) = TextLine::from_spans(std::mem::take(&mut cur_line_spans)) {
                    lines.push(line);
                }
                cur_line_spans.push(span);
            }
        }
        if let Some(line) = TextLine::from_spans(cur_line_spans) {
            lines.push(line);
        }

        // 3. Group lines into paragraphs (consecutive lines with consistent leading)
        let mut paragraphs = Vec::new();
        let mut cur_para_lines = Vec::new();
        let mut next_para_id = 0;

        for line in lines {
            if cur_para_lines.is_empty() {
                cur_para_lines.push(line);
                continue;
            }

            let prev_line: &TextLine = cur_para_lines.last().ok_or_else(|| {
                PdfError::LayoutError("paragraph has no preceding line".into())
            })?;
            let dy = prev_line.baseline_y - line.baseline_y;
            let avg_height = prev_line.bbox.height().max(line.bbox.height());

            // A normal line break is between 0.9x and 2.2x font line height
            let is_consecutive = dy > 0.0 && dy <= avg_height * 2.2;
            let horizontal_overlap = prev_line.bbox.intersects(&line.bbox)
                || (line.bbox.min_x <= prev_line.bbox.max_x && line.bbox.max_x >= prev_line.bbox.min_x);

            if is_consecutive && (horizontal_overlap || cur_para_lines.len() < 2) {
                cur_para_lines.push(line);
            } else {
                let source_nodes = Self::collect_source_node_ids(&cur_para_lines);
                if let Some(para) = ParagraphBlock::new(next_para_id, std::mem::take(&mut cur_para_lines), source_nodes) {
                    paragraphs.push(para);
                    next_para_id += 1;
                }
                cur_para_lines.push(line);
            }
        }

        if !cur_para_lines.is_empty() {
            let source_nodes = Self::collect_source_node_ids(&cur_para_lines);
            if let Some(para) = ParagraphBlock::new(next_para_id, cur_para_lines, source_nodes) {
                paragraphs.push(para);
            }
        }

        Ok(paragraphs)
    }

    fn collect_source_node_ids(lines: &[TextLine]) -> Vec<NodeId> {
        let mut set = std::collections::BTreeSet::new();
        for line in lines {
            for span in &line.spans {
                for g in &span.glyphs {
                    set.insert(g.ast_node_id);
                }
            }
        }
        set.into_iter().collect()
    }
}
