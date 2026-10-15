//! Paints over image samples that fall inside a redaction rectangle.
//!
//! An 8-bit DeviceGray or DeviceRGB image with one decodable filter is rewritten
//! as Flate. A JPEG loses its DCT coefficients. A filter array, a 1-bit image,
//! CMYK, an inline image, an image inside a form, and an image inside an
//! appearance stay. A shared image is split so another placement keeps its own
//! samples. Structure-tree text is a separate pass.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use crate::cos::object::{ObjectId, PdfDictionary, PdfName, PdfObject, PdfStream};
use crate::cos::{decode_stream, encode_flate, Compression, PdfDocument};
use crate::error::{PdfError, PdfResult};
use crate::images::extract_page_images;
use crate::layout::geometry::Rect;
use crate::redact::types::RedactionRect;
use crate::stream::graphics_state::Matrix;
use crate::stream::parser::{build_ast_from_operations, serialize_ast, ContentStreamTokenizer};

const MAX_RASTER_PIXELS: u64 = 12_000_000;
const MAX_RASTER_SIDE: u32 = 65535;
const MAX_REDACT_NAMES: u32 = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColorKind {
    Gray,
    Rgb,
}

struct Decoded {
    width: u32,
    height: u32,
    components: usize,
    color_space: &'static str,
    samples: Vec<u8>,
}

struct SamplePlan {
    object_id: ObjectId,
    do_index: usize,
    width: u32,
    height: u32,
    color_space: &'static str,
    samples: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum XObjectHome {
    XObjectObject(ObjectId),
    ResourcesObject(ObjectId),
    PageInline(ObjectId),
}

/// Destroys samples of painted images that fall inside `redactions` on `page_id`.
///
/// The zone is the redaction rectangle. No glyph match is required. An image that
/// does not meet a rectangle is left byte for byte.
pub(super) fn blackout_image_pixels(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    redactions: &[RedactionRect],
) -> PdfResult<()> {
    if redactions.is_empty() {
        return Ok(());
    }
    let images = extract_page_images(doc, page_id)?;
    let image_names: HashSet<String> = images
        .iter()
        .map(|image| image.resource_name.trim_start_matches('/').to_string())
        .collect();
    let plans = sample_plans(doc, &images, redactions)?;
    if plans.is_empty() {
        return Ok(());
    }
    apply_plans(doc, page_id, &image_names, plans)
}

fn sample_plans(
    doc: &mut PdfDocument,
    images: &[crate::images::ImageInfo],
    redactions: &[RedactionRect],
) -> PdfResult<Vec<SamplePlan>> {
    let mut plans = Vec::new();
    let mut do_index = 0usize;
    for image in images {
        let painted = image.bbox.width() > 0.0 || image.bbox.height() > 0.0;
        if !painted {
            continue;
        }
        let index = do_index;
        do_index += 1;
        if image.ctm.inverse().is_none() {
            continue;
        }
        if !redactions.iter().any(|zone| image.bbox.intersects(&zone.rect)) {
            continue;
        }
        let Some(mut decoded) = decode_image(doc, image.object_id)? else {
            continue;
        };
        paint_samples(
            &mut decoded.samples,
            decoded.width,
            decoded.height,
            decoded.components,
            image.ctm,
            redactions,
        );
        plans.push(SamplePlan {
            object_id: image.object_id,
            do_index: index,
            width: decoded.width,
            height: decoded.height,
            color_space: decoded.color_space,
            samples: decoded.samples,
        });
    }
    Ok(plans)
}

fn apply_plans(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    image_names: &HashSet<String>,
    plans: Vec<SamplePlan>,
) -> PdfResult<()> {
    let counts = painted_placement_counts(doc)?;
    let mut plans_for: HashMap<ObjectId, usize> = HashMap::new();
    for plan in &plans {
        *plans_for.entry(plan.object_id).or_insert(0) += 1;
    }
    let mut used_names = page_xobject_names(doc, page_id)?;
    let mut replacements: HashMap<usize, String> = HashMap::new();
    let mut cloned_from = Vec::new();

    for plan in &plans {
        let painted = counts.get(&plan.object_id).copied().unwrap_or(0);
        let alone = painted == 1 && plans_for.get(&plan.object_id).copied().unwrap_or(0) == 1;
        if alone {
            rewrite_stream(doc, plan.object_id, plan)?;
            continue;
        }
        let new_id = doc.alloc_object_id();
        let name = fresh_name(&mut used_names)?;
        rewrite_stream(doc, new_id, plan)?;
        // The clone must carry the mask and the other dictionary entries of the source.
        copy_untouched_entries(doc, plan.object_id, new_id)?;
        replacements.insert(plan.do_index, name.clone());
        cloned_from.push((plan.object_id, new_id, name));
    }

    if !replacements.is_empty() {
        retarget_image_dos(doc, page_id, image_names, &replacements)?;
        for (_old_id, new_id, name) in &cloned_from {
            insert_xobject_name(doc, page_id, name, *new_id)?;
        }
        let mut released = HashSet::new();
        for (old_id, _new_id, _name) in &cloned_from {
            if released.insert(*old_id) {
                release_unpainted_copy(doc, *old_id)?;
            }
        }
    }
    Ok(())
}

/// Writes painted samples, then copies dictionary entries that `store_samples` does not own.
fn rewrite_stream(doc: &mut PdfDocument, id: ObjectId, plan: &SamplePlan) -> PdfResult<()> {
    let mut stream = if doc.objects.contains_key(&id) {
        match doc.get_object(id)? {
            PdfObject::Stream(stream) => stream,
            _ => {
                return Err(op_err(format!("Image object {id} is not a stream")));
            }
        }
    } else {
        PdfStream::new(PdfDictionary::new(), Vec::new())
    };
    store_samples(
        &mut stream,
        &plan.samples,
        plan.width,
        plan.height,
        plan.color_space,
    )?;
    doc.set_object(id, PdfObject::Stream(stream));
    Ok(())
}

fn copy_untouched_entries(doc: &mut PdfDocument, source_id: ObjectId, dest_id: ObjectId) -> PdfResult<()> {
    let source = match doc.get_object(source_id)? {
        PdfObject::Stream(stream) => stream,
        _ => return Err(op_err(format!("Image object {source_id} is not a stream"))),
    };
    let mut dest = match doc.get_object(dest_id)? {
        PdfObject::Stream(stream) => stream,
        _ => return Err(op_err(format!("Image object {dest_id} is not a stream"))),
    };
    for (key, value) in source.dict.iter() {
        let name = key.as_str().trim_start_matches('/');
        if matches!(
            name,
            "Length" | "Filter" | "Width" | "Height" | "ColorSpace" | "BitsPerComponent"
                | "DecodeParms" | "Decode"
        ) {
            continue;
        }
        if !dest.dict.contains_key(name) {
            dest.dict.insert(name, value.clone());
        }
    }
    if !dest.dict.contains_key("Type") {
        dest.dict.insert("Type", PdfObject::Name(PdfName::new("XObject")));
    }
    if !dest.dict.contains_key("Subtype") {
        dest.dict
            .insert("Subtype", PdfObject::Name(PdfName::new("Image")));
    }
    doc.set_object(dest_id, PdfObject::Stream(dest));
    Ok(())
}

fn store_samples(
    stream: &mut PdfStream,
    samples: &[u8],
    width: u32,
    height: u32,
    color_space: &str,
) -> PdfResult<()> {
    let encoded = encode_flate(samples, Compression::default()).map_err(|err| {
        PdfError::OperationError(format!("Image samples could not be compressed: {err}"))
    })?;
    stream.content = encoded;
    stream
        .dict
        .insert("Length", PdfObject::Integer(stream.content.len() as i64));
    stream
        .dict
        .insert("Filter", PdfObject::Name(PdfName::new("FlateDecode")));
    stream
        .dict
        .insert("Width", PdfObject::Integer(width as i64));
    stream
        .dict
        .insert("Height", PdfObject::Integer(height as i64));
    stream
        .dict
        .insert("ColorSpace", PdfObject::Name(PdfName::new(color_space)));
    stream
        .dict
        .insert("BitsPerComponent", PdfObject::Integer(8));
    stream.dict.remove("DecodeParms");
    stream.dict.remove("Decode");
    Ok(())
}

fn paint_samples(
    samples: &mut [u8],
    width: u32,
    height: u32,
    components: usize,
    ctm: Matrix,
    redactions: &[RedactionRect],
) {
    for iy in 0..height {
        for ix in 0..width {
            let u = if width <= 1 {
                0.5
            } else {
                ix as f64 / (width as f64 - 1.0)
            };
            let v = if height <= 1 {
                0.5
            } else {
                1.0 - (iy as f64 / (height as f64 - 1.0))
            };
            let (x, y) = ctm.transform_point(u, v);
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            let mut fill = None;
            for zone in redactions {
                if point_inside(zone.rect, x, y) {
                    fill = Some(zone.fill_color);
                }
            }
            let Some(color) = fill else {
                continue;
            };
            let offset = ((iy as usize) * (width as usize) + (ix as usize)) * components;
            if components == 1 {
                let luma = 0.299 * color[0] + 0.587 * color[1] + 0.114 * color[2];
                samples[offset] = unit_byte(luma);
            } else if offset + 2 < samples.len() {
                samples[offset] = unit_byte(color[0]);
                samples[offset + 1] = unit_byte(color[1]);
                samples[offset + 2] = unit_byte(color[2]);
            }
        }
    }
}

fn point_inside(rect: Rect, x: f64, y: f64) -> bool {
    x >= rect.min_x && x <= rect.max_x && y >= rect.min_y && y <= rect.max_y
}

fn unit_byte(channel: f64) -> u8 {
    (channel.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn decode_image(doc: &mut PdfDocument, id: ObjectId) -> PdfResult<Option<Decoded>> {
    let stream = match doc.get_object(id)? {
        PdfObject::Stream(stream) => stream,
        _ => return Ok(None),
    };
    let Some(width) = image_side(stream.dict.get("Width").and_then(|item| item.as_f64())) else {
        return Ok(None);
    };
    let Some(height) = image_side(stream.dict.get("Height").and_then(|item| item.as_f64())) else {
        return Ok(None);
    };
    let pixels = (width as u64).saturating_mul(height as u64);
    if pixels == 0 || pixels > MAX_RASTER_PIXELS {
        return Ok(None);
    }
    if !eight_bit(&stream.dict) {
        return Ok(None);
    }
    let Some(kind) = resolve_color(doc, stream.dict.get("ColorSpace"))? else {
        return Ok(None);
    };
    if !identity_decode(kind, &stream.dict) {
        return Ok(None);
    }
    let Some(filter) = single_filter_name(&stream.dict) else {
        return Ok(None);
    };
    let components = match kind {
        ColorKind::Gray => 1,
        ColorKind::Rgb => 3,
    };
    let Some(expected) = (width as usize)
        .checked_mul(height as usize)
        .and_then(|count| count.checked_mul(components))
    else {
        return Ok(None);
    };
    let samples = if filter == "DCTDecode" || filter == "DCT" {
        match decode_jpeg(&stream.content, width, height, kind) {
            Some(samples) => samples,
            None => return Ok(None),
        }
    } else {
        let params = inline_or_referenced_dict(doc, stream.dict.get("DecodeParms"))?;
        let limits = doc.limits.clone();
        match decode_stream(&filter, params.as_ref(), &stream.content, &limits) {
            Ok(samples) => samples,
            Err(PdfError::SecurityLimitExceeded(message)) => {
                return Err(PdfError::SecurityLimitExceeded(message));
            }
            Err(_) => return Ok(None),
        }
    };
    if samples.len() != expected {
        return Ok(None);
    }
    Ok(Some(Decoded {
        width,
        height,
        components,
        color_space: match kind {
            ColorKind::Gray => "DeviceGray",
            ColorKind::Rgb => "DeviceRGB",
        },
        samples,
    }))
}

fn decode_jpeg(data: &[u8], width: u32, height: u32, kind: ColorKind) -> Option<Vec<u8>> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(data));
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    if info.width as u32 != width || info.height as u32 != height {
        return None;
    }
    match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 if kind == ColorKind::Gray => Some(pixels),
        jpeg_decoder::PixelFormat::RGB24 if kind == ColorKind::Rgb => Some(pixels),
        _ => None,
    }
}

fn image_side(value: Option<f64>) -> Option<u32> {
    let value = value?;
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let rounded = value.round();
    if (rounded - value).abs() > 1e-6 || rounded < 1.0 || rounded > MAX_RASTER_SIDE as f64 {
        return None;
    }
    Some(rounded as u32)
}

fn eight_bit(dict: &PdfDictionary) -> bool {
    match dict.get("BitsPerComponent").and_then(|item| item.as_f64()) {
        Some(bits) => bits == 8.0,
        None => false,
    }
}

fn resolve_color(doc: &mut PdfDocument, value: Option<&PdfObject>) -> PdfResult<Option<ColorKind>> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        PdfObject::Name(name) => Ok(classify_color(name.as_str())),
        PdfObject::Reference(id) => match doc.get_object(*id)? {
            PdfObject::Name(name) => Ok(classify_color(name.as_str())),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

fn classify_color(name: &str) -> Option<ColorKind> {
    match name.trim_start_matches('/') {
        "DeviceGray" | "G" => Some(ColorKind::Gray),
        "DeviceRGB" | "RGB" => Some(ColorKind::Rgb),
        _ => None,
    }
}

fn identity_decode(kind: ColorKind, dict: &PdfDictionary) -> bool {
    let Some(value) = dict.get("Decode") else {
        return true;
    };
    let Some(items) = value.as_array() else {
        return false;
    };
    let numbers = items.iter().map(|item| item.as_f64()).collect::<Option<Vec<f64>>>();
    let Some(numbers) = numbers else {
        return false;
    };
    match kind {
        ColorKind::Gray => numbers == [0.0, 1.0],
        ColorKind::Rgb => numbers == [0.0, 1.0, 0.0, 1.0, 0.0, 1.0],
    }
}

/// A filter array, including an array of one name, is not rewritten.
fn single_filter_name(dict: &PdfDictionary) -> Option<String> {
    match dict.get("Filter") {
        None => Some(String::new()),
        Some(PdfObject::Name(name)) => {
            let name = name.as_str().trim_start_matches('/').to_string();
            if matches!(
                name.as_str(),
                "" | "Identity"
                    | "FlateDecode"
                    | "Fl"
                    | "ASCIIHexDecode"
                    | "AHx"
                    | "ASCII85Decode"
                    | "A85"
                    | "DCTDecode"
                    | "DCT"
            ) {
                Some(name)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn inline_or_referenced_dict(
    doc: &mut PdfDocument,
    value: Option<&PdfObject>,
) -> PdfResult<Option<PdfDictionary>> {
    match value {
        None | Some(PdfObject::Null) => Ok(None),
        Some(PdfObject::Dictionary(dict)) => Ok(Some(dict.clone())),
        Some(PdfObject::Reference(id)) => match doc.get_object(*id)? {
            PdfObject::Dictionary(dict) => Ok(Some(dict)),
            _ => Ok(None),
        },
        Some(_) => Ok(None),
    }
}

fn painted_placement_counts(doc: &mut PdfDocument) -> PdfResult<HashMap<ObjectId, usize>> {
    let mut counts = HashMap::new();
    let pages = doc.get_pages()?;
    for page_id in pages {
        for image in extract_page_images(doc, page_id)? {
            if image.bbox.width() > 0.0 || image.bbox.height() > 0.0 {
                *counts.entry(image.object_id).or_insert(0) += 1;
            }
        }
    }
    Ok(counts)
}

fn page_xobject_names(doc: &mut PdfDocument, page_id: ObjectId) -> PdfResult<HashSet<String>> {
    let mut names = HashSet::new();
    let Some(dict) = xobject_dict(doc, page_id)? else {
        return Ok(names);
    };
    for (key, _) in dict.iter() {
        names.insert(key.as_str().trim_start_matches('/').to_string());
    }
    Ok(names)
}

fn fresh_name(used: &mut HashSet<String>) -> PdfResult<String> {
    for number in 1..=MAX_REDACT_NAMES {
        let name = format!("RedactIm{number}");
        if !used.contains(&name) {
            used.insert(name.clone());
            return Ok(name);
        }
    }
    Err(op_err(
        "No free image resource name for a redacted placement.",
    ))
}

fn retarget_image_dos(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    image_names: &HashSet<String>,
    replacements: &HashMap<usize, String>,
) -> PdfResult<()> {
    let bytes = doc.get_page_content_bytes(page_id)?;
    let mut tokenizer = ContentStreamTokenizer::new(&bytes);
    let mut operations = tokenizer.tokenize_all()?;
    let mut seen = 0usize;
    let mut applied = HashSet::new();
    for operation in &mut operations {
        if operation.operator != "Do" {
            continue;
        }
        let Some(name) = operation
            .operands
            .first()
            .and_then(|operand| operand.as_name())
        else {
            continue;
        };
        let name = name.trim_start_matches('/').to_string();
        if !image_names.contains(&name) {
            continue;
        }
        if let Some(new_name) = replacements.get(&seen) {
            if let Some(slot) = operation.operands.first_mut() {
                *slot = PdfObject::Name(PdfName::new(new_name));
            }
            applied.insert(seen);
        }
        seen += 1;
    }
    if applied.len() != replacements.len() {
        return Err(op_err(
            "A redacted image placement could not be retargeted.",
        ));
    }
    let ast = build_ast_from_operations(operations);
    write_content_stream(doc, page_id, serialize_ast(&ast))
}

fn write_content_stream(doc: &mut PdfDocument, page_id: ObjectId, bytes: Vec<u8>) -> PdfResult<()> {
    let page = match doc.get_object(page_id)? {
        PdfObject::Dictionary(dict) => dict,
        _ => return Err(op_err(format!("Page object {page_id} is not a dictionary"))),
    };
    let contents_id = match page.get("Contents") {
        Some(PdfObject::Reference(id)) => *id,
        Some(PdfObject::Array(items)) => items
            .iter()
            .find_map(|item| item.as_reference())
            .ok_or_else(|| op_err("Page has no content stream"))?,
        _ => return Err(op_err("Page has no content stream")),
    };
    let mut stream = match doc.get_object(contents_id)? {
        PdfObject::Stream(stream) => stream,
        _ => return Err(op_err("Page content is not a stream")),
    };
    stream
        .dict
        .insert("Length", PdfObject::Integer(bytes.len() as i64));
    stream.dict.remove("Filter");
    stream.dict.remove("DecodeParms");
    stream.content = bytes;
    doc.set_object(contents_id, PdfObject::Stream(stream));
    Ok(())
}

fn insert_xobject_name(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    name: &str,
    image_id: ObjectId,
) -> PdfResult<()> {
    let name = name.to_string();
    with_page_xobjects(doc, page_id, |dict| {
        dict.insert(name.as_str(), PdfObject::Reference(image_id));
    })
}

fn release_unpainted_copy(doc: &mut PdfDocument, old_id: ObjectId) -> PdfResult<()> {
    let pages = doc.get_pages()?;
    let mut painting_homes = HashSet::new();
    let mut page_paints = Vec::new();
    for page_id in pages {
        let images = extract_page_images(doc, page_id)?;
        let paints = images.iter().any(|image| {
            image.object_id == old_id && (image.bbox.width() > 0.0 || image.bbox.height() > 0.0)
        });
        if paints {
            if let Some(home) = xobject_home(doc, page_id)? {
                painting_homes.insert(home);
            }
        }
        page_paints.push((page_id, paints));
    }
    for (page_id, paints) in page_paints {
        if paints {
            continue;
        }
        let Some(home) = xobject_home(doc, page_id)? else {
            continue;
        };
        if painting_homes.contains(&home) {
            continue;
        }
        with_page_xobjects(doc, page_id, |dict| strip_xobject_ref(dict, old_id))?;
    }
    if painting_homes.is_empty() && !is_referenced(doc, old_id) {
        doc.objects.remove(&old_id);
        doc.xref.entries.remove(&old_id);
    }
    Ok(())
}

fn strip_xobject_ref(dict: &mut PdfDictionary, target: ObjectId) {
    let keys: Vec<String> = dict
        .iter()
        .filter_map(|(key, value)| {
            if value.as_reference() == Some(target) {
                Some(key.as_str().to_string())
            } else {
                None
            }
        })
        .collect();
    for key in keys {
        dict.remove(key.trim_start_matches('/'));
        dict.remove(&key);
    }
}

fn xobject_home(doc: &mut PdfDocument, page_id: ObjectId) -> PdfResult<Option<XObjectHome>> {
    let page = match doc.get_object(page_id)? {
        PdfObject::Dictionary(dict) => dict,
        _ => return Ok(None),
    };
    match page.get("Resources") {
        Some(PdfObject::Reference(resources_id)) => {
            let resources_id = *resources_id;
            let resources = match doc.get_object(resources_id)? {
                PdfObject::Dictionary(dict) => dict,
                _ => return Ok(None),
            };
            Ok(home_from_resources(
                &resources,
                XObjectHome::ResourcesObject(resources_id),
            ))
        }
        Some(PdfObject::Dictionary(resources)) => {
            Ok(home_from_resources(resources, XObjectHome::PageInline(page_id)))
        }
        _ => Ok(None),
    }
}

fn home_from_resources(resources: &PdfDictionary, inline_home: XObjectHome) -> Option<XObjectHome> {
    match resources.get("XObject") {
        Some(PdfObject::Reference(id)) => Some(XObjectHome::XObjectObject(*id)),
        Some(PdfObject::Dictionary(_)) => Some(inline_home),
        _ => None,
    }
}

fn xobject_dict(doc: &mut PdfDocument, page_id: ObjectId) -> PdfResult<Option<PdfDictionary>> {
    let page = match doc.get_object(page_id)? {
        PdfObject::Dictionary(dict) => dict,
        _ => return Ok(None),
    };
    let resources = match page.get("Resources") {
        Some(PdfObject::Dictionary(dict)) => dict.clone(),
        Some(PdfObject::Reference(id)) => match doc.get_object(*id)? {
            PdfObject::Dictionary(dict) => dict,
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    match resources.get("XObject") {
        Some(PdfObject::Dictionary(dict)) => Ok(Some(dict.clone())),
        Some(PdfObject::Reference(id)) => match doc.get_object(*id)? {
            PdfObject::Dictionary(dict) => Ok(Some(dict)),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

fn with_page_xobjects(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    mutate: impl FnOnce(&mut PdfDictionary),
) -> PdfResult<()> {
    let page = match doc.get_object(page_id)? {
        PdfObject::Dictionary(dict) => dict,
        _ => return Err(op_err(format!("Page object {page_id} is not a dictionary"))),
    };
    match page.get("Resources").cloned() {
        Some(PdfObject::Reference(resources_id)) => {
            let mut resources = expect_dict(doc, resources_id)?;
            write_xobject_owner(doc, None, resources_id, &mut resources, mutate)?;
        }
        Some(PdfObject::Dictionary(mut resources)) => {
            let inline = write_xobject_owner(doc, Some(page_id), page_id, &mut resources, mutate)?;
            if inline {
                let mut page = page;
                page.insert("Resources", PdfObject::Dictionary(resources));
                doc.set_object(page_id, PdfObject::Dictionary(page));
            }
        }
        _ => return Err(op_err("Page has no resource dictionary")),
    }
    Ok(())
}

/// Returns true when the XObject dictionary is inline and the caller must store `resources`.
fn write_xobject_owner(
    doc: &mut PdfDocument,
    inline_page: Option<ObjectId>,
    owner_id: ObjectId,
    resources: &mut PdfDictionary,
    mutate: impl FnOnce(&mut PdfDictionary),
) -> PdfResult<bool> {
    match resources.get("XObject").cloned() {
        Some(PdfObject::Reference(xobject_id)) => {
            let mut xobjects = expect_dict(doc, xobject_id)?;
            mutate(&mut xobjects);
            doc.set_object(xobject_id, PdfObject::Dictionary(xobjects));
            Ok(false)
        }
        Some(PdfObject::Dictionary(_)) | None => {
            let mut xobjects = match resources.remove("XObject") {
                Some(PdfObject::Dictionary(dict)) => dict,
                _ => PdfDictionary::new(),
            };
            mutate(&mut xobjects);
            resources.insert("XObject", PdfObject::Dictionary(xobjects));
            if inline_page.is_none() {
                doc.set_object(owner_id, PdfObject::Dictionary(resources.clone()));
                Ok(false)
            } else {
                Ok(true)
            }
        }
        Some(_) => Err(op_err("Page XObject is not a dictionary")),
    }
}

fn expect_dict(doc: &mut PdfDocument, id: ObjectId) -> PdfResult<PdfDictionary> {
    match doc.get_object(id)? {
        PdfObject::Dictionary(dict) => Ok(dict),
        _ => Err(op_err(format!("Object {id} is not a dictionary"))),
    }
}

fn is_referenced(doc: &PdfDocument, target: ObjectId) -> bool {
    doc.objects
        .iter()
        .any(|(id, object)| *id != target && object_references(object, target))
}

fn object_references(object: &PdfObject, target: ObjectId) -> bool {
    match object {
        PdfObject::Reference(id) => *id == target,
        PdfObject::Array(items) => items.iter().any(|item| object_references(item, target)),
        PdfObject::Dictionary(dict) => dict.iter().any(|(_, value)| object_references(value, target)),
        PdfObject::Stream(stream) => stream
            .dict
            .iter()
            .any(|(_, value)| object_references(value, target)),
        _ => false,
    }
}

fn op_err(message: impl Into<String>) -> PdfError {
    PdfError::OperationError(message.into())
}
