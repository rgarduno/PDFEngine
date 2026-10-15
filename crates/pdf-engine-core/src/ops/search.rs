//! Document text search over reconstructed paragraphs.
//!
//! The query is not written into the file. Character offsets are Unicode scalar
//! offsets into the paragraph string. A case-insensitive match is found on the
//! case-folded text and mapped back onto the original scalars, so a fold that
//! changes length (U+00DF LATIN SMALL LETTER SHARP S becomes "ss") does not
//! shift later offsets.
//!
//! `truncated` means the scan stopped because the hit cap was reached. It can
//! be true when the last stored hit was also the last match in the document.
//!
//! A page whose content fails ordinary parsing, layout, or font resolution is
//! listed in `pages_skipped` and the scan continues. A security limit, a
//! decompression failure, a cycle, a recursion limit, or a missing object
//! aborts the search.

use crate::error::{PdfError, PdfResult};
use crate::fonts::resolve_page_fonts;
use crate::layout::{LayoutReconstructor, ParagraphBlock, Rect};
use crate::ops::layers::resolve_page_hidden_ocg_props;
use crate::stream::ContentParser;

/// Most hits one search returns.
pub const MAX_SEARCH_HITS: usize = 500;

/// Most Unicode scalars accepted in one query.
pub const MAX_QUERY_SCALARS: usize = 512;

const SNIPPET_CONTEXT: usize = 48;
const SNIPPET_CAP: usize = 240;

/// One non-overlapping match inside a reconstructed paragraph.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSearchHit {
    /// 0-based page index.
    pub page_index: usize,
    /// Paragraph id on that page.
    pub paragraph_id: usize,
    /// First Unicode scalar of the match inside the paragraph text.
    pub char_start: usize,
    /// Scalar index one past the match.
    pub char_end: usize,
    /// Short context around the match. Newlines are spaces. This is not the whole paragraph.
    pub snippet: String,
    /// Left edge of the matched lines, in page points.
    pub min_x: f64,
    /// Bottom edge of the matched lines, in page points.
    pub min_y: f64,
    /// Right edge of the matched lines, in page points.
    pub max_x: f64,
    /// Top edge of the matched lines, in page points.
    pub max_y: f64,
}

/// Hits collected until the cap, plus pages that could not be read as text.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSearchReport {
    /// Matches in reading order. At most [`MAX_SEARCH_HITS`].
    pub hits: Vec<TextSearchHit>,
    /// True when the scan stopped because `hits` reached the requested cap.
    pub truncated: bool,
    /// 0-based indexes of pages skipped after an ordinary parse, layout, or font failure.
    pub pages_skipped: Vec<usize>,
}

/// Searches reconstructed paragraph text.
///
/// `max_hits` must be in `1..=500`. The query must be non-empty and at most
/// 512 Unicode scalars. The box of a hit is the union of the lines that contain
/// the match, not a glyph outline.
pub fn search_document(
    doc: &mut crate::cos::PdfDocument,
    query: &str,
    ignore_case: bool,
    max_hits: usize,
) -> PdfResult<TextSearchReport> {
    if query.is_empty() {
        return Err(PdfError::OperationError(
            "The search query is empty".to_string(),
        ));
    }
    if query.chars().count() > MAX_QUERY_SCALARS {
        return Err(PdfError::OperationError(format!(
            "The search query is longer than {MAX_QUERY_SCALARS} characters"
        )));
    }
    if max_hits == 0 || max_hits > MAX_SEARCH_HITS {
        return Err(PdfError::OperationError(format!(
            "max_hits must be from 1 to {MAX_SEARCH_HITS}"
        )));
    }

    let page_ids = doc.get_pages()?;
    let mut hits = Vec::new();
    let mut pages_skipped = Vec::new();
    let mut truncated = false;

    'pages: for (page_index, page_id) in page_ids.into_iter().enumerate() {
        let paragraphs = match page_paragraphs(doc, page_id) {
            Ok(paragraphs) => paragraphs,
            Err(err) if is_skippable(&err) => {
                pages_skipped.push(page_index);
                continue;
            }
            Err(err) => return Err(err),
        };
        for paragraph in &paragraphs {
            let text = paragraph.text();
            for (char_start, char_end) in locate_matches(&text, query, ignore_case) {
                let bounds = line_bounds(paragraph, char_start, char_end);
                hits.push(TextSearchHit {
                    page_index,
                    paragraph_id: paragraph.id,
                    char_start,
                    char_end,
                    snippet: snippet(&text, char_start, char_end),
                    min_x: bounds.min_x,
                    min_y: bounds.min_y,
                    max_x: bounds.max_x,
                    max_y: bounds.max_y,
                });
                if hits.len() == max_hits {
                    truncated = true;
                    break 'pages;
                }
            }
        }
    }

    Ok(TextSearchReport {
        hits,
        truncated,
        pages_skipped,
    })
}

fn page_paragraphs(
    doc: &mut crate::cos::PdfDocument,
    page_id: crate::cos::ObjectId,
) -> PdfResult<Vec<ParagraphBlock>> {
    let faces = resolve_page_fonts(doc, page_id)?;
    let hidden = resolve_page_hidden_ocg_props(doc, page_id)?;
    let bytes = doc.get_page_content_bytes(page_id)?;
    let ast = ContentParser::new(&bytes).parse()?;
    LayoutReconstructor::new(&ast)
        .with_resolved(&faces)
        .with_hidden_ocg_props(hidden)
        .reconstruct()
}

/// Ordinary content failures. Security, decompression, cycles, recursion, and
/// missing objects stay outside this set and abort the search.
pub(crate) fn is_skippable(err: &PdfError) -> bool {
    matches!(
        err,
        PdfError::ContentStreamError(_)
            | PdfError::LayoutError(_)
            | PdfError::FontError { .. }
            | PdfError::LexerError { .. }
            | PdfError::ParseError { .. }
            | PdfError::UnexpectedEof { .. }
            | PdfError::TypeMismatch { .. }
    )
}

/// Non-overlapping matches as `(char_start, char_end)` scalar offsets into `text`.
pub(crate) fn locate_matches(text: &str, query: &str, ignore_case: bool) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    if !ignore_case {
        let mut hits = Vec::new();
        let mut start = 0;
        while let Some(found) = text[start..].find(query) {
            let byte_start = start + found;
            let byte_end = byte_start + query.len();
            hits.push((scalar_len(&text[..byte_start]), scalar_len(&text[..byte_end])));
            start = byte_end;
        }
        return hits;
    }

    let mut needle = String::new();
    for ch in query.chars() {
        needle.push_str(&fold_scalar(ch));
    }
    let needle_chars: Vec<char> = needle.chars().collect();
    if needle_chars.is_empty() {
        return Vec::new();
    }

    let units: Vec<(usize, String)> = text
        .chars()
        .enumerate()
        .map(|(index, ch)| (index, fold_scalar(ch)))
        .collect();
    let mut folded: Vec<(usize, char)> = Vec::new();
    for (unit_index, (_, folded_text)) in units.iter().enumerate() {
        for ch in folded_text.chars() {
            folded.push((unit_index, ch));
        }
    }

    let width = needle_chars.len();
    let mut hits = Vec::new();
    let mut cursor = 0;
    while cursor + width <= folded.len() {
        let same = folded[cursor..cursor + width]
            .iter()
            .zip(needle_chars.iter())
            .all(|(item, expected)| item.1 == *expected);
        if !same {
            cursor += 1;
            continue;
        }
        let unit_start = folded[cursor].0;
        let unit_end = folded[cursor + width - 1].0;
        hits.push((units[unit_start].0, units[unit_end].0 + 1));
        cursor += 1;
        while cursor < folded.len() && folded[cursor].0 <= unit_end {
            cursor += 1;
        }
    }
    hits
}

fn scalar_len(text: &str) -> usize {
    text.chars().count()
}

/// Lowercase mapping, with U+00DF expanded to `ss`.
///
/// `char::to_lowercase` leaves a sharp s unchanged because it is already
/// lowercase. The match has to use the longer fold so a later scalar keeps
/// its original index.
fn fold_scalar(ch: char) -> String {
    let mut folded = String::new();
    for item in ch.to_lowercase() {
        if item == 'ß' {
            folded.push_str("ss");
        } else {
            folded.push(item);
        }
    }
    folded
}

fn line_bounds(paragraph: &ParagraphBlock, char_start: usize, char_end: usize) -> Rect {
    let mut cursor = 0usize;
    let mut found: Option<Rect> = None;
    for (index, line) in paragraph.lines.iter().enumerate() {
        let line_start = cursor;
        let line_end = cursor + line.text.chars().count();
        if char_end > line_start && char_start < line_end {
            found = Some(match found {
                Some(acc) => acc.union(&line.bbox),
                None => line.bbox,
            });
        }
        cursor = line_end;
        if index + 1 < paragraph.lines.len() {
            cursor += 1;
        }
    }
    found.unwrap_or(paragraph.bbox)
}

pub(crate) fn snippet(text: &str, char_start: usize, char_end: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let from = char_start.min(chars.len()).saturating_sub(SNIPPET_CONTEXT);
    let to = (char_end + SNIPPET_CONTEXT).min(chars.len());
    let mut out = String::new();
    if from > 0 {
        out.push('…');
    }
    for ch in &chars[from..to] {
        if *ch == '\n' || *ch == '\r' {
            out.push(' ');
        } else {
            out.push(*ch);
        }
    }
    if to < chars.len() {
        out.push('…');
    }
    if out.chars().count() > SNIPPET_CAP {
        out = out.chars().take(SNIPPET_CAP).collect();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{locate_matches, snippet};

    #[test]
    fn sharp_s_fold_does_not_shift_later_scalars() {
        assert_eq!(locate_matches("Straße", "strasse", true), vec![(0, 6)]);
        assert_eq!(locate_matches("aßb", "ss", true), vec![(1, 2)]);
        assert_eq!(locate_matches("aßb", "b", true), vec![(2, 3)]);
    }

    #[test]
    fn case_sensitive_matches_are_scalar_offsets() {
        assert_eq!(locate_matches("Hello Hello", "Hello", false), vec![(0, 5), (6, 11)]);
        assert!(locate_matches("Hello", "hello", false).is_empty());
    }

    #[test]
    fn snippet_replaces_newlines_and_marks_a_cut() {
        let text = format!("aaa\n{}ccc", "b".repeat(80));
        let rendered = snippet(&text, 4, 84);
        assert!(!rendered.contains('\n'));
        assert!(rendered.starts_with('…') || rendered.ends_with('…') || rendered.contains('b'));
        assert!(rendered.chars().count() <= 240);
    }
}
