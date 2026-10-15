//! Page rasterizer (ISO 32000-1 §8 and §9).
//!
//! Paints one page into an RGB bitmap and encodes PNG, JPEG, or baseline TIFF.
//! The displayed box is the CropBox, or the MediaBox when CropBox is absent
//! (§14.11.2). `/Rotate` is applied clockwise after painting (§7.7.3.3).
//! Annotation appearances are not composited (§12.5). Rendering mode 3 is not
//! painted (§9.3.6). Text without an outline program uses a small built-in
//! face, which is a display approximation and not the embedded font. Nested
//! clips, soft masks, and transparency groups are not applied. TIFF output is
//! a single page, not a multi-page stack.

use std::collections::{BTreeMap, HashSet};
use std::io::Cursor;

use crate::cos::{decode_stream, ObjectId, PdfDictionary, PdfDocument, PdfObject, PdfStream};
use crate::error::{PdfError, PdfResult};
use crate::fonts::{
    resolve_page_fonts, FontMetrics, ResolvedFont, ToUnicodeMap, WinAnsiEncoding,
};
use crate::images::{encode_png, parse_jpeg};
use crate::layout::Rect;
use crate::ops::compose::page_boxes;
use crate::ops::rotation::get_page_rotation;
use crate::stream::{ContentStreamTokenizer, GraphicsStateStack, Matrix, Operation};

/// Lowest accepted raster density, in dots per inch.
pub const MIN_RASTER_DPI: f64 = 36.0;
/// Highest accepted raster density, in dots per inch.
pub const MAX_RASTER_DPI: f64 = 300.0;
/// Density used when a caller does not choose one.
pub const DEFAULT_RASTER_DPI: f64 = 144.0;
/// Maximum pixels in one bitmap (`width * height`).
pub const MAX_RASTER_PIXELS: u64 = 12_000_000;
/// Maximum pixels along one side. Also the JPEG encoder's `u16` limit.
pub const MAX_RASTER_SIDE: u32 = 65535;

const MAX_Q_DEPTH: u32 = 32;
const MAX_FORM_DEPTH: u32 = 8;
const MAX_PARENT_HOPS: usize = 16;
const MAX_PATH_POINTS: usize = 100_000;
const MAX_FORM_WIDTHS: usize = 1024;

/// Encoded image of a single page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RasterFormat {
    /// PNG, color type 2.
    Png,
    /// Baseline JPEG.
    Jpeg,
    /// Uncompressed little-endian RGB TIFF, one page.
    Tiff,
}

impl RasterFormat {
    /// Accepts `png`, `jpeg`/`jpg`, and `tiff`/`tif`, ignoring case and surrounding space.
    pub fn parse(value: &str) -> PdfResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "png" => Ok(Self::Png),
            "jpeg" | "jpg" => Ok(Self::Jpeg),
            "tiff" | "tif" => Ok(Self::Tiff),
            _ => Err(raster_err("Raster format must be png, jpeg, or tiff.")),
        }
    }

    /// MIME type of the encoded bytes.
    pub fn media_type(&self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Tiff => "image/tiff",
        }
    }

    /// File extension, without a leading dot. JPEG is `jpg` and TIFF is `tif`.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Tiff => "tif",
        }
    }
}

/// One painted page, already encoded.
#[derive(Debug, Clone, PartialEq)]
pub struct PageRaster {
    /// Bitmap width in pixels, after `/Rotate`.
    pub width: u32,
    /// Bitmap height in pixels, after `/Rotate`.
    pub height: u32,
    /// Density requested by the caller.
    pub dpi: f64,
    /// Encoded PNG, JPEG, or TIFF bytes.
    pub bytes: Vec<u8>,
    /// Features skipped while painting (unsupported images, shadings, inline images).
    pub omitted: u32,
}

/// RGB bitmap before encoding. Tests inspect pixels here.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PaintedPage {
    /// Bitmap width in pixels, after `/Rotate`.
    pub width: u32,
    /// Bitmap height in pixels, after `/Rotate`.
    pub height: u32,
    /// Density requested by the caller.
    pub dpi: f64,
    /// Tightly packed RGB, row 0 at the top.
    pub pixels: Vec<u8>,
    /// Features skipped while painting.
    pub omitted: u32,
}

/// Paints `page_index` (0-based) and encodes it.
///
/// This is a read. It does not rewrite the page. See the module note for the
/// boxes, rotation, and the features this painter does not composite.
pub fn render_page(
    doc: &mut PdfDocument,
    page_index: usize,
    format: RasterFormat,
    dpi: f64,
) -> PdfResult<PageRaster> {
    doc.require_print()?;
    let painted = paint_page_rgb(doc, page_index, dpi)?;
    let bytes = match format {
        RasterFormat::Png => encode_png(painted.width, painted.height, &painted.pixels, true)?,
        RasterFormat::Jpeg => encode_jpeg(&painted.pixels, painted.width, painted.height, 85)?,
        RasterFormat::Tiff => {
            encode_tiff(painted.width, painted.height, painted.dpi, &painted.pixels)?
        }
    };
    Ok(PageRaster {
        width: painted.width,
        height: painted.height,
        dpi: painted.dpi,
        bytes,
        omitted: painted.omitted,
    })
}

/// Paints one page to RGB after applying `/Rotate`. The buffer is white.
pub(crate) fn paint_page_rgb(
    doc: &mut PdfDocument,
    page_index: usize,
    dpi: f64,
) -> PdfResult<PaintedPage> {
    if !dpi.is_finite() || dpi < MIN_RASTER_DPI || dpi > MAX_RASTER_DPI {
        return Err(raster_err(
            "Raster dpi must be a finite number from 36 to 300.",
        ));
    }
    let pages = doc.get_pages()?;
    if page_index >= pages.len() {
        return Err(PdfError::InvalidPageNumber {
            page: page_index,
            total: pages.len(),
        });
    }
    let page_id = pages[page_index];
    let boxes = page_boxes(doc, page_index)?;
    let rotation = get_page_rotation(doc, page_index)?;
    let content = doc.get_page_content_bytes(page_id)?;
    let fonts = resolve_page_fonts(doc, page_id)?;
    let resources = inherited_resources(doc, page_id)?;
    let display = boxes.crop.unwrap_or(boxes.media);
    if !finite_rect(display) || display.width() <= 0.0 || display.height() <= 0.0 {
        return Err(raster_err("Page box is not a finite positive rectangle."));
    }
    let width = side_px(display.width(), dpi)?;
    let height = side_px(display.height(), dpi)?;
    let count = (width as u64).saturating_mul(height as u64);
    if count > MAX_RASTER_PIXELS {
        return Err(raster_err("Raster would exceed 12000000 pixels."));
    }
    let nbytes = buffer_len(width, height)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(nbytes)
        .map_err(|_| raster_err("Raster buffer could not be allocated."))?;
    pixels.resize(nbytes, 255);

    let mut painter = Painter {
        pixels,
        width,
        height,
        scale: dpi / 72.0,
        crop: display,
        gs: GraphicsStateStack::new(),
        style: Style::default(),
        style_stack: Vec::new(),
        path: Path::default(),
        fonts,
        resources,
        omitted: 0,
        in_text: false,
        form_depth: 0,
    };
    let ops = ContentStreamTokenizer::new(&content).tokenize_all()?;
    painter.interpret_ops(doc, &ops)?;
    let omitted = painter.omitted;
    let (pixels, width, height) = rotate_rgb(painter.pixels, width, height, rotation)?;
    Ok(PaintedPage {
        width,
        height,
        dpi,
        pixels,
        omitted,
    })
}

fn raster_err(message: &str) -> PdfError {
    PdfError::OperationError(message.to_string())
}

fn finite_rect(rect: Rect) -> bool {
    rect.min_x.is_finite()
        && rect.min_y.is_finite()
        && rect.max_x.is_finite()
        && rect.max_y.is_finite()
}

fn side_px(points: f64, dpi: f64) -> PdfResult<u32> {
    let px = (points * dpi / 72.0).round();
    if !px.is_finite() || px < 1.0 {
        return Err(raster_err("Raster side must be at least 1 pixel."));
    }
    if px > MAX_RASTER_SIDE as f64 {
        return Err(raster_err("Raster side would exceed 65535 pixels."));
    }
    Ok(px as u32)
}

fn buffer_len(width: u32, height: u32) -> PdfResult<usize> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(3))
        .ok_or_else(|| raster_err("Raster buffer could not be allocated."))
}

fn byte_from_unit(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn gray_bytes(g: f64) -> [u8; 3] {
    let b = byte_from_unit(g);
    [b, b, b]
}

fn rgb_bytes(r: f64, g: f64, b: f64) -> [u8; 3] {
    [byte_from_unit(r), byte_from_unit(g), byte_from_unit(b)]
}

fn cmyk_bytes(c: f64, m: f64, y: f64, k: f64) -> [u8; 3] {
    let (c, m, y, k) = (
        c.clamp(0.0, 1.0),
        m.clamp(0.0, 1.0),
        y.clamp(0.0, 1.0),
        k.clamp(0.0, 1.0),
    );
    [
        byte_from_unit((1.0 - c) * (1.0 - k)),
        byte_from_unit((1.0 - m) * (1.0 - k)),
        byte_from_unit((1.0 - y) * (1.0 - k)),
    ]
}

#[derive(Debug, Clone, Copy)]
struct Style {
    fill: [u8; 3],
    stroke: [u8; 3],
    line_width: f64,
    fill_n: u8,
    stroke_n: u8,
    clip: Option<Rect>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: [0, 0, 0],
            stroke: [0, 0, 0],
            line_width: 1.0,
            fill_n: 1,
            stroke_n: 1,
            clip: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Subpath {
    points: Vec<(f64, f64)>,
    closed: bool,
}

#[derive(Debug, Clone, Default)]
struct Path {
    subpaths: Vec<Subpath>,
    current_user: Option<(f64, f64)>,
    start_user: Option<(f64, f64)>,
    blocked: bool,
    point_count: usize,
}

struct RgbImage {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

struct FormJob {
    ops: Vec<Operation>,
    resources: Option<PdfDictionary>,
    fonts: Option<BTreeMap<String, ResolvedFont>>,
    matrix: Matrix,
    bbox: Option<[f64; 4]>,
}

struct Painter {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    scale: f64,
    crop: Rect,
    gs: GraphicsStateStack,
    style: Style,
    style_stack: Vec<Style>,
    path: Path,
    fonts: BTreeMap<String, ResolvedFont>,
    resources: PdfDictionary,
    omitted: u32,
    in_text: bool,
    form_depth: u32,
}

impl Painter {
    fn omit(&mut self) {
        self.omitted = self.omitted.saturating_add(1);
    }

    fn interpret_ops(&mut self, doc: &mut PdfDocument, ops: &[Operation]) -> PdfResult<()> {
        let mut q_depth = 0u32;
        let result = self.interpret_loop(doc, ops, &mut q_depth);
        self.drain_q(&mut q_depth);
        result
    }

    fn interpret_loop(
        &mut self,
        doc: &mut PdfDocument,
        ops: &[Operation],
        q_depth: &mut u32,
    ) -> PdfResult<()> {
        for op in ops {
            self.dispatch(doc, op, q_depth)?;
        }
        Ok(())
    }

    fn dispatch(
        &mut self,
        doc: &mut PdfDocument,
        op: &Operation,
        q_depth: &mut u32,
    ) -> PdfResult<()> {
        match op.operator.as_str() {
            "q" => self.push_q(q_depth),
            "Q" => self.pop_q(q_depth),
            "cm" => {
                if let Some(n) = finite_prefix(&op.operands) {
                    if n.len() >= 6 {
                        let m = Matrix::new(n[0], n[1], n[2], n[3], n[4], n[5]);
                        self.gs.current.concat_matrix(&m);
                    }
                }
            }
            "w" => {
                if let Some(n) = finite_prefix(&op.operands) {
                    if let Some(w) = n.first() {
                        self.style.line_width = w.abs();
                    }
                }
            }
            "J" | "j" | "M" | "d" | "gs" | "W" | "W*" => {}
            "m" => self.xy(&op.operands, Self::move_to),
            "l" => self.xy(&op.operands, Self::line_to),
            "h" => self.close_path(),
            "re" => {
                if let Some(n) = finite_prefix(&op.operands) {
                    if n.len() >= 4 {
                        self.rect_path(n[0], n[1], n[2], n[3]);
                    }
                }
            }
            "c" => self.curve_op(&op.operands, CurveKind::C),
            "v" => self.curve_op(&op.operands, CurveKind::V),
            "y" => self.curve_op(&op.operands, CurveKind::Y),
            "n" => self.path = Path::default(),
            "S" => self.paint(Paint::Stroke { close: false }),
            "s" => self.paint(Paint::Stroke { close: true }),
            "f" | "F" => self.paint(Paint::Fill { even_odd: false }),
            "f*" => self.paint(Paint::Fill { even_odd: true }),
            "B" => self.paint(Paint::FillStroke {
                even_odd: false,
                close: false,
            }),
            "B*" => self.paint(Paint::FillStroke {
                even_odd: true,
                close: false,
            }),
            "b" => self.paint(Paint::FillStroke {
                even_odd: false,
                close: true,
            }),
            "b*" => self.paint(Paint::FillStroke {
                even_odd: true,
                close: true,
            }),
            "g" => self.set_gray(&op.operands, false),
            "G" => self.set_gray(&op.operands, true),
            "rg" => self.set_rgb(&op.operands, false),
            "RG" => self.set_rgb(&op.operands, true),
            "k" => self.set_cmyk(&op.operands, false),
            "K" => self.set_cmyk(&op.operands, true),
            "cs" => self.set_space(&op.operands, false),
            "CS" => self.set_space(&op.operands, true),
            "sc" | "scn" | "SC" | "SCN" => {
                let stroke = op.operator.starts_with('S');
                self.set_sc(&op.operands, stroke);
            }
            "BT" => {
                self.gs.current.begin_text();
                self.in_text = true;
            }
            "ET" => self.in_text = false,
            "Td" => self.text_move(&op.operands, false),
            "TD" => self.text_move(&op.operands, true),
            "Tm" => {
                if self.in_text {
                    if let Some(n) = finite_prefix(&op.operands) {
                        if n.len() >= 6 {
                            self.gs.current.set_text_matrix(Matrix::new(
                                n[0], n[1], n[2], n[3], n[4], n[5],
                            ));
                        }
                    }
                }
            }
            "T*" => {
                if self.in_text {
                    self.op_tstar();
                }
            }
            "Tc" => self.set_text_num(&op.operands, TextNum::Char),
            "Tw" => self.set_text_num(&op.operands, TextNum::Word),
            "Tz" => self.set_text_num(&op.operands, TextNum::Scale),
            "TL" => self.set_text_num(&op.operands, TextNum::Leading),
            "Tr" => self.set_text_num(&op.operands, TextNum::Mode),
            "Tf" => self.set_font(&op.operands),
            "Tj" => {
                if self.in_text {
                    if let Some(bytes) = string_bytes(&op.operands) {
                        self.show_bytes(bytes);
                    }
                }
            }
            "TJ" => self.show_tj(&op.operands),
            "'" => {
                if self.in_text {
                    self.op_tstar();
                    if let Some(bytes) = string_bytes(&op.operands) {
                        self.show_bytes(bytes);
                    }
                }
            }
            "\"" => self.show_quote(&op.operands),
            "Do" => {
                if let Some(name) = op.operands.iter().find_map(|item| item.as_name()) {
                    let name = name.trim_matches('/').to_string();
                    self.op_do(doc, &name)?;
                } else {
                    self.omit();
                }
            }
            "BI" | "sh" => self.omit(),
            _ => {}
        }
        Ok(())
    }

    fn push_q(&mut self, q_depth: &mut u32) {
        *q_depth += 1;
        if *q_depth <= MAX_Q_DEPTH {
            self.gs.push();
            self.style_stack.push(self.style);
        }
    }

    fn pop_q(&mut self, q_depth: &mut u32) {
        if *q_depth == 0 {
            return;
        }
        if *q_depth <= MAX_Q_DEPTH {
            self.gs.pop();
            if let Some(style) = self.style_stack.pop() {
                self.style = style;
            }
        }
        *q_depth -= 1;
    }

    fn drain_q(&mut self, q_depth: &mut u32) {
        while *q_depth > 0 {
            if *q_depth <= MAX_Q_DEPTH {
                self.gs.pop();
                if let Some(style) = self.style_stack.pop() {
                    self.style = style;
                }
            }
            *q_depth -= 1;
        }
    }

    fn xy(&mut self, operands: &[PdfObject], draw: fn(&mut Self, f64, f64)) {
        if let Some(n) = finite_prefix(operands) {
            if n.len() >= 2 {
                draw(self, n[0], n[1]);
            }
        }
    }

    fn move_to(&mut self, x: f64, y: f64) {
        if self.path.blocked {
            return;
        }
        self.path.current_user = Some((x, y));
        self.path.start_user = Some((x, y));
        self.path.subpaths.push(Subpath::default());
        self.push_page_point(x, y);
    }

    fn line_to(&mut self, x: f64, y: f64) {
        if self.path.blocked || self.path.current_user.is_none() {
            return;
        }
        self.push_page_point(x, y);
        if !self.path.blocked {
            self.path.current_user = Some((x, y));
        }
    }

    fn close_path(&mut self) {
        if self.path.blocked {
            return;
        }
        let Some(start) = self.path.start_user else {
            return;
        };
        if let Some(sub) = self.path.subpaths.last_mut() {
            sub.closed = true;
        }
        self.path.current_user = Some(start);
    }

    fn rect_path(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.move_to(x, y);
        self.line_to(x + w, y);
        self.line_to(x + w, y + h);
        self.line_to(x, y + h);
        self.close_path();
    }

    fn push_page_point(&mut self, x: f64, y: f64) {
        if self.path.blocked {
            return;
        }
        if self.path.point_count >= MAX_PATH_POINTS {
            self.omit();
            self.path = Path {
                blocked: true,
                ..Path::default()
            };
            return;
        }
        let (px, py) = self.gs.current.ctm.transform_point(x, y);
        if !px.is_finite() || !py.is_finite() {
            return;
        }
        if let Some(sub) = self.path.subpaths.last_mut() {
            sub.points.push((px, py));
            self.path.point_count += 1;
        }
    }

    fn curve_op(&mut self, operands: &[PdfObject], kind: CurveKind) {
        let Some(n) = finite_prefix(operands) else {
            return;
        };
        let Some(current) = self.path.current_user else {
            return;
        };
        let points = match kind {
            CurveKind::C if n.len() >= 6 => Some(((n[0], n[1]), (n[2], n[3]), (n[4], n[5]))),
            CurveKind::V if n.len() >= 4 => Some((current, (n[0], n[1]), (n[2], n[3]))),
            CurveKind::Y if n.len() >= 4 => Some(((n[0], n[1]), (n[2], n[3]), (n[2], n[3]))),
            _ => None,
        };
        let Some((p1, p2, p3)) = points else {
            return;
        };
        let mut flat = Vec::new();
        flatten_cubic(current, p1, p2, p3, 3, &mut flat);
        for (x, y) in flat {
            self.line_to(x, y);
            if self.path.blocked {
                return;
            }
        }
    }

    fn paint(&mut self, kind: Paint) {
        let fill = self.style.fill;
        let stroke = self.style.stroke;
        match kind {
            Paint::Fill { even_odd } => self.fill_path(even_odd, fill),
            Paint::Stroke { close } => {
                if close {
                    self.close_path();
                }
                self.stroke_path(stroke);
            }
            Paint::FillStroke { even_odd, close } => {
                if close {
                    self.close_path();
                }
                self.fill_path(even_odd, fill);
                self.stroke_path(stroke);
            }
        }
        self.path = Path::default();
    }

    fn fill_path(&mut self, even_odd: bool, color: [u8; 3]) {
        if let Some(rect) = axis_rect(&self.path) {
            self.fill_rect(rect, color);
            return;
        }
        let Some(bounds) = path_bounds(&self.path) else {
            return;
        };
        let (x0, y0, x1, y1) =
            self.pixel_range(bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y, 0.0);
        for py in y0..y1 {
            for px in x0..x1 {
                let (page_x, page_y) = self.page_at(px, py);
                if self.inside(page_x, page_y, even_odd) {
                    self.set_pixel(px, py, color);
                }
            }
        }
    }

    fn fill_rect(&mut self, rect: Rect, color: [u8; 3]) {
        let (x0, y0, x1, y1) =
            self.pixel_range(rect.min_x, rect.min_y, rect.max_x, rect.max_y, 0.0);
        for py in y0..y1 {
            for px in x0..x1 {
                let (page_x, page_y) = self.page_at(px, py);
                if page_x >= rect.min_x
                    && page_x <= rect.max_x
                    && page_y >= rect.min_y
                    && page_y <= rect.max_y
                {
                    self.set_pixel(px, py, color);
                }
            }
        }
    }

    fn stroke_path(&mut self, color: [u8; 3]) {
        let half = self.page_line_width() * 0.5;
        let mut segs = Vec::new();
        for sub in &self.path.subpaths {
            if sub.points.len() < 2 {
                continue;
            }
            for pair in sub.points.windows(2) {
                segs.push((pair[0], pair[1]));
            }
            if sub.closed {
                let first = sub.points[0];
                if let Some(&last) = sub.points.last() {
                    if (first.0 - last.0).abs() > 1e-6 || (first.1 - last.1).abs() > 1e-6 {
                        segs.push((last, first));
                    }
                }
            }
        }
        for ((x1, y1), (x2, y2)) in segs {
            let min_x = x1.min(x2);
            let max_x = x1.max(x2);
            let min_y = y1.min(y2);
            let max_y = y1.max(y2);
            let (px0, py0, px1, py1) = self.pixel_range(min_x, min_y, max_x, max_y, half);
            for py in py0..py1 {
                for px in px0..px1 {
                    let (page_x, page_y) = self.page_at(px, py);
                    if dist_to_segment(page_x, page_y, x1, y1, x2, y2) <= half {
                        self.set_pixel(px, py, color);
                    }
                }
            }
        }
    }

    fn page_line_width(&self) -> f64 {
        let ctm = self.gs.current.ctm;
        let scaled = self.style.line_width.abs() * ctm.a.hypot(ctm.b);
        scaled.max(0.5 / self.scale)
    }

    fn inside(&self, x: f64, y: f64, even_odd: bool) -> bool {
        let mut wind = 0i32;
        for sub in &self.path.subpaths {
            let pts = &sub.points;
            let n = pts.len();
            if n < 2 {
                continue;
            }
            for i in 0..n {
                let (x1, y1) = pts[i];
                let (x2, y2) = pts[(i + 1) % n];
                if (y1 > y) == (y2 > y) {
                    continue;
                }
                let denom = y2 - y1;
                if denom.abs() < 1e-18 {
                    continue;
                }
                let xint = x1 + (y - y1) * (x2 - x1) / denom;
                if xint > x {
                    if even_odd || y1 < y2 {
                        wind += 1;
                    } else {
                        wind -= 1;
                    }
                }
            }
        }
        if even_odd {
            wind % 2 != 0
        } else {
            wind != 0
        }
    }

    fn set_pixel(&mut self, x: i32, y: i32, color: [u8; 3]) {
        if x < 0 || y < 0 || (x as u32) >= self.width || (y as u32) >= self.height {
            return;
        }
        let (page_x, page_y) = self.page_at(x, y);
        if !self.clip_allows(page_x, page_y) {
            return;
        }
        let i = ((y as usize) * (self.width as usize) + (x as usize)) * 3;
        self.pixels[i] = color[0];
        self.pixels[i + 1] = color[1];
        self.pixels[i + 2] = color[2];
    }

    fn page_at(&self, x: i32, y: i32) -> (f64, f64) {
        let page_x = self.crop.min_x + (x as f64 + 0.5) / self.scale;
        let page_y = self.crop.max_y - (y as f64 + 0.5) / self.scale;
        (page_x, page_y)
    }

    fn clip_allows(&self, x: f64, y: f64) -> bool {
        match self.style.clip {
            None => true,
            Some(rect) => {
                x >= rect.min_x && x <= rect.max_x && y >= rect.min_y && y <= rect.max_y
            }
        }
    }

    fn pixel_range(&self, min_x: f64, min_y: f64, max_x: f64, max_y: f64, pad: f64) -> (i32, i32, i32, i32) {
        let x0 = ((min_x - pad - self.crop.min_x) * self.scale).floor() as i32;
        let x1 = ((max_x + pad - self.crop.min_x) * self.scale).ceil() as i32;
        let y0 = ((self.crop.max_y - (max_y + pad)) * self.scale).floor() as i32;
        let y1 = ((self.crop.max_y - (min_y - pad)) * self.scale).ceil() as i32;
        (
            x0.max(0),
            y0.max(0),
            x1.min(self.width as i32),
            y1.min(self.height as i32),
        )
    }

    fn set_gray(&mut self, operands: &[PdfObject], stroke: bool) {
        let Some(n) = finite_prefix(operands) else {
            return;
        };
        let Some(&g) = n.first() else {
            return;
        };
        self.store_color(gray_bytes(g), 1, stroke);
    }

    fn set_rgb(&mut self, operands: &[PdfObject], stroke: bool) {
        let Some(n) = finite_prefix(operands) else {
            return;
        };
        if n.len() < 3 {
            return;
        }
        self.store_color(rgb_bytes(n[0], n[1], n[2]), 3, stroke);
    }

    fn set_cmyk(&mut self, operands: &[PdfObject], stroke: bool) {
        let Some(n) = finite_prefix(operands) else {
            return;
        };
        if n.len() < 4 {
            return;
        }
        self.store_color(cmyk_bytes(n[0], n[1], n[2], n[3]), 4, stroke);
    }

    fn store_color(&mut self, color: [u8; 3], n: u8, stroke: bool) {
        if stroke {
            self.style.stroke = color;
            self.style.stroke_n = n;
        } else {
            self.style.fill = color;
            self.style.fill_n = n;
        }
    }

    fn set_space(&mut self, operands: &[PdfObject], stroke: bool) {
        let Some(name) = operands.iter().find_map(|item| item.as_name()) else {
            self.omit();
            return;
        };
        let n = match name.trim_matches('/') {
            "DeviceGray" | "G" => 1,
            "DeviceRGB" | "RGB" => 3,
            "DeviceCMYK" | "CMYK" => 4,
            _ => {
                self.omit();
                return;
            }
        };
        if stroke {
            self.style.stroke_n = n;
        } else {
            self.style.fill_n = n;
        }
    }

    fn set_sc(&mut self, operands: &[PdfObject], stroke: bool) {
        if operands.iter().any(|item| matches!(item, PdfObject::Name(_))) {
            self.omit();
            return;
        }
        let Some(n) = finite_prefix(operands) else {
            return;
        };
        let comps = if stroke {
            self.style.stroke_n
        } else {
            self.style.fill_n
        };
        if n.len() < comps as usize {
            return;
        }
        let color = match comps {
            1 => gray_bytes(n[0]),
            3 => rgb_bytes(n[0], n[1], n[2]),
            4 => cmyk_bytes(n[0], n[1], n[2], n[3]),
            _ => {
                self.omit();
                return;
            }
        };
        if stroke {
            self.style.stroke = color;
        } else {
            self.style.fill = color;
        }
    }

    fn text_move(&mut self, operands: &[PdfObject], set_leading: bool) {
        if !self.in_text {
            return;
        }
        let Some(n) = finite_prefix(operands) else {
            return;
        };
        if n.len() < 2 {
            return;
        }
        if set_leading {
            self.gs.current.text_state.leading = -n[1];
        }
        self.gs.current.move_text_position(n[0], n[1]);
    }

    fn op_tstar(&mut self) {
        let leading = self.gs.current.text_state.leading;
        self.gs.current.move_text_position(0.0, -leading);
    }

    fn set_text_num(&mut self, operands: &[PdfObject], which: TextNum) {
        if !self.in_text {
            return;
        }
        let Some(n) = finite_prefix(operands) else {
            return;
        };
        let Some(&v) = n.first() else {
            return;
        };
        let text = &mut self.gs.current.text_state;
        match which {
            TextNum::Char => text.char_spacing = v,
            TextNum::Word => text.word_spacing = v,
            TextNum::Scale => text.horizontal_scaling = v,
            TextNum::Leading => text.leading = v,
            TextNum::Mode => text.rendering_mode = v.round() as i64,
        }
    }

    fn set_font(&mut self, operands: &[PdfObject]) {
        if !self.in_text {
            return;
        }
        let name = operands
            .iter()
            .find_map(|item| item.as_name())
            .unwrap_or("")
            .trim_matches('/')
            .to_string();
        let Some(size) = operands.iter().find_map(|item| item.as_f64()) else {
            return;
        };
        if !size.is_finite() {
            return;
        }
        self.gs.current.text_state.font_name = name;
        self.gs.current.text_state.font_size = size;
    }

    fn show_quote(&mut self, operands: &[PdfObject]) {
        if !self.in_text {
            return;
        }
        let Some(n) = finite_prefix(operands) else {
            return;
        };
        if n.len() >= 2 {
            self.gs.current.text_state.word_spacing = n[0];
            self.gs.current.text_state.char_spacing = n[1];
        }
        self.op_tstar();
        if let Some(bytes) = string_bytes(operands) {
            self.show_bytes(bytes);
        }
    }

    fn show_tj(&mut self, operands: &[PdfObject]) {
        if !self.in_text {
            return;
        }
        let Some(items) = operands.iter().find_map(|item| match item {
            PdfObject::Array(values) => Some(values),
            _ => None,
        }) else {
            return;
        };
        for item in items {
            match item {
                PdfObject::String(text) => self.show_bytes(&text.bytes),
                PdfObject::Integer(k) => {
                    let state = self.gs.current.text_state.clone();
                    let delta = FontMetrics::compute_kerning_displacement(*k as f64, &state);
                    self.gs.current.advance_text(delta);
                }
                PdfObject::Real(k) if k.is_finite() => {
                    let state = self.gs.current.text_state.clone();
                    let delta = FontMetrics::compute_kerning_displacement(*k, &state);
                    self.gs.current.advance_text(delta);
                }
                _ => {}
            }
        }
    }

    fn show_bytes(&mut self, bytes: &[u8]) {
        if !self.in_text {
            return;
        }
        let text_state = self.gs.current.text_state.clone();
        let font = self.fonts.get(&text_state.font_name).cloned();
        let composite = font.as_ref().is_some_and(|item| item.is_composite);
        let mode = text_state.rendering_mode;
        let color = match mode {
            1 | 5 => self.style.stroke,
            _ => self.style.fill,
        };
        let paint = !matches!(mode, 3 | 7);
        if composite {
            let mut index = 0;
            while index + 1 < bytes.len() {
                let code = ((bytes[index] as u32) << 8) | (bytes[index + 1] as u32);
                self.paint_code(
                    &text_state,
                    font.as_ref(),
                    code,
                    bytes[index + 1],
                    true,
                    paint,
                    color,
                );
                index += 2;
            }
        } else {
            for &byte in bytes {
                self.paint_code(
                    &text_state,
                    font.as_ref(),
                    byte as u32,
                    byte,
                    false,
                    paint,
                    color,
                );
            }
        }
    }

    fn paint_code(
        &mut self,
        text_state: &crate::stream::TextState,
        font: Option<&ResolvedFont>,
        code: u32,
        byte: u8,
        composite: bool,
        paint: bool,
        color: [u8; 3],
    ) {
        let tm = self.gs.current.text_matrix;
        let ctm = self.gs.current.ctm;
        let (origin_x, origin_y) = ctm.transform_point(tm.e, tm.f);
        let trm = self.gs.current.text_rendering_matrix();
        let rendered = trm.c.hypot(trm.d);
        let text_advance = if let Some(font) = font {
            font.metrics.compute_char_advance(code, text_state)
        } else if composite {
            text_state.font_size
        } else {
            0.5 * text_state.font_size
        };
        let page_advance = text_advance * (tm.a * ctm.a + tm.b * ctm.c);
        let ch = scalar_for(font, code, byte, composite);
        let skip = !paint
            || code == 32
            || ch == ' '
            || text_state.font_size.abs() < 1e-9
            || rendered < 1e-6
            || page_advance.abs() < 1e-9;
        if !skip {
            self.stamp_glyph(origin_x, origin_y, page_advance, rendered, ch, color);
        }
        self.gs.current.advance_text(text_advance);
    }

    fn stamp_glyph(
        &mut self,
        origin_x: f64,
        origin_y: f64,
        page_advance: f64,
        rendered: f64,
        ch: char,
        color: [u8; 3],
    ) {
        let x_sign = if page_advance < 0.0 { -1.0 } else { 1.0 };
        let cell_w = page_advance.abs() * 0.82;
        let cell_h = rendered;
        if cell_w < 1e-6 || cell_h < 1e-6 {
            return;
        }
        let rows = glyph_rows(ch);
        let far_x = origin_x + x_sign * cell_w;
        let top = origin_y + cell_h;
        let (x0, y0, x1, y1) = self.pixel_range(
            origin_x.min(far_x),
            origin_y,
            origin_x.max(far_x),
            top,
            0.0,
        );
        for py in y0..y1 {
            for px in x0..x1 {
                let (page_x, page_y) = self.page_at(px, py);
                let u = (page_x - origin_x) * x_sign / cell_w;
                let v = (page_y - origin_y) / cell_h;
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                let col = ((u * 5.0).floor() as i32).clamp(0, 4) as u32;
                let row = 6 - (v * 7.0).floor() as i32;
                if !(0..=6).contains(&row) {
                    continue;
                }
                if rows[row as usize] & (0x10 >> col) != 0 {
                    self.set_pixel(px, py, color);
                }
            }
        }
    }

    fn op_do(&mut self, doc: &mut PdfDocument, name: &str) -> PdfResult<()> {
        let Some(xobj_value) = self.resources.get("XObject").cloned() else {
            self.omit();
            return Ok(());
        };
        let Some(xobjects) = lookup_dict(doc, Some(xobj_value))? else {
            self.omit();
            return Ok(());
        };
        let Some(value) = xobjects.get(name).cloned() else {
            self.omit();
            return Ok(());
        };
        let Some(stream) = lookup_stream(doc, Some(value))? else {
            self.omit();
            return Ok(());
        };
        let subtype = stream
            .dict
            .get("Subtype")
            .and_then(|item| item.as_name())
            .unwrap_or("")
            .trim_matches('/')
            .to_string();
        match subtype.as_str() {
            "Image" => match decode_image(doc, &stream)? {
                Some(image) => {
                    if !self.paint_image(&image) {
                        self.omit();
                    }
                }
                None => self.omit(),
            },
            "Form" => self.paint_form(doc, &stream)?,
            _ => self.omit(),
        }
        Ok(())
    }

    fn paint_image(&mut self, image: &RgbImage) -> bool {
        let ctm = self.gs.current.ctm;
        let Some(inverse) = ctm.inverse() else {
            return false;
        };
        let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (u, v) in corners {
            let (x, y) = ctm.transform_point(u, v);
            if !x.is_finite() || !y.is_finite() {
                return false;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        let (x0, y0, x1, y1) = self.pixel_range(min_x, min_y, max_x, max_y, 0.0);
        let iw = image.width.max(1);
        let ih = image.height.max(1);
        for py in y0..y1 {
            for px in x0..x1 {
                let (page_x, page_y) = self.page_at(px, py);
                let (u, v) = inverse.transform_point(page_x, page_y);
                if u < -1e-6 || v < -1e-6 || u > 1.0 + 1e-6 || v > 1.0 + 1e-6 {
                    continue;
                }
                let u = u.clamp(0.0, 1.0);
                let v = v.clamp(0.0, 1.0);
                let ix = (u * (iw as f64 - 1.0)).round() as u32;
                let iy = ((1.0 - v) * (ih as f64 - 1.0)).round() as u32;
                let ix = ix.min(iw - 1);
                let iy = iy.min(ih - 1);
                let src = ((iy as usize) * (iw as usize) + (ix as usize)) * 3;
                if src + 2 < image.rgb.len() {
                    let color = [image.rgb[src], image.rgb[src + 1], image.rgb[src + 2]];
                    self.set_pixel(px, py, color);
                }
            }
        }
        true
    }

    fn paint_form(&mut self, doc: &mut PdfDocument, stream: &PdfStream) -> PdfResult<()> {
        if self.form_depth >= MAX_FORM_DEPTH {
            self.omit();
            return Ok(());
        }
        self.form_depth += 1;
        let result = self.paint_form_inner(doc, stream);
        self.form_depth -= 1;
        result
    }

    fn paint_form_inner(&mut self, doc: &mut PdfDocument, stream: &PdfStream) -> PdfResult<()> {
        let Some(job) = prepare_form(doc, stream)? else {
            self.omit();
            return Ok(());
        };
        self.gs.push();
        self.style_stack.push(self.style);
        let saved_path = std::mem::take(&mut self.path);
        let saved_text = self.in_text;
        self.in_text = false;
        let saved_fonts = job.fonts.as_ref().map(|fonts| {
            std::mem::replace(&mut self.fonts, fonts.clone())
        });
        let saved_resources = job.resources.as_ref().map(|resources| {
            std::mem::replace(&mut self.resources, resources.clone())
        });
        self.gs.current.concat_matrix(&job.matrix);
        if let Some(bbox) = job.bbox {
            self.tighten_clip(bbox);
        }
        let result = self.interpret_ops(doc, &job.ops);
        if let Some(fonts) = saved_fonts {
            self.fonts = fonts;
        }
        if let Some(resources) = saved_resources {
            self.resources = resources;
        }
        self.path = saved_path;
        self.in_text = saved_text;
        self.gs.pop();
        if let Some(style) = self.style_stack.pop() {
            self.style = style;
        }
        result
    }

    fn tighten_clip(&mut self, bbox: [f64; 4]) {
        let corners = [
            (bbox[0], bbox[1]),
            (bbox[2], bbox[1]),
            (bbox[2], bbox[3]),
            (bbox[0], bbox[3]),
        ];
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut any = false;
        for (x, y) in corners {
            let (px, py) = self.gs.current.ctm.transform_point(x, y);
            if px.is_finite() && py.is_finite() {
                any = true;
                min_x = min_x.min(px);
                min_y = min_y.min(py);
                max_x = max_x.max(px);
                max_y = max_y.max(py);
            }
        }
        if !any {
            return;
        }
        let mapped = Rect {
            min_x,
            min_y,
            max_x,
            max_y,
        };
        self.style.clip = Some(intersect_clip(self.style.clip, mapped));
    }
}

enum CurveKind {
    C,
    V,
    Y,
}

enum Paint {
    Fill { even_odd: bool },
    Stroke { close: bool },
    FillStroke { even_odd: bool, close: bool },
}

enum TextNum {
    Char,
    Word,
    Scale,
    Leading,
    Mode,
}

fn finite_prefix(operands: &[PdfObject]) -> Option<Vec<f64>> {
    let mut out = Vec::new();
    for operand in operands {
        match operand {
            PdfObject::Integer(value) => out.push(*value as f64),
            PdfObject::Real(value) => {
                if !value.is_finite() {
                    return None;
                }
                out.push(*value);
            }
            _ => break,
        }
    }
    Some(out)
}

fn string_bytes(operands: &[PdfObject]) -> Option<&[u8]> {
    operands.iter().find_map(|item| match item {
        PdfObject::String(text) => Some(text.bytes.as_slice()),
        _ => None,
    })
}

fn flatten_cubic(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    depth: u8,
    out: &mut Vec<(f64, f64)>,
) {
    if depth == 0 {
        out.push(p3);
        return;
    }
    let mid = |a: (f64, f64), b: (f64, f64)| ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let p01 = mid(p0, p1);
    let p12 = mid(p1, p2);
    let p23 = mid(p2, p3);
    let p012 = mid(p01, p12);
    let p123 = mid(p12, p23);
    let p0123 = mid(p012, p123);
    flatten_cubic(p0, p01, p012, p0123, depth - 1, out);
    flatten_cubic(p0123, p123, p23, p3, depth - 1, out);
}

fn axis_rect(path: &Path) -> Option<Rect> {
    if path.subpaths.len() != 1 {
        return None;
    }
    let pts = &path.subpaths[0].points;
    if pts.len() != 4 {
        return None;
    }
    let xs = [pts[0].0, pts[1].0, pts[2].0, pts[3].0];
    let ys = [pts[0].1, pts[1].1, pts[2].1, pts[3].1];
    let (min_x, max_x) = unique_pair(xs)?;
    let (min_y, max_y) = unique_pair(ys)?;
    Some(Rect {
        min_x,
        min_y,
        max_x,
        max_y,
    })
}

fn unique_pair(values: [f64; 4]) -> Option<(f64, f64)> {
    let mut first: Option<f64> = None;
    let mut second: Option<f64> = None;
    for value in values {
        if let Some(known) = first {
            if (value - known).abs() < 1e-6 {
                continue;
            }
        } else {
            first = Some(value);
            continue;
        }
        if let Some(known) = second {
            if (value - known).abs() < 1e-6 {
                continue;
            }
            return None;
        }
        second = Some(value);
    }
    match (first, second) {
        (Some(a), Some(b)) => Some((a.min(b), a.max(b))),
        _ => None,
    }
}

fn path_bounds(path: &Path) -> Option<Rect> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    for sub in &path.subpaths {
        for (x, y) in &sub.points {
            any = true;
            min_x = min_x.min(*x);
            min_y = min_y.min(*y);
            max_x = max_x.max(*x);
            max_y = max_y.max(*y);
        }
    }
    if !any {
        return None;
    }
    Some(Rect {
        min_x,
        min_y,
        max_x,
        max_y,
    })
}

fn dist_to_segment(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-18 {
        return (px - x1).hypot(py - y1);
    }
    let t = (((px - x1) * dx + (py - y1) * dy) / len2).clamp(0.0, 1.0);
    (px - (x1 + t * dx)).hypot(py - (y1 + t * dy))
}

fn intersect_clip(current: Option<Rect>, incoming: Rect) -> Rect {
    let Some(current) = current else {
        return incoming;
    };
    if current.min_x > current.max_x || current.min_y > current.max_y {
        return current;
    }
    let min_x = current.min_x.max(incoming.min_x);
    let min_y = current.min_y.max(incoming.min_y);
    let max_x = current.max_x.min(incoming.max_x);
    let max_y = current.max_y.min(incoming.max_y);
    if min_x > max_x || min_y > max_y {
        Rect {
            min_x: 1.0,
            min_y: 1.0,
            max_x: 0.0,
            max_y: 0.0,
        }
    } else {
        Rect {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }
}

fn scalar_for(font: Option<&ResolvedFont>, code: u32, byte: u8, composite: bool) -> char {
    if let Some(font) = font {
        if let Some(cmap) = font.cmap.as_ref() {
            if !cmap.char_to_unicode.is_empty() {
                if let Some(text) = cmap.decode_code(code) {
                    return text.chars().next().unwrap_or('\u{FFFD}');
                }
                if composite {
                    return char::from_u32(code).unwrap_or('\u{FFFD}');
                }
                return byte as char;
            }
        }
        if composite {
            return char::from_u32(code).unwrap_or('\u{FFFD}');
        }
        return WinAnsiEncoding::code_to_unicode(byte);
    }
    byte as char
}

fn inherited_resources(doc: &mut PdfDocument, page_id: ObjectId) -> PdfResult<PdfDictionary> {
    let mut current = page_id;
    let mut seen = HashSet::new();
    for _ in 0..MAX_PARENT_HOPS {
        if !seen.insert(current) {
            break;
        }
        let dict = match doc.get_object(current) {
            Ok(PdfObject::Dictionary(dict)) => dict,
            Ok(_) => break,
            Err(PdfError::ObjectNotFound { .. }) => break,
            Err(error) => return Err(error),
        };
        if let Some(resources) = lookup_dict(doc, dict.get("Resources").cloned())? {
            return Ok(resources);
        }
        match dict.get("Parent").and_then(|parent| parent.as_reference()) {
            Some(parent) => current = parent,
            None => break,
        }
    }
    Ok(PdfDictionary::new())
}

fn lookup_dict(
    doc: &mut PdfDocument,
    value: Option<PdfObject>,
) -> PdfResult<Option<PdfDictionary>> {
    match value {
        Some(PdfObject::Dictionary(dict)) => Ok(Some(dict)),
        Some(PdfObject::Reference(id)) => match doc.get_object(id) {
            Ok(PdfObject::Dictionary(dict)) => Ok(Some(dict)),
            Ok(_) => Ok(None),
            Err(PdfError::ObjectNotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        },
        _ => Ok(None),
    }
}

fn lookup_stream(doc: &mut PdfDocument, value: Option<PdfObject>) -> PdfResult<Option<PdfStream>> {
    match value {
        Some(PdfObject::Stream(stream)) => Ok(Some(stream)),
        Some(PdfObject::Reference(id)) => match doc.get_object(id) {
            Ok(PdfObject::Stream(stream)) => Ok(Some(stream)),
            Ok(_) => Ok(None),
            Err(PdfError::ObjectNotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        },
        _ => Ok(None),
    }
}

fn single_filter(dict: &PdfDictionary) -> Option<String> {
    match dict.get("Filter") {
        None => Some(String::new()),
        Some(PdfObject::Name(name)) => Some(name.as_str().trim_matches('/').to_string()),
        Some(PdfObject::Array(items)) if items.len() == 1 => items
            .first()
            .and_then(|item| item.as_name())
            .map(|name| name.trim_matches('/').to_string()),
        _ => None,
    }
}

fn decode_params(doc: &mut PdfDocument, dict: &PdfDictionary) -> PdfResult<Option<PdfDictionary>> {
    lookup_dict(doc, dict.get("DecodeParms").cloned())
}

fn prepare_form(doc: &mut PdfDocument, stream: &PdfStream) -> PdfResult<Option<FormJob>> {
    let Some(filter) = single_filter(&stream.dict) else {
        return Ok(None);
    };
    let limits = doc.limits.clone();
    let params = decode_params(doc, &stream.dict)?;
    let decoded = match decode_stream(&filter, params.as_ref(), &stream.content, &limits) {
        Ok(bytes) => bytes,
        Err(PdfError::SecurityLimitExceeded(message)) => {
            return Err(PdfError::SecurityLimitExceeded(message));
        }
        Err(_) => return Ok(None),
    };
    let ops = match ContentStreamTokenizer::new(&decoded).tokenize_all() {
        Ok(ops) => ops,
        Err(PdfError::SecurityLimitExceeded(message)) => {
            return Err(PdfError::SecurityLimitExceeded(message));
        }
        Err(_) => return Ok(None),
    };
    let matrix = match stream.dict.get("Matrix").and_then(|item| match item {
        PdfObject::Array(values) => Some(values.clone()),
        _ => None,
    }) {
        Some(values) => matrix_from(&values).unwrap_or_else(Matrix::identity),
        None => Matrix::identity(),
    };
    let bbox = stream
        .dict
        .get("BBox")
        .and_then(|item| match item {
            PdfObject::Array(values) => Some(values.clone()),
            _ => None,
        })
        .and_then(|values| four_numbers(&values));
    let (resources, fonts) = if stream.dict.get("Resources").is_some() {
        match lookup_dict(doc, stream.dict.get("Resources").cloned())? {
            Some(resources) => {
                let fonts = form_fonts(doc, &resources)?;
                (Some(resources), Some(fonts))
            }
            None => (None, None),
        }
    } else {
        (None, None)
    };
    Ok(Some(FormJob {
        ops,
        resources,
        fonts,
        matrix,
        bbox,
    }))
}

fn matrix_from(values: &[PdfObject]) -> Option<Matrix> {
    if values.len() < 6 {
        return None;
    }
    let mut n = [0.0; 6];
    for (slot, value) in n.iter_mut().zip(values.iter()) {
        let number = value.as_f64()?;
        if !number.is_finite() {
            return None;
        }
        *slot = number;
    }
    Some(Matrix::new(n[0], n[1], n[2], n[3], n[4], n[5]))
}

fn four_numbers(values: &[PdfObject]) -> Option<[f64; 4]> {
    if values.len() < 4 {
        return None;
    }
    let mut n = [0.0; 4];
    for (slot, value) in n.iter_mut().zip(values.iter()) {
        let number = value.as_f64()?;
        if !number.is_finite() {
            return None;
        }
        *slot = number;
    }
    Some(n)
}

fn form_fonts(
    doc: &mut PdfDocument,
    resources: &PdfDictionary,
) -> PdfResult<BTreeMap<String, ResolvedFont>> {
    let mut fonts = BTreeMap::new();
    let Some(value) = resources.get("Font").cloned() else {
        return Ok(fonts);
    };
    let Some(dict) = lookup_dict(doc, Some(value))? else {
        return Ok(fonts);
    };
    let entries: Vec<(String, PdfObject)> = dict
        .0
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().trim_start_matches('/').to_string(),
                value.clone(),
            )
        })
        .collect();
    for (name, value) in entries {
        if name.is_empty() {
            continue;
        }
        match resolve_one_font(doc, value) {
            Ok(Some(font)) => {
                fonts.insert(name, font);
            }
            Ok(None) => {}
            Err(PdfError::SecurityLimitExceeded(message)) => {
                return Err(PdfError::SecurityLimitExceeded(message));
            }
            Err(_) => {}
        }
    }
    Ok(fonts)
}

fn resolve_one_font(doc: &mut PdfDocument, value: PdfObject) -> PdfResult<Option<ResolvedFont>> {
    let Some(font) = lookup_dict(doc, Some(value))? else {
        return Ok(None);
    };
    let subtype = font
        .get("Subtype")
        .and_then(|item| item.as_name())
        .unwrap_or("")
        .trim_matches('/')
        .to_string();
    if subtype == "Type0" {
        return Ok(Some(composite_form_font(doc, &font)?));
    }
    if matches!(
        subtype.as_str(),
        "Type1" | "TrueType" | "Type3" | "MMType1" | ""
    ) {
        return Ok(Some(simple_form_font(doc, &font)?));
    }
    Ok(None)
}

fn simple_form_font(doc: &mut PdfDocument, font: &PdfDictionary) -> PdfResult<ResolvedFont> {
    let descriptor = lookup_dict(doc, font.get("FontDescriptor").cloned())?;
    let missing = descriptor
        .as_ref()
        .and_then(|dict| dict.get("MissingWidth"))
        .and_then(|item| item.as_f64())
        .filter(|value| value.is_finite())
        .unwrap_or(500.0);
    let first = font
        .get("FirstChar")
        .and_then(|item| item.as_f64())
        .map(|value| value as u32)
        .unwrap_or(0);
    let last = font
        .get("LastChar")
        .and_then(|item| item.as_f64())
        .map(|value| value as u32)
        .unwrap_or(255);
    let widths = read_widths(doc, font.get("Widths").cloned())?;
    let metrics = FontMetrics::new(first, last, widths, missing);
    let cmap = read_tounicode(doc, font)?;
    Ok(ResolvedFont {
        metrics,
        cmap,
        family: String::new(),
        weight: 400,
        style: "normal".to_string(),
        is_composite: false,
    })
}

fn composite_form_font(doc: &mut PdfDocument, font: &PdfDictionary) -> PdfResult<ResolvedFont> {
    let mut metrics = FontMetrics::default();
    if let Some(descendants) = owned_array(doc, font.get("DescendantFonts").cloned())? {
        if let Some(first) = descendants.first() {
            if let Some(desc) = lookup_dict(doc, Some(first.clone()))? {
                if let Some(dw) = desc.get("DW").and_then(|item| item.as_f64()) {
                    if dw.is_finite() {
                        metrics.default_width = dw;
                    }
                }
            }
        }
    }
    let cmap = read_tounicode(doc, font)?;
    Ok(ResolvedFont {
        metrics,
        cmap,
        family: String::new(),
        weight: 400,
        style: "normal".to_string(),
        is_composite: true,
    })
}

fn read_widths(doc: &mut PdfDocument, value: Option<PdfObject>) -> PdfResult<Vec<f64>> {
    let Some(items) = owned_array(doc, value)? else {
        return Ok(Vec::new());
    };
    let mut widths = Vec::new();
    for item in items.into_iter().take(MAX_FORM_WIDTHS) {
        if let Some(value) = item.as_f64() {
            if value.is_finite() {
                widths.push(value);
            }
        }
    }
    Ok(widths)
}

fn owned_array(
    doc: &mut PdfDocument,
    value: Option<PdfObject>,
) -> PdfResult<Option<Vec<PdfObject>>> {
    match value {
        Some(PdfObject::Array(items)) => Ok(Some(items)),
        Some(PdfObject::Reference(id)) => match doc.get_object(id) {
            Ok(PdfObject::Array(items)) => Ok(Some(items)),
            Ok(_) => Ok(None),
            Err(PdfError::ObjectNotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        },
        _ => Ok(None),
    }
}

fn read_tounicode(
    doc: &mut PdfDocument,
    font: &PdfDictionary,
) -> PdfResult<Option<ToUnicodeMap>> {
    let Some(value) = font.get("ToUnicode").cloned() else {
        return Ok(None);
    };
    let Some(stream) = lookup_stream(doc, Some(value))? else {
        return Ok(None);
    };
    let Some(filter) = single_filter(&stream.dict) else {
        return Ok(None);
    };
    let limits = doc.limits.clone();
    let params = decode_params(doc, &stream.dict)?;
    let bytes = match decode_stream(&filter, params.as_ref(), &stream.content, &limits) {
        Ok(bytes) => bytes,
        Err(PdfError::SecurityLimitExceeded(message)) => {
            return Err(PdfError::SecurityLimitExceeded(message));
        }
        Err(_) => return Ok(None),
    };
    match ToUnicodeMap::parse(&bytes) {
        Ok(map) => Ok(Some(map)),
        Err(PdfError::SecurityLimitExceeded(message)) => Err(PdfError::SecurityLimitExceeded(message)),
        Err(_) => Ok(None),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorKind {
    Gray,
    Rgb,
    Cmyk,
    Guess,
    Unsupported,
}

fn classify_colorspace(doc: &mut PdfDocument, value: Option<PdfObject>) -> PdfResult<ColorKind> {
    match value {
        None => Ok(ColorKind::Guess),
        Some(PdfObject::Name(name)) => Ok(name_kind(name.as_str())),
        Some(PdfObject::Array(items)) => {
            if items.len() == 1 {
                if let Some(name) = items.first().and_then(|item| item.as_name()) {
                    return Ok(name_kind(name));
                }
            }
            Ok(ColorKind::Unsupported)
        }
        Some(PdfObject::Reference(id)) => match doc.get_object(id) {
            Ok(inner) => classify_colorspace(doc, Some(inner)),
            Err(PdfError::ObjectNotFound { .. }) => Ok(ColorKind::Unsupported),
            Err(error) => Err(error),
        },
        Some(_) => Ok(ColorKind::Unsupported),
    }
}

fn name_kind(name: &str) -> ColorKind {
    match name.trim_matches('/') {
        "DeviceGray" | "G" => ColorKind::Gray,
        "DeviceRGB" | "RGB" => ColorKind::Rgb,
        "DeviceCMYK" | "CMYK" => ColorKind::Cmyk,
        _ => ColorKind::Unsupported,
    }
}

fn image_side(value: Option<f64>) -> Option<u32> {
    let value = value?;
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let rounded = value.round();
    if rounded < 1.0 || rounded > MAX_RASTER_SIDE as f64 {
        return None;
    }
    Some(rounded as u32)
}

fn decode_image(doc: &mut PdfDocument, stream: &PdfStream) -> PdfResult<Option<RgbImage>> {
    let width = match image_side(stream.dict.get("Width").and_then(|item| item.as_f64())) {
        Some(width) => width,
        None => return Ok(None),
    };
    let height = match image_side(stream.dict.get("Height").and_then(|item| item.as_f64())) {
        Some(height) => height,
        None => return Ok(None),
    };
    let samples = (width as u64).saturating_mul(height as u64);
    if samples > MAX_RASTER_PIXELS {
        return Ok(None);
    }
    let bpc = stream
        .dict
        .get("BitsPerComponent")
        .and_then(|item| item.as_f64())
        .unwrap_or(8.0);
    if bpc != 1.0 && bpc != 8.0 {
        return Ok(None);
    }
    let kind = classify_colorspace(doc, stream.dict.get("ColorSpace").cloned())?;
    if kind == ColorKind::Unsupported {
        return Ok(None);
    }
    if bpc == 1.0 && !matches!(kind, ColorKind::Gray | ColorKind::Guess) {
        return Ok(None);
    }
    let Some(filter) = single_filter(&stream.dict) else {
        return Ok(None);
    };
    if filter == "DCTDecode" || filter == "DCT" {
        return decode_jpeg_image(&stream.content, width, height, kind);
    }
    if !matches!(
        filter.as_str(),
        "" | "Identity" | "FlateDecode" | "Fl" | "ASCIIHexDecode" | "AHx" | "ASCII85Decode" | "A85"
    ) {
        return Ok(None);
    }
    let limits = doc.limits.clone();
    let params = decode_params(doc, &stream.dict)?;
    let raw = match decode_stream(&filter, params.as_ref(), &stream.content, &limits) {
        Ok(bytes) => bytes,
        Err(PdfError::SecurityLimitExceeded(message)) => {
            return Err(PdfError::SecurityLimitExceeded(message));
        }
        Err(_) => return Ok(None),
    };
    let decode = finite_decode(stream.dict.get("Decode"));
    Ok(pack_rgb(width, height, bpc as u8, kind, &raw, decode.as_deref()))
}

fn finite_decode(value: Option<&PdfObject>) -> Option<Vec<f64>> {
    let PdfObject::Array(items) = value? else {
        return None;
    };
    let mut numbers = Vec::new();
    for item in items {
        let number = item.as_f64()?;
        if !number.is_finite() {
            return None;
        }
        numbers.push(number);
    }
    Some(numbers)
}

fn decode_jpeg_image(
    data: &[u8],
    width: u32,
    height: u32,
    kind: ColorKind,
) -> PdfResult<Option<RgbImage>> {
    let header = match parse_jpeg(data) {
        Ok(header) => header,
        Err(_) => return Ok(None),
    };
    if header.width != width || header.height != height {
        return Ok(None);
    }
    if header.width > MAX_RASTER_SIDE || header.height > MAX_RASTER_SIDE {
        return Ok(None);
    }
    let components = header.components as u64;
    let estimate = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(components.max(1));
    if estimate > MAX_RASTER_PIXELS {
        return Ok(None);
    }
    if matches!(kind, ColorKind::Rgb) && header.components == 4 {
        return Ok(None);
    }
    if matches!(kind, ColorKind::Cmyk) && header.components == 3 {
        return Ok(None);
    }
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(data));
    let pixels = match decoder.decode() {
        Ok(pixels) => pixels,
        Err(_) => return Ok(None),
    };
    let Some(info) = decoder.info() else {
        return Ok(None);
    };
    let rgb = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => {
            if !matches!(kind, ColorKind::Gray | ColorKind::Guess) {
                return Ok(None);
            }
            expand_jpeg(&pixels, width, height, 1)
        }
        jpeg_decoder::PixelFormat::RGB24 => {
            if !matches!(kind, ColorKind::Rgb | ColorKind::Guess) {
                return Ok(None);
            }
            expand_jpeg(&pixels, width, height, 3)
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            if !matches!(kind, ColorKind::Cmyk | ColorKind::Guess) {
                return Ok(None);
            }
            expand_jpeg(&pixels, width, height, 4)
        }
        _ => return Ok(None),
    };
    Ok(rgb)
}

fn expand_jpeg(data: &[u8], width: u32, height: u32, n: usize) -> Option<RgbImage> {
    let expect = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(n)?;
    if data.len() != expect {
        return None;
    }
    let mut rgb = Vec::new();
    let need = (width as usize).checked_mul(height as usize)?.checked_mul(3)?;
    if rgb.try_reserve_exact(need).is_err() {
        return None;
    }
    for pixel in data.chunks_exact(n) {
        match n {
            1 => {
                let g = pixel[0];
                rgb.extend_from_slice(&[g, g, g]);
            }
            3 => rgb.extend_from_slice(pixel),
            4 => {
                let c = pixel[0] as f64 / 255.0;
                let m = pixel[1] as f64 / 255.0;
                let y = pixel[2] as f64 / 255.0;
                let k = pixel[3] as f64 / 255.0;
                let color = cmyk_bytes(c, m, y, k);
                rgb.extend_from_slice(&color);
            }
            _ => return None,
        }
    }
    Some(RgbImage {
        width,
        height,
        rgb,
    })
}

fn pack_rgb(
    width: u32,
    height: u32,
    bpc: u8,
    kind: ColorKind,
    data: &[u8],
    decode: Option<&[f64]>,
) -> Option<RgbImage> {
    let n = if bpc == 1 {
        1
    } else {
        match kind {
            ColorKind::Gray => 1,
            ColorKind::Rgb => 3,
            ColorKind::Cmyk => 4,
            ColorKind::Guess => guess_components(width, height, data)?,
            ColorKind::Unsupported => return None,
        }
    };
    if bpc == 1 && n != 1 {
        return None;
    }
    let samples = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(n as u64);
    if samples > MAX_RASTER_PIXELS {
        return None;
    }
    let expected = if bpc == 1 {
        let stride = (width as usize + 7) / 8;
        stride.checked_mul(height as usize)?
    } else {
        (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(n)?
    };
    if data.len() != expected {
        return None;
    }
    let mut rgb = Vec::new();
    let need = (width as usize).checked_mul(height as usize)?.checked_mul(3)?;
    if rgb.try_reserve_exact(need).is_err() {
        return None;
    }
    rgb.resize(need, 0);
    if bpc == 1 {
        let stride = (width as usize + 7) / 8;
        for y in 0..height as usize {
            for x in 0..width as usize {
                let byte = data[y * stride + x / 8];
                let bit = (byte >> (7 - (x % 8))) & 1;
                let unit = apply_decode(if bit == 0 { 0.0 } else { 1.0 }, decode, 0);
                let g = byte_from_unit(unit);
                let i = (y * width as usize + x) * 3;
                rgb[i] = g;
                rgb[i + 1] = g;
                rgb[i + 2] = g;
            }
        }
    } else {
        for y in 0..height as usize {
            for x in 0..width as usize {
                let src = (y * width as usize + x) * n;
                let color = match n {
                    1 => gray_bytes(apply_decode(data[src] as f64 / 255.0, decode, 0)),
                    3 => rgb_bytes(
                        apply_decode(data[src] as f64 / 255.0, decode, 0),
                        apply_decode(data[src + 1] as f64 / 255.0, decode, 1),
                        apply_decode(data[src + 2] as f64 / 255.0, decode, 2),
                    ),
                    4 => cmyk_bytes(
                        apply_decode(data[src] as f64 / 255.0, decode, 0),
                        apply_decode(data[src + 1] as f64 / 255.0, decode, 1),
                        apply_decode(data[src + 2] as f64 / 255.0, decode, 2),
                        apply_decode(data[src + 3] as f64 / 255.0, decode, 3),
                    ),
                    _ => return None,
                };
                let i = (y * width as usize + x) * 3;
                rgb[i] = color[0];
                rgb[i + 1] = color[1];
                rgb[i + 2] = color[2];
            }
        }
    }
    Some(RgbImage {
        width,
        height,
        rgb,
    })
}

fn guess_components(width: u32, height: u32, data: &[u8]) -> Option<usize> {
    let pixels = (width as usize).checked_mul(height as usize)?;
    if pixels == 0 {
        return None;
    }
    if data.len() >= pixels.saturating_mul(4) {
        Some(4)
    } else if data.len() >= pixels.saturating_mul(3) {
        Some(3)
    } else {
        Some(1)
    }
}

fn apply_decode(unit: f64, decode: Option<&[f64]>, component: usize) -> f64 {
    if let Some(decode) = decode {
        let index = component * 2;
        if index + 1 < decode.len() {
            return decode[index] + unit * (decode[index + 1] - decode[index]);
        }
    }
    unit
}

fn encode_jpeg(pixels: &[u8], width: u32, height: u32, quality: u8) -> PdfResult<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());
    jpeg_encoder::Encoder::new(&mut cursor, quality)
        .encode(
            pixels,
            width as u16,
            height as u16,
            jpeg_encoder::ColorType::Rgb,
        )
        .map_err(|err| PdfError::OperationError(format!("JPEG encode failed: {err}")))?;
    Ok(cursor.into_inner())
}

fn encode_tiff(width: u32, height: u32, dpi: f64, pixels: &[u8]) -> PdfResult<Vec<u8>> {
    let byte_count = (width as u64)
        .checked_mul(height as u64)
        .and_then(|n| n.checked_mul(3))
        .ok_or_else(|| raster_err("Raster buffer could not be allocated."))?;
    if byte_count > u32::MAX as u64 || pixels.len() as u64 != byte_count {
        return Err(raster_err("Raster buffer could not be allocated."));
    }
    let bits_off = 158u32;
    let xres_off = 164u32;
    let yres_off = 172u32;
    let pixels_off = 180u32;
    let rational = (dpi * 100.0).round() as u32;
    let mut out = Vec::with_capacity(pixels_off as usize + pixels.len());
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&12u16.to_le_bytes());
    write_long(&mut out, 256, width);
    write_long(&mut out, 257, height);
    write_typed(&mut out, 258, 3, 3, bits_off);
    write_short(&mut out, 259, 1);
    write_short(&mut out, 262, 2);
    write_long(&mut out, 273, pixels_off);
    write_short(&mut out, 277, 3);
    write_long(&mut out, 278, height);
    write_long(&mut out, 279, byte_count as u32);
    write_typed(&mut out, 282, 5, 1, xres_off);
    write_typed(&mut out, 283, 5, 1, yres_off);
    write_short(&mut out, 296, 2);
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..3 {
        out.extend_from_slice(&8u16.to_le_bytes());
    }
    out.extend_from_slice(&rational.to_le_bytes());
    out.extend_from_slice(&100u32.to_le_bytes());
    out.extend_from_slice(&rational.to_le_bytes());
    out.extend_from_slice(&100u32.to_le_bytes());
    out.extend_from_slice(pixels);
    Ok(out)
}

fn write_short(out: &mut Vec<u8>, tag: u16, value: u16) {
    write_typed(out, tag, 3, 1, value as u32);
}

fn write_long(out: &mut Vec<u8>, tag: u16, value: u32) {
    write_typed(out, tag, 4, 1, value);
}

fn write_typed(out: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32) {
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&value.to_le_bytes());
}

fn rotate_rgb(pixels: Vec<u8>, width: u32, height: u32, rotation: i32) -> PdfResult<(Vec<u8>, u32, u32)> {
    match rotation {
        90 => rotate_into(pixels, width, height, height, width, |x, y, _w, h| (h - 1 - y, x)),
        180 => rotate_into(pixels, width, height, width, height, |x, y, w, h| (w - 1 - x, h - 1 - y)),
        270 => rotate_into(pixels, width, height, height, width, |x, y, w, _h| (y, w - 1 - x)),
        _ => Ok((pixels, width, height)),
    }
}

fn rotate_into(
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    new_w: u32,
    new_h: u32,
    map: impl Fn(u32, u32, u32, u32) -> (u32, u32),
) -> PdfResult<(Vec<u8>, u32, u32)> {
    let nbytes = buffer_len(new_w, new_h)?;
    let mut dest = Vec::new();
    dest.try_reserve_exact(nbytes)
        .map_err(|_| raster_err("Raster buffer could not be allocated."))?;
    dest.resize(nbytes, 255);
    for y in 0..height {
        for x in 0..width {
            let (dx, dy) = map(x, y, width, height);
            let src = ((y as usize) * (width as usize) + (x as usize)) * 3;
            let dst = ((dy as usize) * (new_w as usize) + (dx as usize)) * 3;
            dest[dst] = pixels[src];
            dest[dst + 1] = pixels[src + 1];
            dest[dst + 2] = pixels[src + 2];
        }
    }
    Ok((dest, new_w, new_h))
}

const UNKNOWN_GLYPH: [u8; 7] = [0x1F, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1F];

/// Original 5 by 7 face. Bit 0x10 is the leftmost column. Row 0 is the top.
const GLYPHS: &[(char, [u8; 7])] = &[
    ('(', [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02]),
    (')', [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08]),
    (',', [0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x08]),
    ('-', [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00]),
    ('.', [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04]),
    ('/', [0x01, 0x02, 0x04, 0x04, 0x08, 0x10, 0x00]),
    ('0', [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E]),
    ('1', [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E]),
    ('2', [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F]),
    ('3', [0x1F, 0x01, 0x02, 0x06, 0x01, 0x11, 0x0E]),
    ('4', [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02]),
    ('5', [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E]),
    ('6', [0x0E, 0x10, 0x10, 0x1E, 0x11, 0x11, 0x0E]),
    ('7', [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08]),
    ('8', [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E]),
    ('9', [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x01, 0x0E]),
    ('A', [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]),
    ('B', [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E]),
    ('C', [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E]),
    ('D', [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C]),
    ('E', [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F]),
    ('F', [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10]),
    ('G', [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0E]),
    ('H', [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]),
    ('I', [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E]),
    ('J', [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C]),
    ('K', [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11]),
    ('L', [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F]),
    ('M', [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11]),
    ('N', [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11]),
    ('O', [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E]),
    ('P', [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10]),
    ('Q', [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D]),
    ('R', [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11]),
    ('S', [0x0E, 0x11, 0x10, 0x0E, 0x01, 0x11, 0x0E]),
    ('T', [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
    ('U', [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E]),
    ('V', [0x11, 0x11, 0x11, 0x11, 0x0A, 0x0A, 0x04]),
    ('W', [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11]),
    ('X', [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11]),
    ('Y', [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04]),
    ('Z', [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F]),
    ('a', [0x00, 0x00, 0x0E, 0x01, 0x0F, 0x11, 0x0F]),
    ('b', [0x10, 0x10, 0x1E, 0x11, 0x11, 0x11, 0x1E]),
    ('c', [0x00, 0x00, 0x0E, 0x10, 0x10, 0x11, 0x0E]),
    ('d', [0x01, 0x01, 0x0F, 0x11, 0x11, 0x11, 0x0F]),
    ('e', [0x00, 0x00, 0x0E, 0x11, 0x1F, 0x10, 0x0E]),
    ('f', [0x06, 0x08, 0x08, 0x1E, 0x08, 0x08, 0x08]),
    ('g', [0x00, 0x00, 0x0F, 0x11, 0x0F, 0x01, 0x0E]),
    ('h', [0x10, 0x10, 0x16, 0x19, 0x11, 0x11, 0x11]),
    ('i', [0x04, 0x00, 0x04, 0x04, 0x04, 0x04, 0x04]),
    ('j', [0x02, 0x00, 0x06, 0x02, 0x02, 0x12, 0x0C]),
    ('k', [0x10, 0x10, 0x12, 0x14, 0x18, 0x14, 0x12]),
    ('l', [0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x06]),
    ('m', [0x00, 0x00, 0x1A, 0x15, 0x15, 0x11, 0x11]),
    ('n', [0x00, 0x00, 0x16, 0x19, 0x11, 0x11, 0x11]),
    ('o', [0x00, 0x00, 0x0E, 0x11, 0x11, 0x11, 0x0E]),
    ('p', [0x00, 0x00, 0x1E, 0x11, 0x1E, 0x10, 0x10]),
    ('q', [0x00, 0x00, 0x0F, 0x11, 0x0F, 0x01, 0x01]),
    ('r', [0x00, 0x00, 0x16, 0x19, 0x10, 0x10, 0x10]),
    ('s', [0x00, 0x00, 0x0F, 0x10, 0x0E, 0x01, 0x1E]),
    ('t', [0x08, 0x08, 0x1E, 0x08, 0x08, 0x09, 0x06]),
    ('u', [0x00, 0x00, 0x11, 0x11, 0x11, 0x13, 0x0D]),
    ('v', [0x00, 0x00, 0x11, 0x11, 0x11, 0x0A, 0x04]),
    ('w', [0x00, 0x00, 0x11, 0x11, 0x15, 0x15, 0x0A]),
    ('x', [0x00, 0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11]),
    ('y', [0x00, 0x00, 0x11, 0x11, 0x0F, 0x01, 0x0E]),
    ('z', [0x00, 0x00, 0x1F, 0x02, 0x04, 0x08, 0x1F]),
];

fn glyph_rows(ch: char) -> &'static [u8; 7] {
    match GLYPHS.binary_search_by_key(&ch, |row| row.0) {
        Ok(index) => &GLYPHS[index].1,
        Err(_) => &UNKNOWN_GLYPH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cos::{encode_flate, Compression, ObjectId, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfStream, Writer};

    fn write_pdf(
        media: [i64; 4],
        crop: Option<[i64; 4]>,
        rotate: Option<i64>,
        content: &[u8],
        xobjects: &[(&str, PdfStream)],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut writer = Writer::new(&mut out);
        writer.write_header("1.7").unwrap();
        let mut offsets = Vec::new();
        let catalog_id = ObjectId::new(1);
        let pages_id = ObjectId::new(2);
        let page_id = ObjectId::new(3);
        let content_id = ObjectId::new(4);

        let mut stream_dict = PdfDictionary::new();
        stream_dict.insert("Length", content.len() as i64);
        let content_obj = PdfObject::Stream(PdfStream::new(stream_dict, content.to_vec()));

        let mut xobj_dict = PdfDictionary::new();
        let mut xobj_objs = Vec::new();
        for (index, (name, stream)) in xobjects.iter().enumerate() {
            let id = ObjectId::new(5 + index as u32);
            xobj_dict.insert(*name, id);
            xobj_objs.push((id, PdfObject::Stream(stream.clone())));
        }

        let mut font_f1 = PdfDictionary::new();
        font_f1.insert("Type", PdfName::new("Font"));
        font_f1.insert("Subtype", PdfName::new("Type1"));
        font_f1.insert("BaseFont", PdfName::new("Helvetica"));
        let mut font_dict = PdfDictionary::new();
        font_dict.insert("F1", PdfObject::Dictionary(font_f1));
        let mut resources = PdfDictionary::new();
        resources.insert("Font", PdfObject::Dictionary(font_dict));
        if !xobjects.is_empty() {
            resources.insert("XObject", PdfObject::Dictionary(xobj_dict));
        }

        let mut page = PdfDictionary::new();
        page.insert("Type", PdfName::new("Page"));
        page.insert("Parent", pages_id);
        page.insert(
            "MediaBox",
            vec![
                media[0].into(),
                media[1].into(),
                media[2].into(),
                media[3].into(),
            ],
        );
        if let Some(crop) = crop {
            page.insert(
                "CropBox",
                vec![crop[0].into(), crop[1].into(), crop[2].into(), crop[3].into()],
            );
        }
        if let Some(rotate) = rotate {
            page.insert("Rotate", rotate);
        }
        page.insert("Contents", content_id);
        page.insert("Resources", PdfObject::Dictionary(resources));

        let mut catalog = PdfDictionary::new();
        catalog.insert("Type", PdfName::new("Catalog"));
        catalog.insert("Pages", pages_id);
        let off = writer
            .write_indirect_object(catalog_id, &PdfObject::Dictionary(catalog))
            .unwrap();
        offsets.push((catalog_id, off));

        let mut pages = PdfDictionary::new();
        pages.insert("Type", PdfName::new("Pages"));
        pages.insert("Kids", vec![PdfObject::Reference(page_id)]);
        pages.insert("Count", 1i64);
        let off = writer
            .write_indirect_object(pages_id, &PdfObject::Dictionary(pages))
            .unwrap();
        offsets.push((pages_id, off));

        let off = writer
            .write_indirect_object(page_id, &PdfObject::Dictionary(page))
            .unwrap();
        offsets.push((page_id, off));
        let off = writer.write_indirect_object(content_id, &content_obj).unwrap();
        offsets.push((content_id, off));
        for (id, obj) in &xobj_objs {
            let off = writer.write_indirect_object(*id, obj).unwrap();
            offsets.push((*id, off));
        }

        let mut trailer = PdfDictionary::new();
        trailer.insert("Root", catalog_id);
        trailer.insert("Size", (offsets.len() + 1) as i64);
        writer.write_xref_and_trailer(&offsets, &trailer).unwrap();
        out
    }

    fn paint(bytes: &[u8], dpi: f64) -> PaintedPage {
        let mut doc = PdfDocument::load(bytes).unwrap();
        paint_page_rgb(&mut doc, 0, dpi).unwrap()
    }

    fn pixel(page: &PaintedPage, x: u32, y: u32) -> [u8; 3] {
        let i = ((y as usize) * (page.width as usize) + (x as usize)) * 3;
        [page.pixels[i], page.pixels[i + 1], page.pixels[i + 2]]
    }

    fn window_has_ink(page: &PaintedPage, crop: Rect, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
        for py in 0..page.height {
            for px in 0..page.width {
                let page_x = crop.min_x + (px as f64 + 0.5);
                let page_y = crop.max_y - (py as f64 + 0.5);
                if page_x >= x0 && page_x < x1 && page_y >= y0 && page_y < y1 {
                    let sample = pixel(page, px, py);
                    if sample != [255, 255, 255] {
                        return true;
                    }
                }
            }
        }
        false
    }

    #[test]
    fn glyph_table_is_sorted_and_locked() {
        for pair in GLYPHS.windows(2) {
            assert!(pair[0].0 < pair[1].0);
        }
        assert!(glyph_rows('A').contains(&0x1F));
        assert_eq!(glyph_rows('H')[0], 0x11);
        assert_ne!(glyph_rows('A'), glyph_rows('i'));
    }

    #[test]
    fn format_parse_accepts_aliases() {
        assert_eq!(RasterFormat::parse(" JPG ").unwrap(), RasterFormat::Jpeg);
        assert_eq!(RasterFormat::Jpeg.extension(), "jpg");
        assert_eq!(RasterFormat::Jpeg.media_type(), "image/jpeg");
        assert_eq!(RasterFormat::parse("tif").unwrap(), RasterFormat::Tiff);
        let err = RasterFormat::parse("gif").unwrap_err();
        assert!(err.to_string().contains("png, jpeg, or tiff"));
    }

    #[test]
    fn text_paints_near_the_baseline() {
        let bytes = write_pdf(
            [0, 0, 612, 792],
            None,
            None,
            b"BT /F1 12 Tf 72 700 Td (Hi) Tj ET",
            &[],
        );
        let page = paint(&bytes, 72.0);
        assert_eq!(page.omitted, 0);
        let crop = Rect::new(0.0, 0.0, 612.0, 792.0);
        assert!(window_has_ink(&page, crop, 72.0, 700.0, 152.0, 712.0));
    }

    #[test]
    fn filled_rect_is_exact_red() {
        let bytes = write_pdf(
            [0, 0, 200, 200],
            None,
            None,
            b"1 0 0 rg 20 20 40 40 re f",
            &[],
        );
        let page = paint(&bytes, 72.0);
        assert_eq!(page.omitted, 0);
        assert_eq!(pixel(&page, 40, 160), [255, 0, 0]);
    }

    #[test]
    fn encoded_signatures_and_tiff_pixel() {
        let bytes = write_pdf(
            [0, 0, 200, 200],
            None,
            None,
            b"1 0 0 rg 20 20 40 40 re f",
            &[],
        );
        let mut doc = PdfDocument::load(&bytes).unwrap();
        let png = render_page(&mut doc, 0, RasterFormat::Png, 72.0).unwrap();
        assert_eq!(&png.bytes[..8], b"\x89PNG\r\n\x1a\n");
        let jpeg = render_page(&mut doc, 0, RasterFormat::Jpeg, 72.0).unwrap();
        assert_eq!(&jpeg.bytes[..2], &[0xFF, 0xD8]);
        let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(jpeg.bytes.as_slice()));
        let decoded = decoder.decode().unwrap();
        let info = decoder.info().unwrap();
        assert_eq!((info.width, info.height), (200, 200));
        let i = (160 * 200 + 40) * 3;
        assert!(decoded[i] > 200 && decoded[i + 1] < 40 && decoded[i + 2] < 40);
        let tiff = render_page(&mut doc, 0, RasterFormat::Tiff, 72.0).unwrap();
        assert_eq!(&tiff.bytes[..4], b"II*\x00");
        assert_eq!(u16::from_le_bytes([tiff.bytes[8], tiff.bytes[9]]), 12);
        let off = 180 + (160 * 200 + 40) * 3;
        assert_eq!(&tiff.bytes[off..off + 3], &[255, 0, 0]);
    }

    #[test]
    fn dpi_scales_letter_and_rejects_bad_values() {
        let bytes = write_pdf([0, 0, 612, 792], None, None, b"", &[]);
        let low = paint(&bytes, 72.0);
        let high = paint(&bytes, 144.0);
        assert_eq!((low.width, low.height), (612, 792));
        assert_eq!((high.width, high.height), (1224, 1584));
        let mut doc = PdfDocument::load(&bytes).unwrap();
        for dpi in [0.0, f64::NAN, 1000.0] {
            let err = paint_page_rgb(&mut doc, 0, dpi).unwrap_err();
            assert!(err.to_string().contains("36 to 300"), "{err}");
        }
    }

    #[test]
    fn huge_page_is_rejected_before_allocation() {
        let bytes = write_pdf([0, 0, 10000, 10000], None, None, b"", &[]);
        let mut doc = PdfDocument::load(&bytes).unwrap();
        let err = paint_page_rgb(&mut doc, 0, 300.0).unwrap_err();
        assert!(err.to_string().contains("12000000"), "{err}");
    }

    #[test]
    fn rotate_90_moves_the_black_block() {
        let bytes = write_pdf(
            [0, 0, 612, 792],
            None,
            Some(90),
            b"0 0 0 rg 0 752 40 40 re f",
            &[],
        );
        let page = paint(&bytes, 72.0);
        assert_eq!((page.width, page.height), (792, 612));
        assert_eq!(pixel(&page, 780, 10), [0, 0, 0]);
        assert_eq!(pixel(&page, 10, 10), [255, 255, 255]);
    }

    #[test]
    fn crop_box_limits_the_bitmap() {
        let bytes = write_pdf(
            [0, 0, 300, 200],
            Some([0, 0, 144, 72]),
            None,
            b"1 0 0 rg 10 10 20 20 re f 1 0 0 rg 150 10 40 20 re f",
            &[],
        );
        let page = paint(&bytes, 72.0);
        assert_eq!((page.width, page.height), (144, 72));
        assert_eq!(pixel(&page, 20, 46), [255, 0, 0]);
        assert_eq!(pixel(&page, 143, 10), [255, 255, 255]);
    }

    #[test]
    fn rendering_mode_3_does_not_paint() {
        let bytes = write_pdf(
            [0, 0, 80, 80],
            None,
            None,
            b"1 0 0 rg 10 10 20 20 re f BT /F1 24 Tf 3 Tr 10 40 Td (Hi) Tj ET",
            &[],
        );
        let page = paint(&bytes, 72.0);
        assert_eq!(pixel(&page, 20, 59), [255, 0, 0]);
        for y in 0..page.height {
            for x in 0..page.width {
                let (page_x, page_y) = (
                    x as f64 + 0.5,
                    80.0 - (y as f64 + 0.5),
                );
                let inside = page_x >= 10.0 && page_x <= 30.0 && page_y >= 10.0 && page_y <= 30.0;
                if !inside {
                    assert_eq!(pixel(&page, x, y), [255, 255, 255]);
                }
            }
        }
    }

    #[test]
    fn broken_content_fails_and_render_does_not_rewrite() {
        let broken = write_pdf([0, 0, 72, 72], None, None, b"(", &[]);
        let mut doc = PdfDocument::load(&broken).unwrap();
        assert!(paint_page_rgb(&mut doc, 0, 72.0).is_err());

        let bytes = write_pdf([0, 0, 72, 72], None, None, b"1 0 0 rg 10 10 10 10 re f", &[]);
        let mut doc = PdfDocument::load(&bytes).unwrap();
        let page_id = doc.get_pages().unwrap()[0];
        let before = doc.get_page_content_bytes(page_id).unwrap();
        paint_page_rgb(&mut doc, 0, 72.0).unwrap();
        let after = doc.get_page_content_bytes(page_id).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn flate_image_samples_the_top_left_as_red() {
        let raw = [
            255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255,
        ];
        let encoded = encode_flate(&raw, Compression::default()).unwrap();
        let mut dict = PdfDictionary::new();
        dict.insert("Type", PdfName::new("XObject"));
        dict.insert("Subtype", PdfName::new("Image"));
        dict.insert("Width", 2i64);
        dict.insert("Height", 2i64);
        dict.insert("ColorSpace", PdfName::new("DeviceRGB"));
        dict.insert("BitsPerComponent", 8i64);
        dict.insert("Filter", PdfName::new("FlateDecode"));
        dict.insert("Length", encoded.len() as i64);
        let stream = PdfStream::new(dict, encoded);
        let bytes = write_pdf(
            [0, 0, 72, 72],
            None,
            None,
            b"q 20 0 0 20 10 10 cm /Im Do Q",
            &[("Im", stream)],
        );
        let page = paint(&bytes, 72.0);
        assert_eq!(page.omitted, 0);
        assert_eq!(pixel(&page, 15, 46), [255, 0, 0]);
    }

    #[test]
    fn jpeg_image_stays_red_and_missing_xobject_is_omitted() {
        let red = vec![255u8, 0, 0].repeat(64);
        let encoded = encode_jpeg(&red, 8, 8, 95).unwrap();
        let mut dict = PdfDictionary::new();
        dict.insert("Type", PdfName::new("XObject"));
        dict.insert("Subtype", PdfName::new("Image"));
        dict.insert("Width", 8i64);
        dict.insert("Height", 8i64);
        dict.insert("ColorSpace", PdfName::new("DeviceRGB"));
        dict.insert("BitsPerComponent", 8i64);
        dict.insert("Filter", PdfName::new("DCTDecode"));
        dict.insert("Length", encoded.len() as i64);
        let stream = PdfStream::new(dict, encoded);
        let bytes = write_pdf(
            [0, 0, 72, 72],
            None,
            None,
            b"q 72 0 0 72 0 0 cm /Im Do Q",
            &[("Im", stream)],
        );
        let page = paint(&bytes, 72.0);
        assert_eq!(page.omitted, 0);
        let sample = pixel(&page, 36, 36);
        assert!(sample[0] > 200 && sample[1] < 40 && sample[2] < 40, "{sample:?}");

        let missing = write_pdf([0, 0, 72, 72], None, None, b"/NoSuch Do", &[]);
        let page = paint(&missing, 72.0);
        assert_eq!(page.omitted, 1);
        let _ = red;
    }

    #[test]
    fn stroke_and_cubic_leave_ink() {
        let stroke = write_pdf(
            [0, 0, 72, 72],
            None,
            None,
            b"0 0 0 RG 2 w 10 10 m 50 10 l S",
            &[],
        );
        let page = paint(&stroke, 72.0);
        assert!(page.pixels.chunks(3).any(|px| px == [0, 0, 0]));
        let curve = write_pdf(
            [0, 0, 72, 72],
            None,
            None,
            b"1 0 0 rg 10 40 m 10 60 40 60 40 40 c h f",
            &[],
        );
        let page = paint(&curve, 72.0);
        assert!(page.pixels.chunks(3).any(|px| px == [255, 0, 0]));
    }
}
