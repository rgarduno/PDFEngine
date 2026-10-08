//! Ink strokes and vector shapes (ISO 32000-1 §12.5.6.13–§12.5.6.15).
//!
//! Each mark stores the annotation dictionary and a normal appearance stream,
//! so a viewer paints the stroke without changing the page content.

use crate::annots::markup::attach_annotation_to_page;
use crate::annots::types::AnnotationSubtype;
use crate::cos::object::{ObjectId, PdfDictionary, PdfObject, PdfStream};
use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::layout::geometry::Rect;

const MAX_INK_POINTS: usize = 2_000;
const MAX_POLYGON_POINTS: usize = 200;
const COORD_LIMIT: f64 = 20_000.0;

/// A mark that can be drawn on a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    Ink,
    Square,
    Circle,
    Line,
    Polygon,
}

impl ShapeKind {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Ink" => Some(ShapeKind::Ink),
            "Square" => Some(ShapeKind::Square),
            "Circle" => Some(ShapeKind::Circle),
            "Line" => Some(ShapeKind::Line),
            "Polygon" => Some(ShapeKind::Polygon),
            _ => None,
        }
    }

    fn subtype(self) -> AnnotationSubtype {
        match self {
            ShapeKind::Ink => AnnotationSubtype::Ink,
            ShapeKind::Square => AnnotationSubtype::Square,
            ShapeKind::Circle => AnnotationSubtype::Circle,
            ShapeKind::Line => AnnotationSubtype::Line,
            ShapeKind::Polygon => AnnotationSubtype::Polygon,
        }
    }
}

/// Stroke, optional fill, width, and opacity for one mark.
#[derive(Debug, Clone, Copy)]
pub struct ShapeStyle {
    pub stroke: [f64; 3],
    pub fill: Option<[f64; 3]>,
    pub line_width: f64,
    pub opacity: f64,
    /// Ending at the second point of a line: `OpenArrow` or `ClosedArrow`.
    pub line_ending: Option<&'static str>,
}

/// Adds an ink stroke or a vector shape and returns the annotation object id.
pub fn add_shape(
    doc: &mut PdfDocument,
    page_index: usize,
    kind: ShapeKind,
    points: &[[f64; 2]],
    style: ShapeStyle,
) -> PdfResult<ObjectId> {
    let pages = doc.get_pages()?;
    let page_id = pages
        .get(page_index)
        .copied()
        .ok_or_else(|| PdfError::InvalidPageNumber {
            page: page_index + 1,
            total: pages.len(),
        })?;
    let style = checked_style(kind, style)?;
    let geometry = geometry_for(kind, points, &style)?;

    let ap_id = write_appearance(doc, &geometry, &style)?;
    let annot_id = doc.alloc_object_id();
    let mut annot = PdfDictionary::new();
    annot.insert("Type", PdfObject::Name("Annot".into()));
    annot.insert(
        "Subtype",
        PdfObject::Name(kind.subtype().as_pdf_name().into()),
    );
    annot.insert("Rect", rect_array(&geometry.rect));
    annot.insert("C", color_array(style.stroke));
    annot.insert("CA", PdfObject::Real(style.opacity));
    annot.insert("F", PdfObject::Integer(4));
    annot.insert("P", PdfObject::Reference(page_id));
    annot.insert("BS", border_style(style.line_width));
    if let Some(fill) = geometry.fill {
        annot.insert("IC", color_array(fill));
    }
    match kind {
        ShapeKind::Ink => {
            annot.insert("InkList", ink_list(&geometry.points));
        }
        ShapeKind::Line => {
            annot.insert("L", number_array(&geometry.points));
            if let Some(ending) = style.line_ending {
                annot.insert(
                    "LE",
                    PdfObject::Array(vec![
                        PdfObject::Name("None".into()),
                        PdfObject::Name(ending.into()),
                    ]),
                );
            }
        }
        ShapeKind::Polygon => {
            annot.insert("Vertices", number_array(&geometry.points));
        }
        ShapeKind::Square | ShapeKind::Circle => {}
    }
    let mut appearance = PdfDictionary::new();
    appearance.insert("N", PdfObject::Reference(ap_id));
    annot.insert("AP", PdfObject::Dictionary(appearance));
    doc.set_object(annot_id, PdfObject::Dictionary(annot));
    attach_annotation_to_page(doc, page_id, annot_id)?;
    Ok(annot_id)
}

struct Geometry {
    rect: Rect,
    points: Vec<[f64; 2]>,
    fill: Option<[f64; 3]>,
    kind: ShapeKind,
}

fn geometry_for(kind: ShapeKind, points: &[[f64; 2]], style: &ShapeStyle) -> PdfResult<Geometry> {
    for point in points {
        if !point[0].is_finite()
            || !point[1].is_finite()
            || point[0].abs() > COORD_LIMIT
            || point[1].abs() > COORD_LIMIT
        {
            return Err(PdfError::OperationError("Shape point was rejected.".into()));
        }
    }
    let limit = match kind {
        ShapeKind::Ink => MAX_INK_POINTS,
        ShapeKind::Polygon => MAX_POLYGON_POINTS,
        ShapeKind::Line | ShapeKind::Square | ShapeKind::Circle => 2,
    };
    if points.len() > limit {
        return Err(PdfError::OperationError(
            "A shape has too many points.".into(),
        ));
    }
    match kind {
        ShapeKind::Ink if points.len() < 2 => {
            return Err(PdfError::OperationError(
                "A shape needs more points.".into(),
            ));
        }
        ShapeKind::Line if points.len() != 2 => {
            return Err(PdfError::OperationError(
                "A shape needs more points.".into(),
            ));
        }
        ShapeKind::Polygon if points.len() < 3 => {
            return Err(PdfError::OperationError(
                "A shape needs more points.".into(),
            ));
        }
        ShapeKind::Square | ShapeKind::Circle if points.len() != 2 => {
            return Err(PdfError::OperationError(
                "A shape needs more points.".into(),
            ));
        }
        _ => {}
    }

    let owned = points.to_vec();
    let pad = style.line_width.max(1.0)
        + if style.line_ending.is_some() {
            14.0
        } else {
            1.0
        };
    let rect = match kind {
        ShapeKind::Square | ShapeKind::Circle => {
            let rect = Rect::new(points[0][0], points[0][1], points[1][0], points[1][1]);
            if rect.width() < 1.0 || rect.height() < 1.0 {
                return Err(PdfError::OperationError(
                    "A shape needs more points.".into(),
                ));
            }
            rect
        }
        _ => bounds(&owned, pad)?,
    };
    let fill = match kind {
        ShapeKind::Square | ShapeKind::Circle | ShapeKind::Polygon => style.fill,
        ShapeKind::Ink | ShapeKind::Line => None,
    };
    Ok(Geometry {
        rect,
        points: owned,
        fill,
        kind,
    })
}

fn bounds(points: &[[f64; 2]], pad: f64) -> PdfResult<Rect> {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    for point in points {
        min_x = min_x.min(point[0]);
        min_y = min_y.min(point[1]);
        max_x = max_x.max(point[0]);
        max_y = max_y.max(point[1]);
    }
    if !min_x.is_finite() {
        return Err(PdfError::OperationError(
            "A shape needs more points.".into(),
        ));
    }
    let rect = Rect::new(min_x - pad, min_y - pad, max_x + pad, max_y + pad);
    if rect.width() < 1.0 {
        return Ok(Rect::new(
            rect.min_x,
            rect.min_y,
            rect.min_x + 1.0,
            rect.max_y.max(rect.min_y + 1.0),
        ));
    }
    if rect.height() < 1.0 {
        return Ok(Rect::new(
            rect.min_x,
            rect.min_y,
            rect.max_x,
            rect.min_y + 1.0,
        ));
    }
    Ok(rect)
}

fn checked_style(kind: ShapeKind, mut style: ShapeStyle) -> PdfResult<ShapeStyle> {
    if style
        .stroke
        .iter()
        .any(|channel| !channel.is_finite() || !(0.0..=1.0).contains(channel))
    {
        return Err(PdfError::OperationError("Shape color was rejected.".into()));
    }
    if let Some(fill) = style.fill {
        if fill
            .iter()
            .any(|channel| !channel.is_finite() || !(0.0..=1.0).contains(channel))
        {
            return Err(PdfError::OperationError("Shape color was rejected.".into()));
        }
    }
    if !style.line_width.is_finite() || !(0.25..=24.0).contains(&style.line_width) {
        return Err(PdfError::OperationError(
            "Shape line width was rejected.".into(),
        ));
    }
    if !style.opacity.is_finite() || !(0.05..=1.0).contains(&style.opacity) {
        return Err(PdfError::OperationError("Shape color was rejected.".into()));
    }
    if kind != ShapeKind::Line {
        style.line_ending = None;
    } else if let Some(ending) = style.line_ending {
        if ending != "OpenArrow" && ending != "ClosedArrow" {
            return Err(PdfError::OperationError("Shape kind was rejected.".into()));
        }
    }
    Ok(style)
}

fn write_appearance(
    doc: &mut PdfDocument,
    geometry: &Geometry,
    style: &ShapeStyle,
) -> PdfResult<ObjectId> {
    let width = geometry.rect.width();
    let height = geometry.rect.height();
    let mut ops = String::new();
    ops.push_str("q\n");
    if style.opacity < 0.999 {
        ops.push_str("/GS0 gs\n");
    }
    ops.push_str(&appearance_path(geometry, style));
    ops.push_str("Q\n");

    let mut dict = PdfDictionary::new();
    dict.insert("Type", PdfObject::Name("XObject".into()));
    dict.insert("Subtype", PdfObject::Name("Form".into()));
    dict.insert(
        "BBox",
        PdfObject::Array(vec![
            PdfObject::Real(0.0),
            PdfObject::Real(0.0),
            PdfObject::Real(width),
            PdfObject::Real(height),
        ]),
    );
    let mut resources = PdfDictionary::new();
    if style.opacity < 0.999 {
        let mut graphics = PdfDictionary::new();
        graphics.insert("Type", PdfObject::Name("ExtGState".into()));
        graphics.insert("ca", PdfObject::Real(style.opacity));
        graphics.insert("CA", PdfObject::Real(style.opacity));
        let mut states = PdfDictionary::new();
        states.insert("GS0", PdfObject::Dictionary(graphics));
        resources.insert("ExtGState", PdfObject::Dictionary(states));
    }
    dict.insert("Resources", PdfObject::Dictionary(resources));
    let id = doc.alloc_object_id();
    doc.set_object(
        id,
        PdfObject::Stream(PdfStream::new(dict, ops.into_bytes())),
    );
    Ok(id)
}

fn appearance_path(geometry: &Geometry, style: &ShapeStyle) -> String {
    let mut ops = String::new();
    let local = |point: [f64; 2]| {
        (
            point[0] - geometry.rect.min_x,
            point[1] - geometry.rect.min_y,
        )
    };
    ops.push_str(&format!(
        "{:.3} {:.3} {:.3} RG\n{:.2} w\n",
        style.stroke[0], style.stroke[1], style.stroke[2], style.line_width
    ));
    if let Some(fill) = geometry.fill {
        ops.push_str(&format!(
            "{:.3} {:.3} {:.3} rg\n",
            fill[0], fill[1], fill[2]
        ));
    }
    let width = geometry.rect.width();
    let height = geometry.rect.height();
    let paint = if geometry.fill.is_some() {
        "B\n"
    } else {
        "S\n"
    };
    match geometry.kind {
        ShapeKind::Square => {
            ops.push_str(&format!("0 0 {:.2} {:.2} re\n{}", width, height, paint));
        }
        ShapeKind::Circle => {
            ops.push_str(&ellipse_path(width, height));
            ops.push_str(paint);
        }
        ShapeKind::Line => {
            let (x1, y1) = local(geometry.points[0]);
            let (x2, y2) = local(geometry.points[1]);
            if style.line_ending.is_some() {
                ops.push_str(&arrow_path(x1, y1, x2, y2, style.line_width));
            } else {
                ops.push_str(&format!("{:.2} {:.2} m {:.2} {:.2} l S\n", x1, y1, x2, y2));
            }
        }
        ShapeKind::Ink => {
            stroke_polyline(&mut ops, geometry, local);
            ops.push_str("S\n");
        }
        ShapeKind::Polygon => {
            stroke_polyline(&mut ops, geometry, local);
            ops.push_str("h\n");
            ops.push_str(paint);
        }
    }
    ops
}

fn stroke_polyline(ops: &mut String, geometry: &Geometry, local: impl Fn([f64; 2]) -> (f64, f64)) {
    for (index, point) in geometry.points.iter().copied().enumerate() {
        let (x, y) = local(point);
        if index == 0 {
            ops.push_str(&format!("{:.2} {:.2} m\n", x, y));
        } else {
            ops.push_str(&format!("{:.2} {:.2} l\n", x, y));
        }
    }
}

fn ellipse_path(width: f64, height: f64) -> String {
    let cx = width / 2.0;
    let cy = height / 2.0;
    let rx = width / 2.0;
    let ry = height / 2.0;
    let kx = rx * 0.5522847498;
    let ky = ry * 0.5522847498;
    format!(
        "{:.2} {:.2} m\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\nh\n",
        cx, cy + ry,
        cx + kx, cy + ry, cx + rx, cy + ky, cx + rx, cy,
        cx + rx, cy - ky, cx + kx, cy - ry, cx, cy - ry,
        cx - kx, cy - ry, cx - rx, cy - ky, cx - rx, cy,
        cx - rx, cy + ky, cx - kx, cy + ry, cx, cy + ry,
    )
}

fn arrow_path(x1: f64, y1: f64, x2: f64, y2: f64, line_width: f64) -> String {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    let ux = dx / len;
    let uy = dy / len;
    let size = (line_width * 4.0).clamp(6.0, 18.0).min(len * 0.45);
    let px = -uy;
    let py = ux;
    let base_x = x2 - ux * size;
    let base_y = y2 - uy * size;
    let left_x = base_x + px * size * 0.45;
    let left_y = base_y + py * size * 0.45;
    let right_x = base_x - px * size * 0.45;
    let right_y = base_y - py * size * 0.45;
    format!(
        "{:.2} {:.2} m {:.2} {:.2} l S\n{:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l h f\n",
        x1, y1, base_x, base_y, x2, y2, left_x, left_y, right_x, right_y
    )
}

fn rect_array(rect: &Rect) -> PdfObject {
    PdfObject::Array(vec![
        PdfObject::Real(rect.min_x),
        PdfObject::Real(rect.min_y),
        PdfObject::Real(rect.max_x),
        PdfObject::Real(rect.max_y),
    ])
}

fn color_array(color: [f64; 3]) -> PdfObject {
    PdfObject::Array(vec![
        PdfObject::Real(color[0]),
        PdfObject::Real(color[1]),
        PdfObject::Real(color[2]),
    ])
}

fn number_array(points: &[[f64; 2]]) -> PdfObject {
    let mut values = Vec::with_capacity(points.len() * 2);
    for point in points {
        values.push(PdfObject::Real(point[0]));
        values.push(PdfObject::Real(point[1]));
    }
    PdfObject::Array(values)
}

fn ink_list(points: &[[f64; 2]]) -> PdfObject {
    PdfObject::Array(vec![number_array(points)])
}

fn border_style(width: f64) -> PdfObject {
    let mut style = PdfDictionary::new();
    style.insert("W", PdfObject::Real(width));
    style.insert("S", PdfObject::Name("S".into()));
    PdfObject::Dictionary(style)
}
