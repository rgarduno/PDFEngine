//! Original TrueType face used when an archive must embed a simple font.
//!
//! Outlines are 5-by-7 bitmap rectangles. TrueType paints the right side of
//! the travel direction, so each filled cell runs bottom-left, top-left,
//! top-right, then bottom-right. The opposite order would leave the ink
//! outside the cell and the glyph would look empty.
//!
//! The `head` checksum follows the OpenType rule: the directory stores the
//! checksum taken while `checkSumAdjustment` is zero, and only the field
//! inside `head` is patched. Updating the directory as well would add that
//! word twice.

use std::collections::BTreeMap;

use crate::fonts::{GlyphFallback, WinAnsiEncoding};

const NUM_GLYPHS: usize = 225;
const UNITS_PER_EM: u16 = 1000;

/// Bitmap face plus the metrics a PDF font descriptor needs.
pub(crate) struct BuiltFace {
    pub bytes: Vec<u8>,
    pub bbox: [i16; 4],
    pub ascent: i16,
    pub descent: i16,
    pub fixed_pitch: bool,
    pub cap_height: i16,
    pub stem_v: i16,
    pub base_font: String,
}

/// Decoded simple-glyph header and point count.
#[cfg(test)]
pub(crate) struct GlyphOutline {
    pub contours: i16,
    pub xmin: i16,
    pub ymin: i16,
    pub xmax: i16,
    pub ymax: i16,
    pub points: usize,
}

/// Builds one face for the 224 WinAnsi widths from code 32 through 255.
pub(crate) fn build_face(widths: &[i32; 224], tag_index: u32) -> BuiltFace {
    let widths = clamped_widths(widths);
    let tag = tag_name(tag_index);
    let base_font = format!("{tag}+EngineFace");
    let notdef_advance = if widths.iter().all(|width| *width == widths[0]) {
        widths[0]
    } else {
        600
    };

    let mut glyphs = Vec::with_capacity(NUM_GLYPHS);
    glyphs.push(draw_notdef(notdef_advance));
    for code in 32u8..=255 {
        let advance = widths[(code - 32) as usize];
        glyphs.push(draw_char(code, advance));
    }

    let fixed_pitch = glyphs
        .iter()
        .all(|glyph| glyph.advance == glyphs[0].advance);
    let mut xmin = i16::MAX;
    let mut ymin = i16::MAX;
    let mut xmax = i16::MIN;
    let mut ymax = i16::MIN;
    let mut saw_ink = false;
    for glyph in glyphs.iter().filter(|glyph| !glyph.empty) {
        saw_ink = true;
        xmin = xmin.min(glyph.xmin);
        ymin = ymin.min(glyph.ymin);
        xmax = xmax.max(glyph.xmax);
        ymax = ymax.max(glyph.ymax);
    }
    if !saw_ink {
        xmin = 0;
        ymin = 0;
        xmax = 0;
        ymax = 0;
    }

    let ascent = (800.max(ymax as i32)).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    let descent = (-200.min(ymin as i32)).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    let cap_height = glyphs.get(34).map(|glyph| glyph.ymax).unwrap_or(0);
    let stem_source = glyphs.get(34).map(|glyph| glyph.stem).unwrap_or(1);
    let stem_v = stem_source.max(1);

    let hhea_metrics = horizontal_metrics(&glyphs, ascent, descent);
    let bytes = assemble_font(
        &glyphs,
        &base_font,
        [xmin, ymin, xmax, ymax],
        ascent,
        descent,
        fixed_pitch,
        &widths,
        hhea_metrics,
    );

    BuiltFace {
        bytes,
        bbox: [xmin, ymin, xmax, ymax],
        ascent,
        descent,
        fixed_pitch,
        cap_height,
        stem_v,
        base_font,
    }
}

/// Whole-file TrueType checksum. A finished face sums to `0xB1B0AFBA`.
#[cfg(test)]
pub(crate) fn font_checksum(bytes: &[u8]) -> u32 {
    sum32(bytes)
}

/// Reads one simple glyph. An empty `loca` span is a zero-contour glyph.
#[cfg(test)]
pub(crate) fn glyph_outline(font: &[u8], glyph_id: u16) -> Option<GlyphOutline> {
    let glyf = table_record(font, b"glyf")?;
    let loca = table_record(font, b"loca")?;
    let entry = loca.offset + glyph_id as usize * 4;
    if entry + 8 > loca.offset + loca.length || loca.offset + loca.length > font.len() {
        return None;
    }
    let start = read_u32(font, entry)? as usize;
    let end = read_u32(font, entry + 4)? as usize;
    if start > end || glyf.offset + end > font.len() {
        return None;
    }
    if start == end {
        return Some(GlyphOutline {
            contours: 0,
            xmin: 0,
            ymin: 0,
            xmax: 0,
            ymax: 0,
            points: 0,
        });
    }
    let data = font.get(glyf.offset + start..glyf.offset + end)?;
    if data.len() < 10 {
        return None;
    }
    let contours = read_i16(data, 0)?;
    let xmin = read_i16(data, 2)?;
    let ymin = read_i16(data, 4)?;
    let xmax = read_i16(data, 6)?;
    let ymax = read_i16(data, 8)?;
    if contours <= 0 {
        return Some(GlyphOutline {
            contours,
            xmin,
            ymin,
            xmax,
            ymax,
            points: 0,
        });
    }
    let count = contours as usize;
    let ends_at = 10 + count * 2;
    if data.len() < ends_at + 2 {
        return None;
    }
    let last = read_u16(data, ends_at - 2)? as usize;
    let npoints = last + 1;
    let instructions = read_u16(data, ends_at)? as usize;
    let mut cursor = ends_at + 2 + instructions;
    let mut flags = Vec::with_capacity(npoints);
    while flags.len() < npoints {
        let flag = *data.get(cursor)?;
        cursor += 1;
        let mut repeat = 1usize;
        if flag & 0x08 != 0 {
            repeat += *data.get(cursor)? as usize;
            cursor += 1;
        }
        for _ in 0..repeat {
            flags.push(flag);
            if flags.len() == npoints {
                break;
            }
        }
    }
    for flag in &flags {
        if flag & 0x02 != 0 {
            cursor += 1;
        } else if flag & 0x10 == 0 {
            cursor += 2;
        }
    }
    for flag in &flags {
        if flag & 0x04 != 0 {
            cursor += 1;
        } else if flag & 0x20 == 0 {
            cursor += 2;
        }
    }
    if cursor > data.len() {
        return None;
    }
    Some(GlyphOutline {
        contours,
        xmin,
        ymin,
        xmax,
        ymax,
        points: flags.len(),
    })
}

/// Directory length of one table, without the 4-byte file padding.
#[cfg(test)]
pub(crate) fn table_length(font: &[u8], tag: &[u8; 4]) -> Option<u32> {
    table_record(font, tag).map(|record| record.length as u32)
}

struct GlyphRec {
    data: Vec<u8>,
    advance: u16,
    lsb: i16,
    xmin: i16,
    ymin: i16,
    xmax: i16,
    ymax: i16,
    empty: bool,
    contours: usize,
    points: usize,
    stem: i16,
}

struct HheaMetrics {
    advance_max: u16,
    min_lsb: i16,
    min_rsb: i16,
    max_extent: i16,
}

#[cfg(test)]
struct TableRecord {
    offset: usize,
    length: usize,
}

fn clamped_widths(widths: &[i32; 224]) -> [i32; 224] {
    let mut out = [0i32; 224];
    for (index, width) in widths.iter().enumerate() {
        out[index] = (*width).clamp(0, 4000);
    }
    out
}

fn tag_name(index: u32) -> String {
    let mut value = index;
    let mut chars = ['A'; 6];
    for slot in (0..6).rev() {
        chars[slot] = char::from(b'A' + (value % 26) as u8);
        value /= 26;
    }
    chars.iter().collect()
}

fn cell_metrics(advance: i32) -> (i32, i32) {
    let advance = advance.max(0);
    let mut side = 40.min((advance / 4).max(0));
    if side >= advance {
        side = 0;
    }
    let mut cell = if advance > side + 10 {
        (advance - side - 10) / 5
    } else {
        0
    };
    if cell < 1 {
        side = 0;
        cell = if advance > 10 { (advance - 10) / 5 } else { 0 };
        if cell < 1 {
            cell = 1;
        }
    }
    while side + 5 * cell > advance.saturating_sub(10) {
        if cell > 1 {
            cell -= 1;
        } else if side > 0 {
            side -= 1;
        } else {
            break;
        }
    }
    (side, cell.max(1))
}

fn draw_notdef(advance: i32) -> GlyphRec {
    let (side, cell) = cell_metrics(advance);
    let contour = quad(side, 0, side + 5 * cell, 7 * cell);
    encode_glyph(&[contour], advance, cell)
}

fn draw_char(code: u8, advance: i32) -> GlyphRec {
    let (side, cell) = cell_metrics(advance);
    if code == 32 {
        return empty_glyph(advance);
    }
    let rows = bitmap_rows(code);
    let mut contours = Vec::new();
    for (row_index, bits) in rows.iter().enumerate() {
        for column in 0..5 {
            let mask = 1u8 << (4 - column);
            if bits & mask == 0 {
                continue;
            }
            let x0 = side + column * cell;
            let y0 = (6 - row_index as i32) * cell;
            contours.push(quad(x0, y0, x0 + cell, y0 + cell));
        }
    }
    if contours.is_empty() {
        contours.push(quad(side, 0, side + 5 * cell, 7 * cell));
    }
    encode_glyph(&contours, advance, cell)
}

fn empty_glyph(advance: i32) -> GlyphRec {
    GlyphRec {
        data: Vec::new(),
        advance: advance.clamp(0, 4000) as u16,
        lsb: 0,
        xmin: 0,
        ymin: 0,
        xmax: 0,
        ymax: 0,
        empty: true,
        contours: 0,
        points: 0,
        stem: 1,
    }
}

/// `GlyphFallback::transliterate` maps unlisted characters, including ASCII
/// letters, to `?`. Only a non-ASCII WinAnsi code is folded, and then marked.
fn bitmap_rows(code: u8) -> [u8; 7] {
    let unicode = WinAnsiEncoding::code_to_unicode(code);
    if unicode == ' ' {
        return rectangle_rows();
    }
    let ascii = if unicode.is_ascii() {
        unicode
    } else {
        GlyphFallback::transliterate(unicode)
    };
    let mut rows = rows_for_ascii(ascii);
    if !unicode.is_ascii() {
        rows[0] |= 0b00100;
    }
    if rows.iter().all(|row| *row == 0) {
        return rectangle_rows();
    }
    rows
}

fn rectangle_rows() -> [u8; 7] {
    [
        0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111,
    ]
}

fn rows_for_ascii(ch: char) -> [u8; 7] {
    match ch {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        '?' => [
            0b01110, 0b10001, 0b00001, 0b00110, 0b00100, 0b00000, 0b00100,
        ],
        other => hashed_rows(other),
    }
}

fn hashed_rows(ch: char) -> [u8; 7] {
    let mut state = 0x811c9dc5u32 ^ (ch as u32);
    let mut rows = [0u8; 7];
    for row in &mut rows {
        state = state.wrapping_mul(0x01000193);
        state ^= (ch as u32).wrapping_mul(0x9e3779b1);
        *row = 0b10001 | (((state >> 16) as u8) & 0b01110);
    }
    rows
}

fn quad(x0: i32, y0: i32, x1: i32, y1: i32) -> [(i32, i32); 4] {
    let left = x0.min(x1);
    let right = x0.max(x1);
    let bottom = y0.min(y1);
    let top = y0.max(y1);
    [(left, bottom), (left, top), (right, top), (right, bottom)]
}

fn encode_glyph(contours: &[[(i32, i32); 4]], advance: i32, stem: i32) -> GlyphRec {
    let mut points = Vec::new();
    for contour in contours {
        for point in contour {
            points.push((clamp_coord(point.0), clamp_coord(point.1)));
        }
    }
    let mut xmin = i32::MAX;
    let mut ymin = i32::MAX;
    let mut xmax = i32::MIN;
    let mut ymax = i32::MIN;
    for (x, y) in &points {
        xmin = xmin.min(*x);
        ymin = ymin.min(*y);
        xmax = xmax.max(*x);
        ymax = ymax.max(*y);
    }
    let mut flags = Vec::with_capacity(points.len());
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut previous = (0i32, 0i32);
    for point in &points {
        let mut flag = 0x01u8;
        emit_delta(point.0 - previous.0, 0x02, 0x10, &mut flag, &mut xs);
        emit_delta(point.1 - previous.1, 0x04, 0x20, &mut flag, &mut ys);
        flags.push(flag);
        previous = *point;
    }
    let mut data = Vec::new();
    data.extend_from_slice(&(contours.len() as i16).to_be_bytes());
    data.extend_from_slice(&(xmin as i16).to_be_bytes());
    data.extend_from_slice(&(ymin as i16).to_be_bytes());
    data.extend_from_slice(&(xmax as i16).to_be_bytes());
    data.extend_from_slice(&(ymax as i16).to_be_bytes());
    for index in 0..contours.len() {
        let end = (index * 4 + 3) as u16;
        data.extend_from_slice(&end.to_be_bytes());
    }
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&flags);
    data.extend_from_slice(&xs);
    data.extend_from_slice(&ys);
    if data.len() % 2 == 1 {
        data.push(0);
    }
    GlyphRec {
        data,
        advance: advance.clamp(0, 4000) as u16,
        lsb: xmin as i16,
        xmin: xmin as i16,
        ymin: ymin as i16,
        xmax: xmax as i16,
        ymax: ymax as i16,
        empty: false,
        contours: contours.len(),
        points: points.len(),
        stem: stem.clamp(1, i16::MAX as i32) as i16,
    }
}

fn emit_delta(delta: i32, short_bit: u8, same_bit: u8, flag: &mut u8, out: &mut Vec<u8>) {
    if delta == 0 {
        *flag |= same_bit;
        return;
    }
    if (-255..=255).contains(&delta) {
        *flag |= short_bit;
        if delta > 0 {
            *flag |= same_bit;
            out.push(delta as u8);
        } else {
            out.push((-delta) as u8);
        }
        return;
    }
    let wide = delta.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    out.extend_from_slice(&wide.to_be_bytes());
}

fn clamp_coord(value: i32) -> i32 {
    value.clamp(i16::MIN as i32, i16::MAX as i32)
}

fn horizontal_metrics(glyphs: &[GlyphRec], ascent: i16, descent: i16) -> HheaMetrics {
    let _ = (ascent, descent);
    let mut advance_max = 0u16;
    let mut min_lsb = i16::MAX;
    let mut min_rsb = i16::MAX;
    let mut max_extent = i16::MIN;
    for glyph in glyphs {
        advance_max = advance_max.max(glyph.advance);
        min_lsb = min_lsb.min(glyph.lsb);
        let width = if glyph.empty {
            0i32
        } else {
            i32::from(glyph.xmax) - i32::from(glyph.xmin)
        };
        let rsb = i32::from(glyph.advance) - i32::from(glyph.lsb) - width;
        let extent = i32::from(glyph.lsb) + width;
        min_rsb = min_rsb.min(rsb.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
        max_extent = max_extent.max(extent.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
    }
    HheaMetrics {
        advance_max,
        min_lsb,
        min_rsb,
        max_extent,
    }
}

fn assemble_font(
    glyphs: &[GlyphRec],
    base_font: &str,
    bbox: [i16; 4],
    ascent: i16,
    descent: i16,
    fixed_pitch: bool,
    widths: &[i32; 224],
    metrics: HheaMetrics,
) -> Vec<u8> {
    let mut glyf = Vec::new();
    let mut loca = Vec::with_capacity((NUM_GLYPHS + 1) * 4);
    for glyph in glyphs {
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        glyf.extend_from_slice(&glyph.data);
    }
    loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());

    let mut hmtx = Vec::with_capacity(NUM_GLYPHS * 4);
    for glyph in glyphs {
        hmtx.extend_from_slice(&glyph.advance.to_be_bytes());
        hmtx.extend_from_slice(&glyph.lsb.to_be_bytes());
    }

    let max_points = glyphs
        .iter()
        .map(|glyph| glyph.points)
        .max()
        .unwrap_or(0)
        .max(256) as u16;
    let max_contours = glyphs
        .iter()
        .map(|glyph| glyph.contours)
        .max()
        .unwrap_or(0)
        .max(64) as u16;

    let os2 = os2_table(widths, ascent, descent);
    let cmap = cmap_table();
    let head = head_table(bbox);
    let hhea = hhea_table(ascent, descent, metrics);
    let maxp = maxp_table(max_points, max_contours);
    let name = name_table(base_font);
    let post = post_table(fixed_pitch);

    debug_assert_eq!(os2.len(), 86);
    debug_assert_eq!(head.len(), 54);
    debug_assert_eq!(hhea.len(), 36);
    debug_assert_eq!(maxp.len(), 32);
    debug_assert_eq!(post.len(), 32);
    debug_assert_eq!(loca.len(), 904);
    debug_assert_eq!(hmtx.len(), 900);

    assemble_tables(vec![
        (*b"OS/2", os2),
        (*b"cmap", cmap),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ])
}

fn assemble_tables(mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by(|left, right| left.0.cmp(&right.0));
    let num_tables = tables.len() as u16;
    let (search_range, entry_selector, range_shift) = directory_search(num_tables);
    let mut offset = 12 + 16 * tables.len();
    let mut directory = Vec::with_capacity(16 * tables.len());
    let mut body = Vec::new();
    let mut head_at = None;
    for (tag, data) in &tables {
        let checksum = sum32_padded(data);
        directory.extend_from_slice(tag);
        directory.extend_from_slice(&checksum.to_be_bytes());
        directory.extend_from_slice(&(offset as u32).to_be_bytes());
        directory.extend_from_slice(&(data.len() as u32).to_be_bytes());
        if tag == b"head" {
            head_at = Some(offset);
        }
        body.extend_from_slice(data);
        let pad = (4 - (data.len() % 4)) % 4;
        body.extend(std::iter::repeat(0u8).take(pad));
        offset += data.len() + pad;
    }

    let mut file = Vec::with_capacity(offset);
    file.extend_from_slice(&0x00010000u32.to_be_bytes());
    file.extend_from_slice(&num_tables.to_be_bytes());
    file.extend_from_slice(&search_range.to_be_bytes());
    file.extend_from_slice(&entry_selector.to_be_bytes());
    file.extend_from_slice(&range_shift.to_be_bytes());
    file.extend_from_slice(&directory);
    file.extend_from_slice(&body);

    if let Some(head_offset) = head_at {
        let adjust_at = head_offset + 8;
        if adjust_at + 4 <= file.len() {
            let sum = sum32(&file);
            let adjustment = 0xB1B0AFBAu32.wrapping_sub(sum);
            file[adjust_at..adjust_at + 4].copy_from_slice(&adjustment.to_be_bytes());
        }
    }
    file
}

fn directory_search(num_tables: u16) -> (u16, u16, u16) {
    let mut power = 1u16;
    let mut entry = 0u16;
    while power.saturating_mul(2) <= num_tables {
        power *= 2;
        entry += 1;
    }
    let search = power.saturating_mul(16);
    let shift = num_tables.saturating_mul(16).saturating_sub(search);
    (search, entry, shift)
}

fn head_table(bbox: [i16; 4]) -> Vec<u8> {
    let mut data = Vec::with_capacity(54);
    push_u16(&mut data, 1);
    push_u16(&mut data, 0);
    push_u32(&mut data, 0x00010000);
    push_u32(&mut data, 0);
    push_u32(&mut data, 0x5F0F3CF5);
    push_u16(&mut data, 0x0001);
    push_u16(&mut data, UNITS_PER_EM);
    push_u32(&mut data, 0);
    push_u32(&mut data, 0);
    push_u32(&mut data, 0);
    push_u32(&mut data, 0);
    push_i16(&mut data, bbox[0]);
    push_i16(&mut data, bbox[1]);
    push_i16(&mut data, bbox[2]);
    push_i16(&mut data, bbox[3]);
    push_u16(&mut data, 0);
    push_u16(&mut data, 8);
    push_i16(&mut data, 2);
    push_i16(&mut data, 1);
    push_i16(&mut data, 0);
    debug_assert_eq!(data.len(), 54);
    data
}

fn hhea_table(ascent: i16, descent: i16, metrics: HheaMetrics) -> Vec<u8> {
    let mut data = Vec::with_capacity(36);
    push_u16(&mut data, 1);
    push_u16(&mut data, 0);
    push_i16(&mut data, ascent);
    push_i16(&mut data, descent);
    push_i16(&mut data, 0);
    push_u16(&mut data, metrics.advance_max);
    push_i16(&mut data, metrics.min_lsb);
    push_i16(&mut data, metrics.min_rsb);
    push_i16(&mut data, metrics.max_extent);
    push_i16(&mut data, 1);
    push_i16(&mut data, 0);
    push_i16(&mut data, 0);
    push_i16(&mut data, 0);
    push_i16(&mut data, 0);
    push_i16(&mut data, 0);
    push_i16(&mut data, 0);
    push_i16(&mut data, 0);
    push_u16(&mut data, NUM_GLYPHS as u16);
    debug_assert_eq!(data.len(), 36);
    data
}

fn maxp_table(max_points: u16, max_contours: u16) -> Vec<u8> {
    let mut data = Vec::with_capacity(32);
    push_u32(&mut data, 0x00010000);
    push_u16(&mut data, NUM_GLYPHS as u16);
    push_u16(&mut data, max_points);
    push_u16(&mut data, max_contours);
    push_u16(&mut data, 0);
    push_u16(&mut data, 0);
    push_u16(&mut data, 2);
    push_u16(&mut data, 0);
    push_u16(&mut data, 0);
    push_u16(&mut data, 0);
    push_u16(&mut data, 0);
    push_u16(&mut data, 0);
    push_u16(&mut data, 0);
    push_u16(&mut data, 0);
    push_u16(&mut data, 0);
    debug_assert_eq!(data.len(), 32);
    data
}

fn os2_table(widths: &[i32; 224], ascent: i16, descent: i16) -> Vec<u8> {
    let average = widths.iter().sum::<i32>() / 224;
    let win_ascent = (800.max(i32::from(ascent))) as u16;
    let mut data = Vec::with_capacity(86);
    push_u16(&mut data, 1);
    push_i16(&mut data, average as i16);
    push_u16(&mut data, 400);
    push_u16(&mut data, 5);
    push_u16(&mut data, 0);
    push_i16(&mut data, 500);
    push_i16(&mut data, 500);
    push_i16(&mut data, 0);
    push_i16(&mut data, 100);
    push_i16(&mut data, 500);
    push_i16(&mut data, 500);
    push_i16(&mut data, 0);
    push_i16(&mut data, 300);
    push_i16(&mut data, 50);
    push_i16(&mut data, 300);
    push_i16(&mut data, 0);
    data.extend_from_slice(&[0u8; 10]);
    push_u32(&mut data, 3);
    push_u32(&mut data, 0);
    push_u32(&mut data, 0);
    push_u32(&mut data, 0);
    data.extend_from_slice(b"pdfE");
    push_u16(&mut data, 0x0040);
    push_u16(&mut data, 32);
    push_u16(&mut data, 255);
    push_i16(&mut data, ascent);
    push_i16(&mut data, descent);
    push_i16(&mut data, 0);
    push_u16(&mut data, win_ascent);
    push_u16(&mut data, 200);
    push_u32(&mut data, 1);
    push_u32(&mut data, 0);
    debug_assert_eq!(data.len(), 86);
    data
}

fn post_table(fixed_pitch: bool) -> Vec<u8> {
    let mut data = Vec::with_capacity(32);
    push_u32(&mut data, 0x00030000);
    push_u32(&mut data, 0);
    push_i16(&mut data, -100);
    push_i16(&mut data, 50);
    push_u32(&mut data, if fixed_pitch { 1 } else { 0 });
    push_u32(&mut data, 0);
    push_u32(&mut data, 0);
    push_u32(&mut data, 0);
    push_u32(&mut data, 0);
    debug_assert_eq!(data.len(), 32);
    data
}

fn name_table(base_font: &str) -> Vec<u8> {
    let strings = [
        (1u16, "EngineFace"),
        (2, "Regular"),
        (3, base_font),
        (4, "EngineFace Regular"),
        (6, base_font),
    ];
    let encoded: Vec<(u16, Vec<u8>)> = strings
        .iter()
        .map(|(id, text)| (*id, utf16_be(text)))
        .collect();
    let mut data = Vec::new();
    push_u16(&mut data, 0);
    push_u16(&mut data, encoded.len() as u16);
    push_u16(&mut data, (6 + 12 * encoded.len()) as u16);
    let mut storage = Vec::new();
    for (id, bytes) in &encoded {
        push_u16(&mut data, 3);
        push_u16(&mut data, 1);
        push_u16(&mut data, 0x0409);
        push_u16(&mut data, *id);
        push_u16(&mut data, bytes.len() as u16);
        push_u16(&mut data, storage.len() as u16);
        storage.extend_from_slice(bytes);
    }
    data.extend_from_slice(&storage);
    data
}

fn utf16_be(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

fn cmap_table() -> Vec<u8> {
    let mut mapping: BTreeMap<u32, u16> = BTreeMap::new();
    for code in 32u16..=255 {
        let unicode = WinAnsiEncoding::code_to_unicode(code as u8) as u32;
        if unicode == 0x20 && code != 32 {
            continue;
        }
        let glyph = code - 31;
        mapping.entry(unicode).or_insert(glyph);
    }
    let mut segments = Vec::new();
    let entries: Vec<(u32, u16)> = mapping.into_iter().collect();
    let mut index = 0;
    while index < entries.len() {
        let (start_unicode, start_glyph) = entries[index];
        let delta = start_glyph as i32 - start_unicode as i32;
        if !(i16::MIN as i32..=i16::MAX as i32).contains(&delta) {
            index += 1;
            continue;
        }
        let mut end_unicode = start_unicode;
        let mut next = index + 1;
        while next < entries.len() {
            let (unicode, glyph) = entries[next];
            let step = glyph as i32 - unicode as i32;
            if unicode == end_unicode + 1 && step == delta {
                end_unicode = unicode;
                next += 1;
            } else {
                break;
            }
        }
        segments.push((start_unicode as u16, end_unicode as u16, delta as i16));
        index = next;
    }
    segments.push((0xFFFF, 0xFFFF, 1));

    let seg_count = segments.len() as u16;
    let seg_count_x2 = seg_count.saturating_mul(2);
    let search_range = (1u16 << floor_log2(seg_count as u32)).saturating_mul(2);
    let entry_selector = floor_log2(seg_count as u32) as u16;
    let range_shift = seg_count_x2.saturating_sub(search_range);
    let sub_len = 16 + 8 * seg_count as usize;

    let mut sub = Vec::with_capacity(sub_len);
    push_u16(&mut sub, 4);
    push_u16(&mut sub, sub_len as u16);
    push_u16(&mut sub, 0);
    push_u16(&mut sub, seg_count_x2);
    push_u16(&mut sub, search_range);
    push_u16(&mut sub, entry_selector);
    push_u16(&mut sub, range_shift);
    for (_, end, _) in &segments {
        push_u16(&mut sub, *end);
    }
    push_u16(&mut sub, 0);
    for (start, _, _) in &segments {
        push_u16(&mut sub, *start);
    }
    for (_, _, delta) in &segments {
        push_i16(&mut sub, *delta);
    }
    for _ in &segments {
        push_u16(&mut sub, 0);
    }
    debug_assert_eq!(sub.len(), sub_len);

    let mut table = Vec::with_capacity(12 + sub.len());
    push_u16(&mut table, 0);
    push_u16(&mut table, 1);
    push_u16(&mut table, 3);
    push_u16(&mut table, 1);
    push_u32(&mut table, 12);
    table.extend_from_slice(&sub);
    table
}

fn floor_log2(value: u32) -> u32 {
    if value == 0 {
        0
    } else {
        31 - value.leading_zeros()
    }
}

#[cfg(test)]
fn table_record(font: &[u8], tag: &[u8; 4]) -> Option<TableRecord> {
    if font.len() < 12 {
        return None;
    }
    let count = read_u16(font, 4)? as usize;
    for index in 0..count {
        let at = 12 + index * 16;
        let found = font.get(at..at + 4)?;
        if found == tag {
            let offset = read_u32(font, at + 8)? as usize;
            let length = read_u32(font, at + 12)? as usize;
            return Some(TableRecord { offset, length });
        }
    }
    None
}

fn sum32(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    let mut index = 0;
    while index + 4 <= data.len() {
        sum = sum.wrapping_add(u32::from_be_bytes([
            data[index],
            data[index + 1],
            data[index + 2],
            data[index + 3],
        ]));
        index += 4;
    }
    if index < data.len() {
        let mut word = [0u8; 4];
        word[..data.len() - index].copy_from_slice(&data[index..]);
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

fn sum32_padded(data: &[u8]) -> u32 {
    let pad = (4 - (data.len() % 4)) % 4;
    if pad == 0 {
        return sum32(data);
    }
    let mut padded = Vec::with_capacity(data.len() + pad);
    padded.extend_from_slice(data);
    padded.extend(std::iter::repeat(0u8).take(pad));
    sum32(&padded)
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push_i16(out: &mut Vec<u8>, value: i16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

#[cfg(test)]
fn read_u16(data: &[u8], at: usize) -> Option<u16> {
    let bytes: [u8; 2] = data.get(at..at + 2)?.try_into().ok()?;
    Some(u16::from_be_bytes(bytes))
}

#[cfg(test)]
fn read_i16(data: &[u8], at: usize) -> Option<i16> {
    let bytes: [u8; 2] = data.get(at..at + 2)?.try_into().ok()?;
    Some(i16::from_be_bytes(bytes))
}

#[cfg(test)]
fn read_u32(data: &[u8], at: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}
