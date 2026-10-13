//! Page composition and ISO 32000-1 page-boundary boxes.
//!
//! Blank pages, duplicates, and pages copied from another document are appended
//! and then moved with [`reorder_pages`]. An insert index equal to the current
//! page count appends. The first page is index 0.
//!
//! [`import_page`](crate::ops::ObjectCloner::import_page) always appends, and one
//! cloner reuses a source object it has already copied. A repeated source index
//! in a single insert is rejected so two destination pages do not share one
//! content stream.

use std::collections::HashSet;

use crate::cos::{ObjectId, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfStream};
use crate::error::{PdfError, PdfResult};
use crate::layout::Rect;
use crate::ops::cloner::ObjectCloner;
use crate::ops::reorder::reorder_pages;
use crate::ops::split::extract_pages;

/// Largest page coordinate Acrobat accepts, in default user-space points.
const MAX_USER_UNIT: f64 = 14400.0;

/// Pages copied by one [`insert_pages`] call.
const MAX_INSERT_PAGES: usize = 200;

/// `/Parent` links followed while a page box is inherited (ISO 32000-1 Table 30).
const MAX_PARENT_HOPS: usize = 16;

/// One of the five page boundaries in ISO 32000-1 Table 30.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageBoundary {
    /// The physical medium. This entry cannot be removed.
    Media,
    /// The region displayed or printed. Viewers clip to this box when it is present.
    Crop,
    /// The region including bleed marks.
    Bleed,
    /// The finished page size after trimming.
    Trim,
    /// The meaningful content region inside the trim box.
    Art,
}

impl PageBoundary {
    /// Dictionary key written on the page.
    pub fn key(self) -> &'static str {
        match self {
            Self::Media => "MediaBox",
            Self::Crop => "CropBox",
            Self::Bleed => "BleedBox",
            Self::Trim => "TrimBox",
            Self::Art => "ArtBox",
        }
    }
}

/// Accepts `media` or `MediaBox`, and the same pair for crop, bleed, trim, and art.
pub fn parse_page_boundary(raw: &str) -> Option<PageBoundary> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "media" | "mediabox" => Some(PageBoundary::Media),
        "crop" | "cropbox" => Some(PageBoundary::Crop),
        "bleed" | "bleedbox" => Some(PageBoundary::Bleed),
        "trim" | "trimbox" => Some(PageBoundary::Trim),
        "art" | "artbox" => Some(PageBoundary::Art),
        _ => None,
    }
}

/// Resolved page boundaries. Missing optional boxes stay `None`.
///
/// `media` is the inherited MediaBox, or US Letter (612 by 792) when no
/// ancestor defines one. That fallback is only for the reported value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageBoxes {
    /// Inherited MediaBox, or US Letter when the page tree does not define one.
    pub media: Rect,
    /// Inherited CropBox.
    pub crop: Option<Rect>,
    /// Inherited BleedBox.
    pub bleed: Option<Rect>,
    /// Inherited TrimBox.
    pub trim: Option<Rect>,
    /// Inherited ArtBox.
    pub art: Option<Rect>,
}

/// Inserts an empty page at `index`.
///
/// `index == page_count` appends. Width and height are in points and must be
/// finite and inside `(0, 14400]`. The new page has an empty content stream
/// and its own MediaBox. Returns the index where the page landed.
pub fn insert_blank_page(
    doc: &mut PdfDocument,
    index: usize,
    width: f64,
    height: f64,
) -> PdfResult<usize> {
    require_page_size(width, height)?;
    let old_count = doc.get_pages()?.len();
    require_insert_at(index, old_count)?;

    let pages_id = doc.pages_id()?;
    let content_id = doc.alloc_object_id();
    let mut stream_dict = PdfDictionary::new();
    stream_dict.insert("Length", 0i64);
    doc.set_object(
        content_id,
        PdfObject::Stream(PdfStream::new(stream_dict, Vec::new())),
    );

    let page_id = doc.alloc_object_id();
    let mut page_dict = PdfDictionary::new();
    page_dict.insert("Type", PdfName::new("Page"));
    page_dict.insert("Parent", pages_id);
    page_dict.insert(
        "MediaBox",
        vec![
            PdfObject::Real(0.0),
            PdfObject::Real(0.0),
            PdfObject::Real(width),
            PdfObject::Real(height),
        ],
    );
    page_dict.insert("Contents", content_id);
    page_dict.insert("Resources", PdfDictionary::new());
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    append_page(doc, page_id)?;
    place_appended_block(doc, old_count, 1, index)?;
    Ok(index)
}

/// Copies `source_index` and inserts the copy at `insert_at`.
///
/// The copy is built in a temporary document and imported back, so the two
/// pages do not share a content stream. `insert_at == page_count` appends.
/// Returns the index where the copy landed.
pub fn duplicate_page(
    doc: &mut PdfDocument,
    source_index: usize,
    insert_at: usize,
) -> PdfResult<usize> {
    let old_count = doc.get_pages()?.len();
    if source_index >= old_count {
        return Err(PdfError::InvalidPageNumber {
            page: source_index + 1,
            total: old_count,
        });
    }
    require_insert_at(insert_at, old_count)?;

    let mut donor = extract_pages(doc, &[source_index])?;
    let donor_pages = donor.get_pages()?;
    let source_page = *donor_pages.first().ok_or_else(|| {
        PdfError::OperationError("The donor document has no page to copy".to_string())
    })?;
    let mut cloner = ObjectCloner::new(&mut donor)?;
    cloner.import_page(&mut donor, source_page, doc)?;
    place_appended_block(doc, old_count, 1, insert_at)?;
    Ok(insert_at)
}

/// Copies `source_indices` from `source` into `dest` at `insert_at`.
///
/// Indices are 0-based, must be unique, and the call copies at most 200 pages.
/// `source` is read and may cache parsed objects; it is not otherwise modified.
/// Returns the number of pages inserted.
pub fn insert_pages(
    dest: &mut PdfDocument,
    source: &mut PdfDocument,
    source_indices: &[usize],
    insert_at: usize,
) -> PdfResult<usize> {
    if source_indices.is_empty() {
        return Err(PdfError::OperationError(
            "At least one source page is required".to_string(),
        ));
    }
    if source_indices.len() > MAX_INSERT_PAGES {
        return Err(PdfError::OperationError(format!(
            "One insert copies at most {MAX_INSERT_PAGES} pages"
        )));
    }

    let source_pages = source.get_pages()?;
    let mut seen = HashSet::with_capacity(source_indices.len());
    for &index in source_indices {
        if index >= source_pages.len() {
            return Err(PdfError::InvalidPageNumber {
                page: index + 1,
                total: source_pages.len(),
            });
        }
        if !seen.insert(index) {
            return Err(PdfError::OperationError(format!(
                "Source page index {index} is repeated; each copied page needs its own content stream"
            )));
        }
    }

    let old_count = dest.get_pages()?.len();
    require_insert_at(insert_at, old_count)?;

    let mut cloner = ObjectCloner::new(source)?;
    for &index in source_indices {
        cloner.import_page(source, source_pages[index], dest)?;
    }
    place_appended_block(dest, old_count, source_indices.len(), insert_at)?;
    Ok(source_indices.len())
}

/// Reverses the page order. A document with fewer than two pages is unchanged.
pub fn reverse_pages(doc: &mut PdfDocument) -> PdfResult<()> {
    let count = doc.get_pages()?.len();
    let order: Vec<usize> = (0..count).rev().collect();
    reorder_pages(doc, &order)
}

/// Reads the five page boundaries, following `/Parent` for at most 16 nodes.
///
/// A cycle aborts the read. A key present on the page is that page's value
/// even when an ancestor also defines the same key.
pub fn page_boxes(doc: &mut PdfDocument, page_index: usize) -> PdfResult<PageBoxes> {
    let page_id = page_id_at(doc, page_index)?;
    let media = match inherited_rect(doc, page_id, PageBoundary::Media.key())? {
        Some(rect) => rect,
        None => Rect::new(0.0, 0.0, 612.0, 792.0),
    };
    Ok(PageBoxes {
        media,
        crop: inherited_rect(doc, page_id, PageBoundary::Crop.key())?,
        bleed: inherited_rect(doc, page_id, PageBoundary::Bleed.key())?,
        trim: inherited_rect(doc, page_id, PageBoundary::Trim.key())?,
        art: inherited_rect(doc, page_id, PageBoundary::Art.key())?,
    })
}

/// Writes or clears one page boundary on the page itself.
///
/// `corners` is `[x0, y0, x1, y1]` before sorting. `None` removes the key from
/// this page only: an inherited ancestor value remains visible to [`page_boxes`].
/// MediaBox cannot be removed. A new optional box must intersect the resolved
/// MediaBox. Changing MediaBox does not move the other boxes.
pub fn set_page_boundary(
    doc: &mut PdfDocument,
    page_index: usize,
    kind: &str,
    corners: Option<&[f64]>,
) -> PdfResult<PageBoxes> {
    let boundary = parse_page_boundary(kind).ok_or_else(|| {
        PdfError::OperationError(format!(
            "Unknown page box '{kind}'. Use media, crop, bleed, trim, or art"
        ))
    })?;
    let page_id = page_id_at(doc, page_index)?;

    let stored = match corners {
        None => {
            if boundary == PageBoundary::Media {
                return Err(PdfError::OperationError(
                    "MediaBox cannot be removed".to_string(),
                ));
            }
            None
        }
        Some(raw) => {
            if raw.len() != 4 {
                return Err(PdfError::OperationError(
                    "A page box needs four numbers: x0, y0, x1, y1".to_string(),
                ));
            }
            if raw.iter().any(|value| !value.is_finite()) {
                return Err(PdfError::OperationError(
                    "A page box must use finite coordinates".to_string(),
                ));
            }
            let rect = validate_box(Rect::new(raw[0], raw[1], raw[2], raw[3]))?;
            if boundary != PageBoundary::Media {
                let media = page_boxes(doc, page_index)?.media;
                if !rect.intersects(&media) {
                    return Err(PdfError::OperationError(
                        "The page box must intersect the page MediaBox".to_string(),
                    ));
                }
            }
            Some(rect)
        }
    };

    let mut page_dict = match doc.get_object(page_id)? {
        PdfObject::Dictionary(dict) => dict,
        _ => {
            return Err(PdfError::OperationError(
                "Page object is not a dictionary".to_string(),
            ))
        }
    };
    match stored {
        Some(rect) => page_dict.insert(boundary.key(), rect_array(rect)),
        None => {
            page_dict.remove(boundary.key());
        }
    }
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));
    page_boxes(doc, page_index)
}

fn require_page_size(width: f64, height: f64) -> PdfResult<()> {
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || width > MAX_USER_UNIT
        || height > MAX_USER_UNIT
    {
        return Err(PdfError::OperationError(
            "Page width and height must be finite and in (0, 14400] points".to_string(),
        ));
    }
    Ok(())
}

fn require_insert_at(insert_at: usize, page_count: usize) -> PdfResult<()> {
    if insert_at > page_count {
        return Err(PdfError::InvalidPageNumber {
            page: insert_at + 1,
            total: page_count,
        });
    }
    Ok(())
}

fn validate_box(rect: Rect) -> PdfResult<Rect> {
    let edges = [rect.min_x, rect.min_y, rect.max_x, rect.max_y];
    if edges.iter().any(|value| !value.is_finite()) {
        return Err(PdfError::OperationError(
            "A page box must use finite coordinates".to_string(),
        ));
    }
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return Err(PdfError::OperationError(
            "A page box must have a positive width and height".to_string(),
        ));
    }
    if edges.iter().any(|value| value.abs() > MAX_USER_UNIT) {
        return Err(PdfError::OperationError(
            "A page box coordinate must stay within 14400 points of the origin".to_string(),
        ));
    }
    Ok(rect)
}

fn rect_array(rect: Rect) -> PdfObject {
    PdfObject::Array(vec![
        PdfObject::Real(rect.min_x),
        PdfObject::Real(rect.min_y),
        PdfObject::Real(rect.max_x),
        PdfObject::Real(rect.max_y),
    ])
}

fn page_id_at(doc: &mut PdfDocument, page_index: usize) -> PdfResult<ObjectId> {
    let pages = doc.get_pages()?;
    pages.get(page_index).copied().ok_or(PdfError::InvalidPageNumber {
        page: page_index + 1,
        total: pages.len(),
    })
}

fn append_page(doc: &mut PdfDocument, page_id: ObjectId) -> PdfResult<()> {
    let pages_id = doc.pages_id()?;
    let mut pages_dict = match doc.get_object(pages_id)? {
        PdfObject::Dictionary(dict) => dict,
        _ => {
            return Err(PdfError::OperationError(
                "Root /Pages is not a dictionary".to_string(),
            ))
        }
    };
    let mut kids = match pages_dict.get("Kids") {
        Some(PdfObject::Array(items)) => items.clone(),
        Some(_) => {
            return Err(PdfError::OperationError(
                "Root /Pages /Kids must be a direct array".to_string(),
            ))
        }
        None => Vec::new(),
    };
    kids.push(PdfObject::Reference(page_id));
    let count = kids.len() as i64;
    pages_dict.insert("Kids", PdfObject::Array(kids));
    pages_dict.insert("Count", count);
    doc.set_object(pages_id, PdfObject::Dictionary(pages_dict));
    Ok(())
}

/// Moves the block that was just appended (`old_count..old_count+added`) to `insert_at`.
fn place_appended_block(
    doc: &mut PdfDocument,
    old_count: usize,
    added: usize,
    insert_at: usize,
) -> PdfResult<()> {
    let total = old_count + added;
    let mut order = Vec::with_capacity(total);
    order.extend(0..insert_at);
    order.extend(old_count..total);
    order.extend(insert_at..old_count);
    reorder_pages(doc, &order)
}

fn inherited_rect(
    doc: &mut PdfDocument,
    start: ObjectId,
    key: &str,
) -> PdfResult<Option<Rect>> {
    let mut current = start;
    let mut seen = HashSet::new();
    for _ in 0..MAX_PARENT_HOPS {
        if !seen.insert(current) {
            return Err(PdfError::CircularReference {
                id: current.number,
                gen: current.generation,
            });
        }
        let dict = match doc.get_object(current)? {
            PdfObject::Dictionary(dict) => dict,
            _ => {
                return Err(PdfError::OperationError(
                    "A page-tree node is not a dictionary".to_string(),
                ))
            }
        };
        if let Some(value) = dict.get(key).cloned() {
            return read_box(doc, &value).map(Some);
        }
        match dict.get("Parent").and_then(|parent| parent.as_reference()) {
            Some(parent) => current = parent,
            None => return Ok(None),
        }
    }
    Err(PdfError::OperationError(
        "Page box inheritance exceeded 16 parent nodes".to_string(),
    ))
}

fn read_box(doc: &mut PdfDocument, value: &PdfObject) -> PdfResult<Rect> {
    let owned = match value {
        PdfObject::Reference(id) => doc.get_object(*id)?,
        other => other.clone(),
    };
    let array = owned.as_array().ok_or_else(|| {
        PdfError::OperationError("A page box must be an array of four numbers".to_string())
    })?;
    if array.len() != 4 {
        return Err(PdfError::OperationError(
            "A page box must contain four numbers".to_string(),
        ));
    }
    let mut corners = [0.0; 4];
    for (index, item) in array.iter().enumerate() {
        let number = item.as_f64().ok_or_else(|| {
            PdfError::OperationError("A page box entry is not a number".to_string())
        })?;
        if !number.is_finite() {
            return Err(PdfError::OperationError(
                "A page box entry must be a finite number".to_string(),
            ));
        }
        corners[index] = number;
    }
    Ok(Rect::new(corners[0], corners[1], corners[2], corners[3]))
}
