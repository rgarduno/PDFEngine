//! Semantic layout reconstruction algorithm.
//!
//! Evaluates page content ASTs, simulates graphics state matrices, extracts
//! positioned glyphs, and hierarchically clusters them into Spans -> Lines -> ParagraphBlocks.

use std::collections::BTreeMap;

use crate::error::{PdfError, PdfResult};
use crate::fonts::{FontMetrics, ResolvedFont, ToUnicodeMap};
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

    /// Registers metrics and `/ToUnicode` maps from a page font dictionary.
    ///
    /// Applied after `with_font`, so a real resource named `F1` replaces the fallback.
    pub fn with_resolved(mut self, faces: &BTreeMap<String, ResolvedFont>) -> Self {
        for (name, face) in faces {
            self.fonts.insert(name.clone(), face.metrics.clone());
            if let Some(cmap) = &face.cmap {
                self.cmaps.insert(name.clone(), cmap.clone());
            }
        }
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

            let font_size = state_stack.current.text_state.font_size;
            let text_advance = if let Some(m) = metrics {
                m.compute_char_advance(char_code, &state_stack.current.text_state)
            } else {
                (500.0 / 1000.0) * font_size
            };

            // Origins are already in page space. The stored advance has to use
            // that same unit, or a scaled text matrix looks like a gap between letters.
            let tm = state_stack.current.text_matrix;
            let ctm = state_stack.current.ctm;
            let page_advance = text_advance * (tm.a * ctm.a + tm.b * ctm.c);
            let trm = state_stack.current.text_rendering_matrix();
            let rendered_size = trm.c.hypot(trm.d);

            let (origin_x, origin_y) = ctm.transform_point(tm.e, tm.f);
            let origin = Point::new(origin_x, origin_y);
            glyphs.push(PositionedGlyph::new(
                char_code,
                unicode,
                origin,
                page_advance,
                font_name.clone(),
                font_size,
                rendered_size,
                node_id,
            ));

            state_stack.current.advance_text(text_advance);
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
            let visual = prev.rendered_size.max(prev.font_size);
            let not_huge_gap = horiz_dist < visual * 2.5;

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
            let visual = cur_line_spans[0].rendered_size.max(cur_line_spans[0].font_size);
            let tolerance = visual * 0.2;

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

            // A normal line break is between 0.9x and 2.2x font line height.
            // A size or font change starts a new block so each block keeps one face.
            let is_consecutive = dy > 0.0 && dy <= avg_height * 2.2;
            let same_face = Self::same_typographic_face(prev_line, &line);
            let horizontal_overlap = prev_line.bbox.intersects(&line.bbox)
                || (line.bbox.min_x <= prev_line.bbox.max_x && line.bbox.max_x >= prev_line.bbox.min_x);

            if is_consecutive && same_face && (horizontal_overlap || cur_para_lines.len() < 2) {
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

    /// Lines share a face when the first span's resource and visual size match.
    fn same_typographic_face(left: &TextLine, right: &TextLine) -> bool {
        match (left.spans.first(), right.spans.first()) {
            (Some(before), Some(after)) => {
                before.font_name == after.font_name
                    && (before.rendered_size - after.rendered_size).abs() < 0.5
            }
            _ => false,
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::ToUnicodeMap;
    use crate::stream::{build_ast_from_operations, ContentStreamTokenizer};

    fn reconstruct_with(stream: &str, builder: impl FnOnce(LayoutReconstructor) -> LayoutReconstructor) -> Vec<ParagraphBlock> {
        let mut tokenizer = ContentStreamTokenizer::new(stream.as_bytes());
        let ops = tokenizer.tokenize_all().expect("content stream tokenizes");
        let ast = build_ast_from_operations(ops);
        builder(LayoutReconstructor::new(&ast))
            .reconstruct()
            .expect("layout reconstructs")
    }

    #[test]
    fn font_size_change_starts_a_new_paragraph() {
        let paragraphs = reconstruct_with(
            "BT\n/F1 16 Tf\n50 700 Tm\n(Rafael) Tj\n0 -20 Td\n/F2 9 Tf\n(Skills) Tj\nET\n",
            |recon| {
                let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
                recon.with_font("F1", metrics.clone()).with_font("F2", metrics)
            },
        );
        assert_eq!(paragraphs.len(), 2);
        assert_eq!(paragraphs[0].text(), "Rafael");
        assert_eq!(paragraphs[1].text(), "Skills");
        assert!((paragraphs[0].rendered_size() - 16.0).abs() < 0.01);
        assert!((paragraphs[1].rendered_size() - 9.0).abs() < 0.01);
    }

    #[test]
    fn scaled_subset_font_decodes_without_inserted_spaces() {
        let mut cmap = ToUnicodeMap::new();
        cmap.insert(0x21, "R".to_string());
        cmap.insert(0x22, "a".to_string());
        cmap.insert(0x23, "f".to_string());
        cmap.insert(0x24, "e".to_string());
        cmap.insert(0x25, "l".to_string());
        let metrics = FontMetrics::new(33, 37, vec![500.0; 5], 500.0);
        let letters = std::str::from_utf8(&[0x21, 0x22, 0x23, 0x22, 0x24, 0x25]).expect("latin-1 bytes");
        let stream = format!(
            "0.2577778 0 0 0.24 -14.33629 601.92 cm\nBT\n67 0 0 67 806.4168 578 Tm\n/TT2 1 Tf\n({letters}) Tj\nET\n"
        );

        let paragraphs = reconstruct_with(&stream, |recon| {
            recon.with_font("TT2", metrics).with_cmap("TT2", cmap)
        });

        assert_eq!(paragraphs.len(), 1);
        assert_eq!(paragraphs[0].text(), "Rafael");
        let glyph = &paragraphs[0].lines[0].spans[0].glyphs[0];
        assert_eq!(glyph.char_code, 0x21);
        assert_eq!(glyph.font_size, 1.0);
        assert!((glyph.rendered_size - 16.08).abs() < 0.01);
        let expected_advance = 0.5 * 67.0 * 0.2577778;
        assert!((glyph.advance - expected_advance).abs() < 0.001);
        assert_eq!(paragraphs[0].lines[0].spans.len(), 1);
    }

    #[test]
    fn identity_ascii_keeps_latin1_and_text_space_advance() {
        let paragraphs = reconstruct_with("BT /F1 12 Tf (Hello) Tj ET\n", |recon| recon);
        assert_eq!(paragraphs.len(), 1);
        assert_eq!(paragraphs[0].text(), "Hello");
        let span = &paragraphs[0].lines[0].spans[0];
        assert_eq!(span.glyphs.len(), 5);
        assert_eq!(span.font_size, 12.0);
        assert!((span.rendered_size - 12.0).abs() < 0.001);
        assert!((span.glyphs[0].advance - 6.0).abs() < 0.001);
        assert_eq!(span.glyphs[0].char_code, b'H' as u32);
    }
}
