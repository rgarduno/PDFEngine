//! Intelligent Form Field Detection according to ISO 32000-1.
//!
//! Evaluates page vector graphics (underlines, bounding boxes, checkboxes) and adjacent
//! semantic text labels to detect form field candidates, infer field types (Text, Checkbox,
//! Signature), calculate confidence scores, and automatically synthesize interactive AcroForms.

use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::forms::builder::{create_form_field, FormFieldCreateOptions};
use crate::forms::reader::extract_document_forms;
use crate::forms::types::{FormField, FormFieldType};
use crate::layout::geometry::{Point, Rect};
use crate::layout::line::TextLine;
use crate::layout::reconstructor::LayoutReconstructor;
use crate::stream::ast::{ContentAst, ContentNode, Operation};
use crate::stream::graphics_state::{GraphicsStateStack, Matrix};
use crate::stream::parser::{build_ast_from_operations, ContentStreamTokenizer};

/// A detected form field candidate before being converted into an interactive AcroForm widget.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedFormFieldCandidate {
    /// 1-based unique identifier for this candidate on the page.
    pub candidate_id: usize,
    /// Inferred form field type.
    pub field_type: FormFieldType,
    /// Proposed field identifier for AcroForm /T dictionary entry.
    pub suggested_name: String,
    /// Detected descriptive text label or tooltip, if present.
    pub label: Option<String>,
    /// Bounding rectangle in page coordinates.
    pub rect: Rect,
    /// Confidence score in range [0.0, 1.0].
    pub confidence: f64,
}

/// A horizontal line segment candidate for underlines.
#[derive(Debug, Clone, Copy, PartialEq)]
struct HorizLine {
    y: f64,
    min_x: f64,
    max_x: f64,
}

/// A bounding box candidate extracted from vectors.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BoxCandidate {
    rect: Rect,
    is_square: bool,
}

/// Maximum vector primitives processed per page to guard resources.
const MAX_DETECT_VECTORS: usize = 8192;

/// Detects interactive form field candidates on the specified page (1-indexed).
pub fn detect_form_field_candidates(
    doc: &mut PdfDocument,
    page_number: usize,
) -> PdfResult<Vec<DetectedFormFieldCandidate>> {
    let pages = doc.get_pages()?;
    if page_number == 0 || page_number > pages.len() {
        return Err(PdfError::InvalidPageNumber {
            page: page_number,
            total: pages.len(),
        });
    }

    let page_id = pages[page_number - 1];
    let content_bytes = doc.get_page_content_bytes(page_id)?;
    if content_bytes.is_empty() {
        return Ok(Vec::new());
    }

    let mut tokenizer = ContentStreamTokenizer::new(&content_bytes);
    let operations = tokenizer.tokenize_all().unwrap_or_default();
    let ast = build_ast_from_operations(operations);

    // 1. Reconstruct text layout to collect text lines for label association
    let reconstructor = LayoutReconstructor::new(&ast);
    let paragraphs = reconstructor.reconstruct().unwrap_or_default();
    let mut text_lines: Vec<TextLine> = Vec::new();
    for p in &paragraphs {
        for line in &p.lines {
            if !line.text.trim().is_empty() {
                text_lines.push(line.clone());
            }
        }
    }

    // 2. Query existing AcroForm fields on this page to avoid duplicating already active fields
    let existing_fields = extract_document_forms(doc).unwrap_or_default();
    let page_existing: Vec<&FormField> = existing_fields
        .iter()
        .filter(|f| f.page_number == page_number)
        .collect();

    // 3. Extract vector geometries (underlines, bounding boxes, checkboxes)
    let (horiz_lines, mut box_candidates) = extract_geometries(&ast)?;

    // 4. Convert unboxed horizontal underlines into input field boxes
    for h in horiz_lines {
        let is_box_border = box_candidates.iter().any(|b| {
            (b.rect.min_y - h.y).abs() <= 2.0
                && (b.rect.min_x - h.min_x).abs() <= 8.0
                && (b.rect.max_x - h.max_x).abs() <= 8.0
        });

        if !is_box_border {
            // An underline defines the baseline of an input field sitting above it
            let rect = Rect::new(h.min_x, h.y, h.max_x, h.y + 18.0);
            box_candidates.push(BoxCandidate {
                rect,
                is_square: false,
            });
        }
    }

    // 5. Deduplicate overlapping or redundant box candidates
    let mut deduplicated: Vec<BoxCandidate> = Vec::new();
    for b in box_candidates {
        let already_present = deduplicated.iter().any(|existing| {
            calculate_iou(&existing.rect, &b.rect) > 0.55
                || ((existing.rect.min_x - b.rect.min_x).abs() <= 3.0
                    && (existing.rect.max_x - b.rect.max_x).abs() <= 3.0
                    && (existing.rect.min_y - b.rect.min_y).abs() <= 3.0
                    && (existing.rect.max_y - b.rect.max_y).abs() <= 3.0)
        });

        if !already_present {
            // Discard candidate if it overlaps with an existing active AcroForm widget
            let overlaps_existing = page_existing.iter().any(|f| {
                calculate_iou(&f.rect, &b.rect) > 0.4
                    || (b.rect.center_x() >= f.rect.min_x
                        && b.rect.center_x() <= f.rect.max_x
                        && ((b.rect.min_y + b.rect.max_y) * 0.5) >= f.rect.min_y
                        && ((b.rect.min_y + b.rect.max_y) * 0.5) <= f.rect.max_y)
            });

            if !overlaps_existing {
                deduplicated.push(b);
            }
        }
    }

    // Sort spatially: Top-to-bottom (descending y), then Left-to-right (ascending x)
    deduplicated.sort_by(|a, b| {
        let y_diff = b
            .rect
            .max_y
            .partial_cmp(&a.rect.max_y)
            .unwrap_or(std::cmp::Ordering::Equal);
        if y_diff == std::cmp::Ordering::Equal || (b.rect.max_y - a.rect.max_y).abs() < 8.0 {
            a.rect
                .min_x
                .partial_cmp(&b.rect.min_x)
                .unwrap_or(std::cmp::Ordering::Equal)
        } else {
            y_diff
        }
    });

    // 6. Associate labels, infer types and calculate confidence
    let mut candidates: Vec<DetectedFormFieldCandidate> = Vec::new();
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (idx, b) in deduplicated.into_iter().enumerate() {
        let candidate_id = idx + 1;
        let best_label = find_associated_label(&b, &text_lines);
        let (field_type, confidence) = classify_candidate(&b, best_label.as_deref());

        let base_name = if let Some(ref l) = best_label {
            let sanitized = sanitize_identifier(l);
            if sanitized.is_empty() {
                format!("Field_{}", candidate_id)
            } else {
                sanitized
            }
        } else if b.is_square {
            format!("Checkbox_{}", candidate_id)
        } else if field_type == FormFieldType::Signature {
            format!("Signature_{}", candidate_id)
        } else {
            format!("TextField_{}", candidate_id)
        };

        let mut unique_name = base_name.clone();
        let mut counter = 2;
        while used_names.contains(&unique_name) {
            unique_name = format!("{}_{}", base_name, counter);
            counter += 1;
        }
        used_names.insert(unique_name.clone());

        candidates.push(DetectedFormFieldCandidate {
            candidate_id,
            field_type,
            suggested_name: unique_name,
            label: best_label,
            rect: b.rect,
            confidence,
        });
    }

    Ok(candidates)
}

/// Applies a set of detected form field candidates into interactive AcroForms on the page.
pub fn apply_detected_form_fields(
    doc: &mut PdfDocument,
    page_number: usize,
    candidates: &[DetectedFormFieldCandidate],
) -> PdfResult<Vec<FormField>> {
    let mut created_fields: Vec<FormField> = Vec::new();
    let existing_doc_fields = extract_document_forms(doc).unwrap_or_default();
    let mut existing_names: std::collections::HashSet<String> =
        existing_doc_fields.into_iter().map(|f| f.name).collect();

    for cand in candidates {
        // Ensure name does not conflict with existing fields
        let mut final_name = cand.suggested_name.clone();
        let mut counter = 2;
        while existing_names.contains(&final_name) {
            final_name = format!("{}_{}", cand.suggested_name, counter);
            counter += 1;
        }
        existing_names.insert(final_name.clone());

        let is_multiline = cand.field_type == FormFieldType::Text && cand.rect.height() >= 45.0;

        let opts = FormFieldCreateOptions {
            name: final_name,
            field_type: cand.field_type,
            rect: cand.rect,
            value: None,
            default_value: None,
            alt_name: cand.label.clone(),
            options: None,
            is_read_only: false,
            is_required: false,
            is_multiline,
            max_length: None,
            font_size: Some(12.0),
        };

        let created = create_form_field(doc, page_number, &opts)?;
        created_fields.push(created);
    }

    Ok(created_fields)
}

/// Extracts horizontal lines and candidate boxes by walking AST and tracking CTM.
fn extract_geometries(ast: &ContentAst) -> PdfResult<(Vec<HorizLine>, Vec<BoxCandidate>)> {
    let mut horiz = Vec::new();
    let mut boxes = Vec::new();
    let mut state_stack = GraphicsStateStack::new();

    scan_nodes(&ast.nodes, &mut state_stack, &mut horiz, &mut boxes)?;

    // Merge collinear horizontal line segments
    merge_collinear_horiz_lines(&mut horiz);

    // Keep horizontal underlines of reasonable width [25.0, 550.0]
    horiz.retain(|h| {
        let w = h.max_x - h.min_x;
        w >= 25.0 && w <= 550.0
    });

    Ok((horiz, boxes))
}

fn scan_nodes(
    nodes: &[ContentNode],
    state_stack: &mut GraphicsStateStack,
    horiz: &mut Vec<HorizLine>,
    boxes: &mut Vec<BoxCandidate>,
) -> PdfResult<()> {
    let mut current_point: Option<Point> = None;
    let mut subpath_start: Option<Point> = None;
    let mut subpath: Vec<Point> = Vec::new();

    for node in nodes {
        match node {
            ContentNode::GraphicsGroup { children, .. } => {
                state_stack.push();
                scan_nodes(children, state_stack, horiz, boxes)?;
                state_stack.pop();
            }
            ContentNode::Instruction { operation, .. } => {
                process_op(
                    operation,
                    state_stack,
                    &mut current_point,
                    &mut subpath_start,
                    &mut subpath,
                    horiz,
                    boxes,
                )?;
            }
            ContentNode::TextBlock { .. } => {}
        }
    }
    Ok(())
}

fn process_op(
    op: &Operation,
    state_stack: &mut GraphicsStateStack,
    current_point: &mut Option<Point>,
    subpath_start: &mut Option<Point>,
    subpath: &mut Vec<Point>,
    horiz: &mut Vec<HorizLine>,
    boxes: &mut Vec<BoxCandidate>,
) -> PdfResult<()> {
    if horiz.len() + boxes.len() >= MAX_DETECT_VECTORS {
        return Ok(());
    }

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
                state_stack
                    .current
                    .concat_matrix(&Matrix::new(a, b, c, d, e, f));
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
                subpath.clear();
                subpath.push(p);
            }
        }
        "l" => {
            if op.operands.len() >= 2 {
                let x = op.operands[0].as_f64().unwrap_or(0.0);
                let y = op.operands[1].as_f64().unwrap_or(0.0);
                let (px, py) = ctm.transform_point(x, y);
                let p2 = Point::new(px, py);

                if let Some(p1) = *current_point {
                    subpath.push(p2);
                    if (p1.y - p2.y).abs() <= 1.5 && (p1.x - p2.x).abs() >= 10.0 {
                        horiz.push(HorizLine {
                            y: (p1.y + p2.y) * 0.5,
                            min_x: p1.x.min(p2.x),
                            max_x: p1.x.max(p2.x),
                        });
                    }
                }
                *current_point = Some(p2);
            }
        }
        "h" | "s" | "b" | "B" | "S" | "f" | "F" => {
            if let (Some(p1), Some(start)) = (*current_point, *subpath_start) {
                if (p1.x - start.x).abs() > 0.5 || (p1.y - start.y).abs() > 0.5 {
                    if (p1.y - start.y).abs() <= 1.5 && (p1.x - start.x).abs() >= 10.0 {
                        horiz.push(HorizLine {
                            y: (p1.y + start.y) * 0.5,
                            min_x: p1.x.min(start.x),
                            max_x: p1.x.max(start.x),
                        });
                    }
                    subpath.push(start);
                }
            }

            // Check if subpath forms an axis-aligned rectangle
            if subpath.len() >= 4 {
                if let Some(candidate) = evaluate_subpath_box(subpath) {
                    boxes.push(candidate);
                }
            }

            *current_point = *subpath_start;
            subpath.clear();
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

                if width >= 25.0 && height <= 3.0 {
                    // Underline represented via thin filled rectangle
                    horiz.push(HorizLine {
                        y: (min_y + max_y) * 0.5,
                        min_x,
                        max_x,
                    });
                } else if width >= 8.0 && height >= 8.0 {
                    let is_square =
                        (width - height).abs() <= 4.0 && width <= 24.0 && height <= 24.0;
                    if is_square || (width >= 40.0 && height >= 12.0 && height <= 120.0) {
                        boxes.push(BoxCandidate {
                            rect: Rect::new(min_x, min_y, max_x, max_y),
                            is_square,
                        });
                    }
                }
            }
        }
        _ => {}
    }

    Ok(())
}

/// Checks if a sequence of points in a subpath forms an axis-aligned box.
fn evaluate_subpath_box(pts: &[Point]) -> Option<BoxCandidate> {
    if pts.len() < 4 {
        return None;
    }

    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for p in pts {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }

    let width = max_x - min_x;
    let height = max_y - min_y;

    if width < 8.0 || height < 8.0 {
        return None;
    }

    let is_square = (width - height).abs() <= 4.0 && width <= 24.0 && height <= 24.0;
    if is_square || (width >= 40.0 && height >= 12.0 && height <= 120.0) {
        Some(BoxCandidate {
            rect: Rect::new(min_x, min_y, max_x, max_y),
            is_square,
        })
    } else {
        None
    }
}

/// Merges collinear horizontal lines whose endpoints nearly touch.
fn merge_collinear_horiz_lines(lines: &mut Vec<HorizLine>) {
    if lines.is_empty() {
        return;
    }

    lines.sort_by(|a, b| {
        a.y.partial_cmp(&b.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.min_x
                    .partial_cmp(&b.min_x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let mut merged = Vec::new();
    let mut current = lines[0];

    for next in lines.iter().skip(1) {
        if (current.y - next.y).abs() <= 1.5 && next.min_x <= current.max_x + 4.0 {
            current.max_x = current.max_x.max(next.max_x);
            current.min_x = current.min_x.min(next.min_x);
        } else {
            merged.push(current);
            current = *next;
        }
    }
    merged.push(current);
    *lines = merged;
}

/// Finds the most semantically relevant text label adjacent to a form field candidate.
fn find_associated_label(b: &BoxCandidate, lines: &[TextLine]) -> Option<String> {
    let box_center_y = (b.rect.min_y + b.rect.max_y) * 0.5;

    let mut best_label: Option<(String, f64)> = None;

    for line in lines {
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }

        let line_center_y = (line.bbox.min_y + line.bbox.max_y) * 0.5;

        // 1. Left Label: text immediately to the left of the field
        if line.bbox.max_x <= b.rect.min_x + 5.0
            && b.rect.min_x - line.bbox.max_x <= 150.0
            && ((line_center_y - box_center_y).abs() <= 14.0
                || (line.baseline_y >= b.rect.min_y - 4.0 && line.baseline_y <= b.rect.max_y + 4.0))
        {
            let dist = b.rect.min_x - line.bbox.max_x;
            if best_label.as_ref().is_none_or(|(_, d)| dist < *d) {
                best_label = Some((text.to_string(), dist));
            }
        }

        // 2. Top Label: text directly above the field
        if line.bbox.min_y >= b.rect.max_y - 4.0
            && line.bbox.min_y - b.rect.max_y <= 28.0
            && line.bbox.min_x >= b.rect.min_x - 30.0
            && line.bbox.min_x <= b.rect.max_x
        {
            let dist = (line.bbox.min_y - b.rect.max_y) * 1.5;
            if best_label.as_ref().is_none_or(|(_, d)| dist < *d) {
                best_label = Some((text.to_string(), dist));
            }
        }

        // 3. Right Label: for Checkboxes only (e.g. [ ] Agree to terms)
        if b.is_square
            && line.bbox.min_x >= b.rect.max_x - 4.0
            && line.bbox.min_x - b.rect.max_x <= 160.0
            && (line_center_y - box_center_y).abs() <= 12.0
        {
            let dist = line.bbox.min_x - b.rect.max_x;
            if best_label.as_ref().is_none_or(|(_, d)| dist < *d) {
                best_label = Some((text.to_string(), dist));
            }
        }
    }

    best_label
        .map(|(t, _)| clean_label_text(&t))
        .filter(|t| !t.is_empty())
}

/// Cleans raw label text by trimming trailing colons, underlines, dots, and hyphens.
fn clean_label_text(raw: &str) -> String {
    let mut trimmed = raw.trim();
    while let Some(last) = trimmed.chars().last() {
        if last == ':'
            || last == '_'
            || last == '.'
            || last == '-'
            || last == ';'
            || last.is_whitespace()
        {
            trimmed = &trimmed[..trimmed.len() - last.len_utf8()];
        } else {
            break;
        }
    }
    trimmed.trim().to_string()
}

/// Converts a label into a valid identifier suitable for AcroForm field /T name.
fn sanitize_identifier(label: &str) -> String {
    let mut ident = String::new();
    let mut capitalize_next = true;

    for ch in label.chars() {
        if ch.is_alphanumeric() {
            if capitalize_next {
                for c in ch.to_uppercase() {
                    ident.push(c);
                }
                capitalize_next = false;
            } else {
                ident.push(ch);
            }
        } else {
            capitalize_next = true;
        }
    }

    ident
}

/// Classifies field type and computes a confidence score from geometry and label heuristics.
fn classify_candidate(b: &BoxCandidate, label: Option<&str>) -> (FormFieldType, f64) {
    if b.is_square {
        let confidence = if label.is_some() { 0.90 } else { 0.80 };
        return (FormFieldType::Checkbox, confidence);
    }

    if let Some(l) = label {
        let lower = l.to_lowercase();

        if lower.contains("sign")
            || lower.contains("firma")
            || lower.contains("rubrica")
            || lower.contains("signature")
            || lower.contains("witness")
        {
            return (FormFieldType::Signature, 0.92);
        }

        if lower.contains("date")
            || lower.contains("fecha")
            || lower.contains("dob")
            || lower.contains("birth")
            || lower.contains("dated")
        {
            return (FormFieldType::Text, 0.88);
        }

        if lower.contains("name")
            || lower.contains("nombre")
            || lower.contains("email")
            || lower.contains("correo")
            || lower.contains("phone")
            || lower.contains("tel")
            || lower.contains("cell")
            || lower.contains("address")
            || lower.contains("direccion")
            || lower.contains("city")
            || lower.contains("state")
            || lower.contains("zip")
            || lower.contains("postal")
            || lower.contains("ssn")
            || lower.contains("title")
            || lower.contains("notes")
            || lower.contains("comments")
        {
            return (FormFieldType::Text, 0.88);
        }

        return (FormFieldType::Text, 0.80);
    }

    // Unlabelled candidates
    let confidence = if b.rect.height() >= 12.0 && b.rect.width() >= 40.0 {
        0.65
    } else {
        0.55
    };
    (FormFieldType::Text, confidence)
}

/// Calculates Intersection over Union (IoU) of two bounding boxes.
fn calculate_iou(a: &Rect, b: &Rect) -> f64 {
    let inter_min_x = a.min_x.max(b.min_x);
    let inter_max_x = a.max_x.min(b.max_x);
    let inter_min_y = a.min_y.max(b.min_y);
    let inter_max_y = a.max_y.min(b.max_y);

    if inter_min_x >= inter_max_x || inter_min_y >= inter_max_y {
        return 0.0;
    }

    let inter_area = (inter_max_x - inter_min_x) * (inter_max_y - inter_min_y);
    let area_a = a.width() * a.height();
    let area_b = b.width() * b.height();
    let union_area = area_a + area_b - inter_area;

    if union_area <= 0.0 {
        0.0
    } else {
        inter_area / union_area
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cos::PdfDocument;

    /// Generates a test PDF with synthetic vector form fields and text labels.
    fn create_test_form_detection_pdf() -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

        // 1 0 obj: Catalog
        let off1 = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

        // 2 0 obj: Pages
        let off2 = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

        // 3 0 obj: Page
        let off3 = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n",
        );

        // 4 0 obj: Contents with form vector primitives and labels
        let content = b"q\n\
BT /F1 12 Tf 50 700 Td (Full Name:) Tj ET\n\
120 696 200 20 re S\n\
\n\
BT /F1 12 Tf 50 650 Td (Signature:) Tj ET\n\
120 648 m 320 648 l S\n\
\n\
50 600 14 14 re S\n\
BT /F1 12 Tf 72 602 Td (I agree to terms) Tj ET\n\
Q\n";

        let off4 = pdf.len();
        pdf.extend_from_slice(
            format!("4 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes(),
        );
        pdf.extend_from_slice(content);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");

        // XRef
        let xref_off = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n");
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off3).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off4).as_bytes());

        // Trailer
        pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n");
        pdf.extend_from_slice(format!("{}\n%%EOF\n", xref_off).as_bytes());

        pdf
    }

    #[test]
    fn test_detect_form_fields_synthetic() {
        let bytes = create_test_form_detection_pdf();
        let mut doc = PdfDocument::load(&bytes).expect("Failed to parse test PDF");

        let candidates = detect_form_field_candidates(&mut doc, 1).expect("Detection failed");
        assert_eq!(candidates.len(), 3, "Expected 3 candidates detected");

        // Candidate 1: Full Name text field
        let name_cand = &candidates[0];
        assert_eq!(name_cand.field_type, FormFieldType::Text);
        assert_eq!(name_cand.suggested_name, "FullName");
        assert_eq!(name_cand.label.as_deref(), Some("Full Name"));
        assert!(name_cand.confidence >= 0.85);

        // Candidate 2: Signature field
        let sig_cand = &candidates[1];
        assert_eq!(sig_cand.field_type, FormFieldType::Signature);
        assert_eq!(sig_cand.suggested_name, "Signature");
        assert_eq!(sig_cand.label.as_deref(), Some("Signature"));
        assert!(sig_cand.confidence >= 0.90);

        // Candidate 3: Checkbox field
        let check_cand = &candidates[2];
        assert_eq!(check_cand.field_type, FormFieldType::Checkbox);
        assert_eq!(check_cand.suggested_name, "IAgreeToTerms");
        assert_eq!(check_cand.label.as_deref(), Some("I agree to terms"));
        assert!(check_cand.confidence >= 0.85);
    }

    #[test]
    fn test_apply_detected_form_fields() {
        let bytes = create_test_form_detection_pdf();
        let mut doc = PdfDocument::load(&bytes).expect("Failed to parse test PDF");

        let candidates = detect_form_field_candidates(&mut doc, 1).expect("Detection failed");
        let created =
            apply_detected_form_fields(&mut doc, 1, &candidates).expect("Application failed");

        assert_eq!(created.len(), 3);
        assert_eq!(created[0].name, "FullName");
        assert_eq!(created[1].name, "Signature");
        assert_eq!(created[2].name, "IAgreeToTerms");

        // Verify fields are now extracted as AcroForm fields
        let forms = extract_document_forms(&mut doc).expect("Extraction failed");
        assert_eq!(forms.len(), 3);

        // Re-running detection should not detect overlapping fields anymore
        let re_detected = detect_form_field_candidates(&mut doc, 1).expect("Re-detection failed");
        assert_eq!(
            re_detected.len(),
            0,
            "Expected 0 candidates because fields are already interactive"
        );
    }
}
