//! PDF Document Comparison and Semantic Diff Engine (ISO 32000-1).
//!
//! Provides document-level and page-level comparison between two revisions of a PDF document:
//! 1. Metadata synchronization and discrepancy tracking (Title, Author, Subject, etc.).
//! 2. Page count and geometric MediaBox boundary analysis.
//! 3. Semantic text paragraph comparison with bipartite spatial-textual matching.
//! 4. Token-level LCS (Longest Common Subsequence) difference highlighting (Added, Deleted, Unchanged).
//! 5. Image XObject addition, deletion, and property discrepancy tracking.

use crate::cos::object::ObjectId;
use crate::cos::PdfDocument;
use crate::error::PdfResult;
use crate::fonts::resolve_page_fonts;
use crate::images::{extract_page_images, ImageInfo};
use crate::layout::geometry::Rect;
use crate::layout::paragraph::ParagraphBlock;
use crate::layout::reconstructor::LayoutReconstructor;
use crate::ops::metadata::{extract_metadata, DocumentMetadata};
use crate::stream::parser::{build_ast_from_operations, ContentStreamTokenizer};
use crate::watermark::helpers::get_page_mediabox;

/// Classification of a difference item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    /// Item is unchanged between base and target.
    Unchanged,
    /// Item exists in target but not in base (addition).
    Added,
    /// Item exists in base but not in target (deletion).
    Deleted,
    /// Item exists in both with content or geometry changes.
    Modified,
}

impl DiffKind {
    /// Returns the lowercase string identifier.
    pub fn as_str(&self) -> &'static str {
        match self {
            DiffKind::Unchanged => "unchanged",
            DiffKind::Added => "added",
            DiffKind::Deleted => "deleted",
            DiffKind::Modified => "modified",
        }
    }
}

/// A word-level token difference within a modified text block.
#[derive(Debug, Clone, PartialEq)]
pub struct WordDiff {
    pub kind: DiffKind,
    pub text: String,
}

/// A textual difference item comparing a paragraph or line.
#[derive(Debug, Clone, PartialEq)]
pub struct TextDiffItem {
    pub kind: DiffKind,
    pub base_text: Option<String>,
    pub target_text: Option<String>,
    pub base_bbox: Option<[f64; 4]>,
    pub target_bbox: Option<[f64; 4]>,
    pub word_diffs: Vec<WordDiff>,
}

/// An image discrepancy item.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageDiffItem {
    pub kind: DiffKind,
    pub resource_name: String,
    pub base_bbox: Option<[f64; 4]>,
    pub target_bbox: Option<[f64; 4]>,
    pub base_dimensions: Option<[u32; 2]>,
    pub target_dimensions: Option<[u32; 2]>,
}

/// Page dimensions in PDF points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageDimensions {
    pub width: f64,
    pub height: f64,
}

/// Comparison result for a specific page.
#[derive(Debug, Clone, PartialEq)]
pub struct PageDiff {
    pub page_number_base: Option<usize>,
    pub page_number_target: Option<usize>,
    pub kind: DiffKind,
    pub base_dimensions: Option<PageDimensions>,
    pub target_dimensions: Option<PageDimensions>,
    pub dimensions_changed: bool,
    pub text_diffs: Vec<TextDiffItem>,
    pub image_diffs: Vec<ImageDiffItem>,
}

/// Discrepancy between document metadata fields.
#[derive(Debug, Clone, PartialEq)]
pub struct MetadataDiffItem {
    pub field: String,
    pub base_value: Option<String>,
    pub target_value: Option<String>,
}

/// High-level summary of all detected discrepancies.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DiffSummary {
    pub base_page_count: usize,
    pub target_page_count: usize,
    pub total_pages_with_changes: usize,
    pub text_additions: usize,
    pub text_deletions: usize,
    pub text_modifications: usize,
    pub image_additions: usize,
    pub image_deletions: usize,
    pub image_modifications: usize,
    pub metadata_changes: usize,
}

/// Full comparison report between two PDF documents.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffReport {
    pub is_identical: bool,
    pub summary: DiffSummary,
    pub metadata_diffs: Vec<MetadataDiffItem>,
    pub pages: Vec<PageDiff>,
}

/// Options to tune comparison sensitivity.
#[derive(Debug, Clone)]
pub struct DiffOptions {
    /// Ignore leading/trailing and redundant whitespace.
    pub ignore_whitespace: bool,
    /// Compare text case-insensitively.
    pub ignore_case: bool,
    /// Similarity threshold (0.0 to 1.0) to consider two paragraphs a modification rather than delete+add.
    pub similarity_threshold: f64,
    /// Compare Image XObjects.
    pub compare_images: bool,
    /// Compare document metadata.
    pub compare_metadata: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            ignore_whitespace: true,
            ignore_case: false,
            similarity_threshold: 0.35,
            compare_images: true,
            compare_metadata: true,
        }
    }
}

/// Escapes a string for JSON output.
fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0C' => out.push_str("\\f"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

impl DiffReport {
    /// Serializes the comparison report to a structured JSON string.
    pub fn to_json(&self) -> String {
        let mut json = String::new();
        json.push('{');

        json.push_str(&format!("\"is_identical\":{},", self.is_identical));

        // Summary
        json.push_str("\"summary\":{");
        json.push_str(&format!(
            "\"base_page_count\":{},",
            self.summary.base_page_count
        ));
        json.push_str(&format!(
            "\"target_page_count\":{},",
            self.summary.target_page_count
        ));
        json.push_str(&format!(
            "\"total_pages_with_changes\":{},",
            self.summary.total_pages_with_changes
        ));
        json.push_str(&format!(
            "\"text_additions\":{},",
            self.summary.text_additions
        ));
        json.push_str(&format!(
            "\"text_deletions\":{},",
            self.summary.text_deletions
        ));
        json.push_str(&format!(
            "\"text_modifications\":{},",
            self.summary.text_modifications
        ));
        json.push_str(&format!(
            "\"image_additions\":{},",
            self.summary.image_additions
        ));
        json.push_str(&format!(
            "\"image_deletions\":{},",
            self.summary.image_deletions
        ));
        json.push_str(&format!(
            "\"image_modifications\":{},",
            self.summary.image_modifications
        ));
        json.push_str(&format!(
            "\"metadata_changes\":{}",
            self.summary.metadata_changes
        ));
        json.push_str("},");

        // Metadata diffs
        json.push_str("\"metadata_diffs\":[");
        for (idx, m) in self.metadata_diffs.iter().enumerate() {
            if idx > 0 {
                json.push(',');
            }
            json.push('{');
            json.push_str(&format!("\"field\":\"{}\",", escape_json(&m.field)));
            if let Some(ref bv) = m.base_value {
                json.push_str(&format!("\"base_value\":\"{}\",", escape_json(bv)));
            } else {
                json.push_str("\"base_value\":null,");
            }
            if let Some(ref tv) = m.target_value {
                json.push_str(&format!("\"target_value\":\"{}\"", escape_json(tv)));
            } else {
                json.push_str("\"target_value\":null");
            }
            json.push('}');
        }
        json.push_str("],");

        // Pages
        json.push_str("\"pages\":[");
        for (p_idx, page) in self.pages.iter().enumerate() {
            if p_idx > 0 {
                json.push(',');
            }
            json.push('{');
            if let Some(pnb) = page.page_number_base {
                json.push_str(&format!("\"page_number_base\":{},", pnb));
            } else {
                json.push_str("\"page_number_base\":null,");
            }
            if let Some(pnt) = page.page_number_target {
                json.push_str(&format!("\"page_number_target\":{},", pnt));
            } else {
                json.push_str("\"page_number_target\":null,");
            }
            json.push_str(&format!("\"kind\":\"{}\",", page.kind.as_str()));
            json.push_str(&format!(
                "\"dimensions_changed\":{},",
                page.dimensions_changed
            ));

            if let Some(ref bd) = page.base_dimensions {
                json.push_str(&format!(
                    "\"base_dimensions\":{{\"width\":{:.2},\"height\":{:.2}}},",
                    bd.width, bd.height
                ));
            } else {
                json.push_str("\"base_dimensions\":null,");
            }

            if let Some(ref td) = page.target_dimensions {
                json.push_str(&format!(
                    "\"target_dimensions\":{{\"width\":{:.2},\"height\":{:.2}}},",
                    td.width, td.height
                ));
            } else {
                json.push_str("\"target_dimensions\":null,");
            }

            // Text diffs
            json.push_str("\"text_diffs\":[");
            for (t_idx, td) in page.text_diffs.iter().enumerate() {
                if t_idx > 0 {
                    json.push(',');
                }
                json.push('{');
                json.push_str(&format!("\"kind\":\"{}\",", td.kind.as_str()));
                if let Some(ref bt) = td.base_text {
                    json.push_str(&format!("\"base_text\":\"{}\",", escape_json(bt)));
                } else {
                    json.push_str("\"base_text\":null,");
                }
                if let Some(ref tt) = td.target_text {
                    json.push_str(&format!("\"target_text\":\"{}\",", escape_json(tt)));
                } else {
                    json.push_str("\"target_text\":null,");
                }
                if let Some(bb) = td.base_bbox {
                    json.push_str(&format!(
                        "\"base_bbox\":[{:.2},{:.2},{:.2},{:.2}],",
                        bb[0], bb[1], bb[2], bb[3]
                    ));
                } else {
                    json.push_str("\"base_bbox\":null,");
                }
                if let Some(tb) = td.target_bbox {
                    json.push_str(&format!(
                        "\"target_bbox\":[{:.2},{:.2},{:.2},{:.2}],",
                        tb[0], tb[1], tb[2], tb[3]
                    ));
                } else {
                    json.push_str("\"target_bbox\":null,");
                }

                // Word diffs
                json.push_str("\"word_diffs\":[");
                for (w_idx, wd) in td.word_diffs.iter().enumerate() {
                    if w_idx > 0 {
                        json.push(',');
                    }
                    json.push('{');
                    json.push_str(&format!(
                        "\"kind\":\"{}\",\"text\":\"{}\"",
                        wd.kind.as_str(),
                        escape_json(&wd.text)
                    ));
                    json.push('}');
                }
                json.push(']');

                json.push('}');
            }
            json.push_str("],");

            // Image diffs
            json.push_str("\"image_diffs\":[");
            for (i_idx, id) in page.image_diffs.iter().enumerate() {
                if i_idx > 0 {
                    json.push(',');
                }
                json.push('{');
                json.push_str(&format!("\"kind\":\"{}\",", id.kind.as_str()));
                json.push_str(&format!(
                    "\"resource_name\":\"{}\",",
                    escape_json(&id.resource_name)
                ));
                if let Some(bb) = id.base_bbox {
                    json.push_str(&format!(
                        "\"base_bbox\":[{:.2},{:.2},{:.2},{:.2}],",
                        bb[0], bb[1], bb[2], bb[3]
                    ));
                } else {
                    json.push_str("\"base_bbox\":null,");
                }
                if let Some(tb) = id.target_bbox {
                    json.push_str(&format!(
                        "\"target_bbox\":[{:.2},{:.2},{:.2},{:.2}],",
                        tb[0], tb[1], tb[2], tb[3]
                    ));
                } else {
                    json.push_str("\"target_bbox\":null,");
                }
                if let Some(bd) = id.base_dimensions {
                    json.push_str(&format!("\"base_dimensions\":[{},{}],", bd[0], bd[1]));
                } else {
                    json.push_str("\"base_dimensions\":null,");
                }
                if let Some(td) = id.target_dimensions {
                    json.push_str(&format!("\"target_dimensions\":[{},{}]", td[0], td[1]));
                } else {
                    json.push_str("\"target_dimensions\":null");
                }
                json.push('}');
            }
            json.push(']');

            json.push('}');
        }
        json.push(']');

        json.push('}');
        json
    }
}

/// Tokenizes text into words for difference analysis.
fn tokenize_words(text: &str) -> Vec<String> {
    text.split_whitespace().map(|s| s.to_string()).collect()
}

/// Computes word-level Longest Common Subsequence (LCS) edit script between two word sequences.
///
/// Returns the list of `WordDiff` elements and the token similarity ratio in `[0.0, 1.0]`.
pub fn compute_word_diffs(
    words_a: &[String],
    words_b: &[String],
    ignore_case: bool,
) -> (Vec<WordDiff>, f64) {
    let n = words_a.len();
    let m = words_b.len();

    if n == 0 && m == 0 {
        return (Vec::new(), 1.0);
    }
    if n == 0 {
        let diffs = words_b
            .iter()
            .map(|w| WordDiff {
                kind: DiffKind::Added,
                text: w.clone(),
            })
            .collect();
        return (diffs, 0.0);
    }
    if m == 0 {
        let diffs = words_a
            .iter()
            .map(|w| WordDiff {
                kind: DiffKind::Deleted,
                text: w.clone(),
            })
            .collect();
        return (diffs, 0.0);
    }

    // Dynamic programming matrix with safe bounds (cap at 2000 tokens for DoS protection)
    let max_len = 2000;
    let n_clamped = n.min(max_len);
    let m_clamped = m.min(max_len);

    let mut dp = vec![vec![0usize; m_clamped + 1]; n_clamped + 1];

    for i in 0..n_clamped {
        for j in 0..m_clamped {
            let eq = if ignore_case {
                words_a[i].eq_ignore_ascii_case(&words_b[j])
            } else {
                words_a[i] == words_b[j]
            };
            if eq {
                dp[i + 1][j + 1] = dp[i][j] + 1;
            } else {
                dp[i + 1][j + 1] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }

    let lcs_len = dp[n_clamped][m_clamped];
    let similarity = (2.0 * lcs_len as f64) / (n + m) as f64;

    // Backtrack to extract edit operations
    let mut i = n_clamped;
    let mut j = m_clamped;
    let mut diffs = Vec::new();

    while i > 0 || j > 0 {
        let eq = if i > 0 && j > 0 {
            if ignore_case {
                words_a[i - 1].eq_ignore_ascii_case(&words_b[j - 1])
            } else {
                words_a[i - 1] == words_b[j - 1]
            }
        } else {
            false
        };

        if i > 0 && j > 0 && eq {
            diffs.push(WordDiff {
                kind: DiffKind::Unchanged,
                text: words_a[i - 1].clone(),
            });
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            diffs.push(WordDiff {
                kind: DiffKind::Added,
                text: words_b[j - 1].clone(),
            });
            j -= 1;
        } else if i > 0 {
            diffs.push(WordDiff {
                kind: DiffKind::Deleted,
                text: words_a[i - 1].clone(),
            });
            i -= 1;
        }
    }

    diffs.reverse();
    (diffs, similarity)
}

/// Measures the spatial intersection over union (IoU) of two bounding boxes.
fn compute_box_iou(a: &Rect, b: &Rect) -> f64 {
    let inter_min_x = a.min_x.max(b.min_x);
    let inter_min_y = a.min_y.max(b.min_y);
    let inter_max_x = a.max_x.min(b.max_x);
    let inter_max_y = a.max_y.min(b.max_y);

    if inter_max_x <= inter_min_x || inter_max_y <= inter_min_y {
        return 0.0;
    }

    let inter_area = (inter_max_x - inter_min_x) * (inter_max_y - inter_min_y);
    let a_area = a.width() * a.height();
    let b_area = b.width() * b.height();
    let union_area = a_area + b_area - inter_area;

    if union_area <= 0.0 {
        0.0
    } else {
        inter_area / union_area
    }
}

/// Computes the centroid distance between two bounding boxes normalized by page scale.
fn compute_centroid_distance(a: &Rect, b: &Rect) -> f64 {
    let ax = (a.min_x + a.max_x) * 0.5;
    let ay = (a.min_y + a.max_y) * 0.5;
    let bx = (b.min_x + b.max_x) * 0.5;
    let by = (b.min_y + b.max_y) * 0.5;
    ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt()
}

/// Extracts paragraphs on a page by decoding the content stream and evaluating layout.
fn extract_page_paragraphs(
    doc: &mut PdfDocument,
    page_id: ObjectId,
) -> PdfResult<Vec<ParagraphBlock>> {
    let faces = resolve_page_fonts(doc, page_id)?;
    let content_bytes = doc.get_page_content_bytes(page_id)?;
    if content_bytes.is_empty() {
        return Ok(Vec::new());
    }

    let mut tokenizer = ContentStreamTokenizer::new(&content_bytes);
    let operations = tokenizer.tokenize_all().unwrap_or_default();
    let ast = build_ast_from_operations(operations);

    let reconstructor = LayoutReconstructor::new(&ast).with_resolved(&faces);
    reconstructor.reconstruct()
}

/// Compares paragraphs between a base page and target page using a greedy bipartite matching algorithm.
fn diff_page_paragraphs(
    paragraphs_base: &[ParagraphBlock],
    paragraphs_target: &[ParagraphBlock],
    options: &DiffOptions,
) -> Vec<TextDiffItem> {
    let mut items = Vec::new();

    let mut matched_base = vec![false; paragraphs_base.len()];
    let mut matched_target = vec![false; paragraphs_target.len()];

    // Pass 1: Look for exact text matches (spatial positioning can vary slightly)
    for (i, p_base) in paragraphs_base.iter().enumerate() {
        let text_a = p_base.text();
        for (j, p_target) in paragraphs_target.iter().enumerate() {
            if matched_target[j] {
                continue;
            }
            let text_b = p_target.text();

            let is_match = if options.ignore_case {
                text_a.eq_ignore_ascii_case(&text_b)
            } else {
                text_a == text_b
            };

            if is_match {
                let dist = compute_centroid_distance(&p_base.bbox, &p_target.bbox);
                // Allow up to 100 points of vertical or horizontal displacement for exact text
                if dist < 120.0 {
                    matched_base[i] = true;
                    matched_target[j] = true;
                    break;
                }
            }
        }
    }

    // Pass 2: Bipartite similarity scoring for modified paragraphs
    struct Candidate {
        base_idx: usize,
        target_idx: usize,
        score: f64,
        word_diffs: Vec<WordDiff>,
    }

    let mut candidates = Vec::new();

    for (i, p_base) in paragraphs_base.iter().enumerate() {
        if matched_base[i] {
            continue;
        }
        let words_a = tokenize_words(&p_base.text());
        if words_a.is_empty() {
            continue;
        }

        for (j, p_target) in paragraphs_target.iter().enumerate() {
            if matched_target[j] {
                continue;
            }
            let words_b = tokenize_words(&p_target.text());
            if words_b.is_empty() {
                continue;
            }

            let (word_diffs, text_sim) =
                compute_word_diffs(&words_a, &words_b, options.ignore_case);
            let iou = compute_box_iou(&p_base.bbox, &p_target.bbox);
            let dist = compute_centroid_distance(&p_base.bbox, &p_target.bbox);

            let spatial_sim = if iou > 0.1 {
                iou
            } else {
                (1.0 - (dist / 400.0)).max(0.0)
            };

            let combined_score = 0.65 * text_sim + 0.35 * spatial_sim;

            if combined_score >= options.similarity_threshold || text_sim >= 0.5 {
                candidates.push(Candidate {
                    base_idx: i,
                    target_idx: j,
                    score: combined_score,
                    word_diffs,
                });
            }
        }
    }

    // Sort candidates descending by score
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for cand in candidates {
        if matched_base[cand.base_idx] || matched_target[cand.target_idx] {
            continue;
        }
        matched_base[cand.base_idx] = true;
        matched_target[cand.target_idx] = true;

        let p_base = &paragraphs_base[cand.base_idx];
        let p_target = &paragraphs_target[cand.target_idx];

        items.push(TextDiffItem {
            kind: DiffKind::Modified,
            base_text: Some(p_base.text()),
            target_text: Some(p_target.text()),
            base_bbox: Some([
                p_base.bbox.min_x,
                p_base.bbox.min_y,
                p_base.bbox.max_x,
                p_base.bbox.max_y,
            ]),
            target_bbox: Some([
                p_target.bbox.min_x,
                p_target.bbox.min_y,
                p_target.bbox.max_x,
                p_target.bbox.max_y,
            ]),
            word_diffs: cand.word_diffs,
        });
    }

    // Pass 3: Unmatched base paragraphs are Deletions
    for (i, p_base) in paragraphs_base.iter().enumerate() {
        if !matched_base[i] {
            let text = p_base.text();
            let words = tokenize_words(&text);
            let word_diffs = words
                .into_iter()
                .map(|w| WordDiff {
                    kind: DiffKind::Deleted,
                    text: w,
                })
                .collect();

            items.push(TextDiffItem {
                kind: DiffKind::Deleted,
                base_text: Some(text),
                target_text: None,
                base_bbox: Some([
                    p_base.bbox.min_x,
                    p_base.bbox.min_y,
                    p_base.bbox.max_x,
                    p_base.bbox.max_y,
                ]),
                target_bbox: None,
                word_diffs,
            });
        }
    }

    // Pass 4: Unmatched target paragraphs are Additions
    for (j, p_target) in paragraphs_target.iter().enumerate() {
        if !matched_target[j] {
            let text = p_target.text();
            let words = tokenize_words(&text);
            let word_diffs = words
                .into_iter()
                .map(|w| WordDiff {
                    kind: DiffKind::Added,
                    text: w,
                })
                .collect();

            items.push(TextDiffItem {
                kind: DiffKind::Added,
                base_text: None,
                target_text: Some(text),
                base_bbox: None,
                target_bbox: Some([
                    p_target.bbox.min_x,
                    p_target.bbox.min_y,
                    p_target.bbox.max_x,
                    p_target.bbox.max_y,
                ]),
                word_diffs,
            });
        }
    }

    items
}

/// Compares Image XObjects on a page between base and target.
fn diff_page_images(images_base: &[ImageInfo], images_target: &[ImageInfo]) -> Vec<ImageDiffItem> {
    let mut items = Vec::new();
    let mut matched_target = vec![false; images_target.len()];

    for img_base in images_base {
        let mut found = false;
        for (j, img_target) in images_target.iter().enumerate() {
            if matched_target[j] {
                continue;
            }
            // Match by resource name or high spatial overlap
            let name_match = img_base.resource_name == img_target.resource_name;
            let iou = compute_box_iou(&img_base.bbox, &img_target.bbox);

            if name_match || iou > 0.5 {
                matched_target[j] = true;
                found = true;

                let dims_match = img_base.width_px == img_target.width_px
                    && img_base.height_px == img_target.height_px;
                let size_match = img_base.byte_size == img_target.byte_size;

                if !dims_match || !size_match {
                    items.push(ImageDiffItem {
                        kind: DiffKind::Modified,
                        resource_name: img_base.resource_name.clone(),
                        base_bbox: Some([
                            img_base.bbox.min_x,
                            img_base.bbox.min_y,
                            img_base.bbox.max_x,
                            img_base.bbox.max_y,
                        ]),
                        target_bbox: Some([
                            img_target.bbox.min_x,
                            img_target.bbox.min_y,
                            img_target.bbox.max_x,
                            img_target.bbox.max_y,
                        ]),
                        base_dimensions: Some([img_base.width_px, img_base.height_px]),
                        target_dimensions: Some([img_target.width_px, img_target.height_px]),
                    });
                }
                break;
            }
        }

        if !found {
            items.push(ImageDiffItem {
                kind: DiffKind::Deleted,
                resource_name: img_base.resource_name.clone(),
                base_bbox: Some([
                    img_base.bbox.min_x,
                    img_base.bbox.min_y,
                    img_base.bbox.max_x,
                    img_base.bbox.max_y,
                ]),
                target_bbox: None,
                base_dimensions: Some([img_base.width_px, img_base.height_px]),
                target_dimensions: None,
            });
        }
    }

    for (j, img_target) in images_target.iter().enumerate() {
        if !matched_target[j] {
            items.push(ImageDiffItem {
                kind: DiffKind::Added,
                resource_name: img_target.resource_name.clone(),
                base_bbox: None,
                target_bbox: Some([
                    img_target.bbox.min_x,
                    img_target.bbox.min_y,
                    img_target.bbox.max_x,
                    img_target.bbox.max_y,
                ]),
                base_dimensions: None,
                target_dimensions: Some([img_target.width_px, img_target.height_px]),
            });
        }
    }

    items
}

/// Compares two metadata structures and produces a list of discrepancies.
fn diff_metadata(meta_a: &DocumentMetadata, meta_b: &DocumentMetadata) -> Vec<MetadataDiffItem> {
    let mut diffs = Vec::new();

    let fields = [
        ("title", &meta_a.title, &meta_b.title),
        ("author", &meta_a.author, &meta_b.author),
        ("subject", &meta_a.subject, &meta_b.subject),
        ("keywords", &meta_a.keywords, &meta_b.keywords),
        ("creator", &meta_a.creator, &meta_b.creator),
        ("producer", &meta_a.producer, &meta_b.producer),
    ];

    for (name, a, b) in fields {
        let val_a = a.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty());
        let val_b = b.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty());

        if val_a != val_b {
            diffs.push(MetadataDiffItem {
                field: name.to_string(),
                base_value: a.clone(),
                target_value: b.clone(),
            });
        }
    }

    diffs
}

/// Compares two PDF documents and generates a complete semantic DiffReport.
///
/// # Arguments
/// * `doc_base`: The original reference document.
/// * `doc_target`: The modified document to compare against base.
/// * `options`: Sensitivity and scope flags.
pub fn compare_documents(
    doc_base: &mut PdfDocument,
    doc_target: &mut PdfDocument,
    options: &DiffOptions,
) -> PdfResult<DiffReport> {
    let base_pages = doc_base.get_pages()?;
    let target_pages = doc_target.get_pages()?;

    let base_count = base_pages.len();
    let target_count = target_pages.len();
    let max_pages = base_count.max(target_count);

    let mut metadata_diffs = Vec::new();
    if options.compare_metadata {
        let meta_base = extract_metadata(doc_base)?;
        let meta_target = extract_metadata(doc_target)?;
        metadata_diffs = diff_metadata(&meta_base, &meta_target);
    }

    let mut pages_diff = Vec::new();
    let mut summary = DiffSummary {
        base_page_count: base_count,
        target_page_count: target_count,
        metadata_changes: metadata_diffs.len(),
        ..Default::default()
    };

    for idx in 0..max_pages {
        let base_page_id = base_pages.get(idx).copied();
        let target_page_id = target_pages.get(idx).copied();

        match (base_page_id, target_page_id) {
            (Some(b_id), Some(t_id)) => {
                let base_mb = get_page_mediabox(doc_base, b_id);
                let target_mb = get_page_mediabox(doc_target, t_id);

                let base_dims = PageDimensions {
                    width: base_mb.width(),
                    height: base_mb.height(),
                };
                let target_dims = PageDimensions {
                    width: target_mb.width(),
                    height: target_mb.height(),
                };

                let dims_changed = (base_dims.width - target_dims.width).abs() > 0.5
                    || (base_dims.height - target_dims.height).abs() > 0.5;

                let paras_base = extract_page_paragraphs(doc_base, b_id)?;
                let paras_target = extract_page_paragraphs(doc_target, t_id)?;
                let text_diffs = diff_page_paragraphs(&paras_base, &paras_target, options);

                let image_diffs = if options.compare_images {
                    let imgs_base = extract_page_images(doc_base, b_id)?;
                    let imgs_target = extract_page_images(doc_target, t_id)?;
                    diff_page_images(&imgs_base, &imgs_target)
                } else {
                    Vec::new()
                };

                let has_changes = dims_changed || !text_diffs.is_empty() || !image_diffs.is_empty();

                for td in &text_diffs {
                    match td.kind {
                        DiffKind::Added => summary.text_additions += 1,
                        DiffKind::Deleted => summary.text_deletions += 1,
                        DiffKind::Modified => summary.text_modifications += 1,
                        DiffKind::Unchanged => {}
                    }
                }

                for id in &image_diffs {
                    match id.kind {
                        DiffKind::Added => summary.image_additions += 1,
                        DiffKind::Deleted => summary.image_deletions += 1,
                        DiffKind::Modified => summary.image_modifications += 1,
                        DiffKind::Unchanged => {}
                    }
                }

                if has_changes {
                    summary.total_pages_with_changes += 1;
                }

                let page_kind = if has_changes {
                    DiffKind::Modified
                } else {
                    DiffKind::Unchanged
                };

                pages_diff.push(PageDiff {
                    page_number_base: Some(idx + 1),
                    page_number_target: Some(idx + 1),
                    kind: page_kind,
                    base_dimensions: Some(base_dims),
                    target_dimensions: Some(target_dims),
                    dimensions_changed: dims_changed,
                    text_diffs,
                    image_diffs,
                });
            }
            (Some(b_id), None) => {
                // Page deleted in target
                let base_mb = get_page_mediabox(doc_base, b_id);
                let base_dims = PageDimensions {
                    width: base_mb.width(),
                    height: base_mb.height(),
                };

                let paras_base = extract_page_paragraphs(doc_base, b_id)?;
                let mut text_diffs = Vec::new();
                for p in paras_base {
                    let text = p.text();
                    let words = tokenize_words(&text);
                    text_diffs.push(TextDiffItem {
                        kind: DiffKind::Deleted,
                        base_text: Some(text),
                        target_text: None,
                        base_bbox: Some([p.bbox.min_x, p.bbox.min_y, p.bbox.max_x, p.bbox.max_y]),
                        target_bbox: None,
                        word_diffs: words
                            .into_iter()
                            .map(|w| WordDiff {
                                kind: DiffKind::Deleted,
                                text: w,
                            })
                            .collect(),
                    });
                    summary.text_deletions += 1;
                }

                let mut image_diffs = Vec::new();
                if options.compare_images {
                    let imgs_base = extract_page_images(doc_base, b_id)?;
                    for img in imgs_base {
                        image_diffs.push(ImageDiffItem {
                            kind: DiffKind::Deleted,
                            resource_name: img.resource_name,
                            base_bbox: Some([
                                img.bbox.min_x,
                                img.bbox.min_y,
                                img.bbox.max_x,
                                img.bbox.max_y,
                            ]),
                            target_bbox: None,
                            base_dimensions: Some([img.width_px, img.height_px]),
                            target_dimensions: None,
                        });
                        summary.image_deletions += 1;
                    }
                }

                summary.total_pages_with_changes += 1;

                pages_diff.push(PageDiff {
                    page_number_base: Some(idx + 1),
                    page_number_target: None,
                    kind: DiffKind::Deleted,
                    base_dimensions: Some(base_dims),
                    target_dimensions: None,
                    dimensions_changed: false,
                    text_diffs,
                    image_diffs,
                });
            }
            (None, Some(t_id)) => {
                // Page added in target
                let target_mb = get_page_mediabox(doc_target, t_id);
                let target_dims = PageDimensions {
                    width: target_mb.width(),
                    height: target_mb.height(),
                };

                let paras_target = extract_page_paragraphs(doc_target, t_id)?;
                let mut text_diffs = Vec::new();
                for p in paras_target {
                    let text = p.text();
                    let words = tokenize_words(&text);
                    text_diffs.push(TextDiffItem {
                        kind: DiffKind::Added,
                        base_text: None,
                        target_text: Some(text),
                        base_bbox: None,
                        target_bbox: Some([p.bbox.min_x, p.bbox.min_y, p.bbox.max_x, p.bbox.max_y]),
                        word_diffs: words
                            .into_iter()
                            .map(|w| WordDiff {
                                kind: DiffKind::Added,
                                text: w,
                            })
                            .collect(),
                    });
                    summary.text_additions += 1;
                }

                let mut image_diffs = Vec::new();
                if options.compare_images {
                    let imgs_target = extract_page_images(doc_target, t_id)?;
                    for img in imgs_target {
                        image_diffs.push(ImageDiffItem {
                            kind: DiffKind::Added,
                            resource_name: img.resource_name,
                            base_bbox: None,
                            target_bbox: Some([
                                img.bbox.min_x,
                                img.bbox.min_y,
                                img.bbox.max_x,
                                img.bbox.max_y,
                            ]),
                            base_dimensions: None,
                            target_dimensions: Some([img.width_px, img.height_px]),
                        });
                        summary.image_additions += 1;
                    }
                }

                summary.total_pages_with_changes += 1;

                pages_diff.push(PageDiff {
                    page_number_base: None,
                    page_number_target: Some(idx + 1),
                    kind: DiffKind::Added,
                    base_dimensions: None,
                    target_dimensions: Some(target_dims),
                    dimensions_changed: false,
                    text_diffs,
                    image_diffs,
                });
            }
            (None, None) => break,
        }
    }

    let is_identical = summary.text_additions == 0
        && summary.text_deletions == 0
        && summary.text_modifications == 0
        && summary.image_additions == 0
        && summary.image_deletions == 0
        && summary.image_modifications == 0
        && summary.metadata_changes == 0
        && summary.base_page_count == summary.target_page_count
        && !pages_diff.iter().any(|p| p.dimensions_changed);

    Ok(DiffReport {
        is_identical,
        summary,
        metadata_diffs,
        pages: pages_diff,
    })
}
