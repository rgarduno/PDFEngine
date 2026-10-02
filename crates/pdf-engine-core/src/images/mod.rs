//! Image XObject extraction, inspection, and surgical in-place replacement (ISO 32000-1 §8.9).
//!
//! Supports JPEG (`/DCTDecode`) and PNG (`/FlateDecode`) extraction, spatial bounding box calculation
//! via CTM graphics state projection, and zero-drift in-place image swapping.

pub mod jpeg;
pub mod png;

pub use jpeg::{parse_jpeg, JpegHeader};
pub use png::{encode_png, parse_png_header, parse_png_pixels, PngHeader};

use std::collections::HashMap;
use std::io::Write;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::cos::filters::decode_stream;
use crate::cos::object::{ObjectId, PdfDictionary, PdfName, PdfObject, PdfStream};
use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::layout::geometry::Rect;
use crate::stream::graphics_state::{GraphicsStateStack, Matrix};
use crate::stream::parser::ContentStreamTokenizer;

/// Represents an extracted Image XObject on a page with its spatial placement.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageInfo {
    /// PDF Indirect Object identifier.
    pub object_id: ObjectId,
    /// Resource name registered in page `/Resources /XObject` (e.g. "Im0", "Image1").
    pub resource_name: String,
    /// Image width in pixels.
    pub width_px: u32,
    /// Image height in pixels.
    pub height_px: u32,
    /// Color space identifier (e.g. "DeviceRGB", "DeviceGray", "DeviceCMYK").
    pub color_space: String,
    /// Bits per component (typically 8).
    pub bits_per_component: u32,
    /// Compression filter name (e.g. "DCTDecode", "FlateDecode").
    pub filter: Option<String>,
    /// Byte length of the compressed stream.
    pub byte_size: usize,
    /// Computed spatial bounding box on the page in PDF points.
    pub bbox: Rect,
    /// Current Transformation Matrix active when the image was painted (`Do` operator).
    pub ctm: Matrix,
}

/// Extracts all Image XObjects on a specific page with their bounding boxes and transformation matrices.
pub fn extract_page_images(
    doc: &mut PdfDocument,
    page_id: ObjectId,
) -> PdfResult<Vec<ImageInfo>> {
    let page_obj = doc.get_object(page_id)?;
    let page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => return Ok(Vec::new()),
    };

    let resources = match page_dict.get("Resources") {
        Some(PdfObject::Dictionary(d)) => d.clone(),
        Some(PdfObject::Reference(r)) => match doc.get_object(*r)? {
            PdfObject::Dictionary(d) => d,
            _ => return Ok(Vec::new()),
        },
        _ => return Ok(Vec::new()),
    };

    let xobjects = match resources.get("XObject") {
        Some(PdfObject::Dictionary(d)) => d.clone(),
        Some(PdfObject::Reference(r)) => match doc.get_object(*r)? {
            PdfObject::Dictionary(d) => d,
            _ => return Ok(Vec::new()),
        },
        _ => return Ok(Vec::new()),
    };

    // 1. Catalog all Image XObjects declared in page resources
    struct RawImageMeta {
        object_id: ObjectId,
        width_px: u32,
        height_px: u32,
        color_space: String,
        bits_per_component: u32,
        filter: Option<String>,
        byte_size: usize,
    }

    let mut declared_images = HashMap::new();

    for (key, val) in xobjects.0 {
        let (obj_id, stream_dict, byte_size) = match val {
            PdfObject::Reference(r) => {
                let obj = doc.get_object(r)?;
                if let PdfObject::Stream(s) = obj {
                    (r, s.dict, s.content.len())
                } else {
                    continue;
                }
            }
            _ => continue,
        };

        let subtype = stream_dict.get("Subtype").and_then(|s| s.as_name()).unwrap_or("");
        if subtype != "Image" {
            continue;
        }

        let width = stream_dict.get("Width").and_then(|w| w.as_i64()).unwrap_or(0) as u32;
        let height = stream_dict.get("Height").and_then(|h| h.as_i64()).unwrap_or(0) as u32;
        let color_space = stream_dict
            .get("ColorSpace")
            .and_then(|c| c.as_name())
            .unwrap_or("DeviceRGB")
            .to_string();
        let bits = stream_dict.get("BitsPerComponent").and_then(|b| b.as_i64()).unwrap_or(8) as u32;
        let filter = stream_dict.get("Filter").and_then(|f| f.as_name()).map(|f| f.to_string());

        let clean_name = key.as_str().trim_start_matches('/').to_string();
        declared_images.insert(
            clean_name,
            RawImageMeta {
                object_id: obj_id,
                width_px: width,
                height_px: height,
                color_space,
                bits_per_component: bits,
                filter,
                byte_size,
            },
        );
    }

    if declared_images.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Parse page content stream to locate where each image is painted (`Do` operator)
    let content_bytes = doc.get_page_content_bytes(page_id)?;
    let mut tokenizer = ContentStreamTokenizer::new(&content_bytes);
    let operations = tokenizer.tokenize_all().unwrap_or_default();

    let mut state_stack = GraphicsStateStack::new();
    let mut image_instances = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    for op in operations {
        match op.operator.as_str() {
            "q" => {
                state_stack.push();
            }
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
                    state_stack.current.concat_matrix(&Matrix::new(a, b, c, d, e, f));
                }
            }
            "Do" => {
                if let Some(first_operand) = op.operands.first() {
                    let target_name = match first_operand {
                        PdfObject::Name(n) => n.as_str().trim_start_matches('/'),
                        _ => "",
                    };

                    if let Some(meta) = declared_images.get(target_name) {
                        seen_names.insert(target_name.to_string());
                        let ctm = state_stack.current.ctm;

                        // Image unit square [0, 0] to [1, 1] transformed by CTM
                        let p0 = ctm.transform_point(0.0, 0.0);
                        let p1 = ctm.transform_point(1.0, 0.0);
                        let p2 = ctm.transform_point(1.0, 1.0);
                        let p3 = ctm.transform_point(0.0, 1.0);

                        let min_x = p0.0.min(p1.0).min(p2.0).min(p3.0);
                        let min_y = p0.1.min(p1.1).min(p2.1).min(p3.1);
                        let max_x = p0.0.max(p1.0).max(p2.0).max(p3.0);
                        let max_y = p0.1.max(p1.1).max(p2.1).max(p3.1);

                        let bbox = Rect::new(min_x, min_y, max_x, max_y);

                        image_instances.push(ImageInfo {
                            object_id: meta.object_id,
                            resource_name: target_name.to_string(),
                            width_px: meta.width_px,
                            height_px: meta.height_px,
                            color_space: meta.color_space.clone(),
                            bits_per_component: meta.bits_per_component,
                            filter: meta.filter.clone(),
                            byte_size: meta.byte_size,
                            bbox,
                            ctm,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // 3. Include any declared images that weren't explicitly painted via Do
    for (name, meta) in declared_images {
        if !seen_names.contains(&name) {
            image_instances.push(ImageInfo {
                object_id: meta.object_id,
                resource_name: name,
                width_px: meta.width_px,
                height_px: meta.height_px,
                color_space: meta.color_space,
                bits_per_component: meta.bits_per_component,
                filter: meta.filter,
                byte_size: meta.byte_size,
                bbox: Rect::new(0.0, 0.0, 0.0, 0.0),
                ctm: Matrix::identity(),
            });
        }
    }

    Ok(image_instances)
}

/// Retrieves the raw image bytes and MIME type for browser rendering or export.
pub fn get_image_binary(
    doc: &mut PdfDocument,
    image_id: ObjectId,
) -> PdfResult<(Vec<u8>, &'static str)> {
    let obj = doc.get_object(image_id)?;
    let stream = match obj {
        PdfObject::Stream(s) => s,
        _ => {
            return Err(PdfError::TypeMismatch {
                id: image_id.number,
                gen: image_id.generation,
                expected: "Stream",
                found: "Non-stream object",
            });
        }
    };

    let filter = stream.dict.get("Filter").and_then(|f| f.as_name()).unwrap_or("");

    // If DCTDecode, stream content is already a valid JPEG file
    if filter == "DCTDecode" || filter == "DCT" {
        return Ok((stream.content, "image/jpeg"));
    }

    // If FlateDecode or uncompressed, decode samples and synthesize PNG
    let decode_parms = stream.dict.get("DecodeParms").and_then(|p| p.as_dict());
    let decoded = decode_stream(filter, decode_parms, &stream.content, &doc.limits)?;

    let width = stream.dict.get("Width").and_then(|w| w.as_i64()).unwrap_or(1) as u32;
    let height = stream.dict.get("Height").and_then(|h| h.as_i64()).unwrap_or(1) as u32;
    let color_space = stream.dict.get("ColorSpace").and_then(|c| c.as_name()).unwrap_or("DeviceRGB");
    let is_rgb = color_space != "DeviceGray";

    let png_bytes = encode_png(width, height, &decoded, is_rgb)?;
    Ok((png_bytes, "image/png"))
}

/// Surgically replaces an existing Image XObject in the document in-place.
pub fn replace_image_content(
    doc: &mut PdfDocument,
    image_id: ObjectId,
    new_bytes: &[u8],
) -> PdfResult<()> {
    if new_bytes.len() >= 2 && new_bytes[0] == 0xFF && new_bytes[1] == 0xD8 {
        // JPEG Replacement
        let header = parse_jpeg(new_bytes)?;
        let color_space = match header.components {
            1 => "DeviceGray",
            3 => "DeviceRGB",
            4 => "DeviceCMYK",
            _ => "DeviceRGB",
        };

        let stream = doc.get_stream_mut(image_id)?;
        stream.dict.insert("Type", PdfObject::Name(PdfName::new("XObject")));
        stream.dict.insert("Subtype", PdfObject::Name(PdfName::new("Image")));
        stream.dict.insert("Width", PdfObject::Integer(header.width as i64));
        stream.dict.insert("Height", PdfObject::Integer(header.height as i64));
        stream.dict.insert("ColorSpace", PdfObject::Name(PdfName::new(color_space)));
        stream.dict.insert("BitsPerComponent", PdfObject::Integer(header.precision as i64));
        stream.dict.insert("Filter", PdfObject::Name(PdfName::new("DCTDecode")));
        stream.dict.insert("Length", PdfObject::Integer(new_bytes.len() as i64));
        stream.dict.remove("DecodeParms");
        stream.dict.remove("SMask");
        stream.content = new_bytes.to_vec();

        Ok(())
    } else if new_bytes.len() >= 8 && &new_bytes[0..8] == png::PNG_SIGNATURE {
        // PNG Replacement
        let (width, height, color_samples, alpha_opt) = parse_png_pixels(new_bytes)?;

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&color_samples)
            .map_err(|e| PdfError::DecompressionError {
                filter: "FlateDecode".to_string(),
                message: format!("PNG color zlib compression failed: {}", e),
            })?;
        let compressed_rgb = encoder.finish().map_err(|e| PdfError::DecompressionError {
            filter: "FlateDecode".to_string(),
            message: format!("PNG color zlib finish failed: {}", e),
        })?;

        let compressed_alpha = if let Some(alpha_samples) = alpha_opt {
            let mut a_encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            a_encoder
                .write_all(&alpha_samples)
                .map_err(|e| PdfError::DecompressionError {
                    filter: "FlateDecode".to_string(),
                    message: format!("Alpha zlib compression failed: {}", e),
                })?;
            Some(a_encoder.finish().map_err(|e| PdfError::DecompressionError {
                filter: "FlateDecode".to_string(),
                message: format!("Alpha zlib finish failed: {}", e),
            })?)
        } else {
            None
        };

        let stream = doc.get_stream_mut(image_id)?;
        stream.dict.insert("Type", PdfObject::Name(PdfName::new("XObject")));
        stream.dict.insert("Subtype", PdfObject::Name(PdfName::new("Image")));
        stream.dict.insert("Width", PdfObject::Integer(width as i64));
        stream.dict.insert("Height", PdfObject::Integer(height as i64));
        stream.dict.insert("ColorSpace", PdfObject::Name(PdfName::new("DeviceRGB")));
        stream.dict.insert("BitsPerComponent", PdfObject::Integer(8));
        stream.dict.insert("Filter", PdfObject::Name(PdfName::new("FlateDecode")));
        stream.dict.insert("Length", PdfObject::Integer(compressed_rgb.len() as i64));
        stream.dict.remove("DecodeParms");
        stream.content = compressed_rgb;

        if let Some(alpha_bytes) = compressed_alpha {
            let smask_id = if let Some(r) = stream.dict.get("SMask").and_then(|s| s.as_reference()) {
                r
            } else {
                doc.alloc_object_id()
            };

            let mut smask_dict = PdfDictionary::new();
            smask_dict.insert("Type", PdfObject::Name(PdfName::new("XObject")));
            smask_dict.insert("Subtype", PdfObject::Name(PdfName::new("Image")));
            smask_dict.insert("Width", PdfObject::Integer(width as i64));
            smask_dict.insert("Height", PdfObject::Integer(height as i64));
            smask_dict.insert("ColorSpace", PdfObject::Name(PdfName::new("DeviceGray")));
            smask_dict.insert("BitsPerComponent", PdfObject::Integer(8));
            smask_dict.insert("Filter", PdfObject::Name(PdfName::new("FlateDecode")));
            smask_dict.insert("Length", PdfObject::Integer(alpha_bytes.len() as i64));

            doc.set_object(smask_id, PdfObject::Stream(PdfStream::new(smask_dict, alpha_bytes)));

            let stream_mut = doc.get_stream_mut(image_id)?;
            stream_mut.dict.insert("SMask", PdfObject::Reference(smask_id));
        } else {
            stream.dict.remove("SMask");
        }

        Ok(())
    } else {
        Err(PdfError::ParseError {
            offset: 0,
            message: "Unsupported image format: input must be a valid JPEG or PNG file".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_pdf_with_image() -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

        let off1 = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

        let off2 = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>\nendobj\n");

        let off3 = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 612 792 ] /Resources << /XObject << /Im1 5 0 R >> >> /Contents 4 0 R >>\nendobj\n",
        );

        let off4 = pdf.len();
        let stream_content = b"q\n200 0 0 100 50 600 cm\n/Im1 Do\nQ\n";
        pdf.extend_from_slice(
            format!("4 0 obj\n<< /Length {} >>\nstream\n", stream_content.len()).as_bytes(),
        );
        pdf.extend_from_slice(stream_content);
        pdf.extend_from_slice(b"endstream\nendobj\n");

        let off5 = pdf.len();
        // Minimal 2x2 RGB raw samples: 12 bytes
        let raw_pixels = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        pdf.extend_from_slice(
            format!(
                "5 0 obj\n<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
                raw_pixels.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&raw_pixels);
        pdf.extend_from_slice(b"endstream\nendobj\n");

        let xref_offset = pdf.len();
        pdf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off3).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off4).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off5).as_bytes());

        pdf.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\n");
        pdf.extend_from_slice(format!("startxref\n{}\n%%EOF\n", xref_offset).as_bytes());

        pdf
    }

    #[test]
    fn test_extract_and_replace_page_image_roundtrip() {
        let pdf_bytes = create_test_pdf_with_image();
        let mut doc = PdfDocument::load(&pdf_bytes).expect("Load PDF with image");
        let page_ids = doc.get_pages().expect("Resolve page IDs");
        assert_eq!(page_ids.len(), 1);

        // 1. Extract images from page
        let images = extract_page_images(&mut doc, page_ids[0]).expect("Extract page images");
        assert_eq!(images.len(), 1, "Should find exactly 1 image on the page");

        let img = &images[0];
        assert_eq!(img.resource_name, "Im1");
        assert_eq!(img.width_px, 2);
        assert_eq!(img.height_px, 2);
        assert_eq!(img.color_space, "DeviceRGB");

        // Verify exact placement bbox from 200 0 0 100 50 600 cm
        assert_eq!(img.bbox.min_x, 50.0);
        assert_eq!(img.bbox.min_y, 600.0);
        assert_eq!(img.bbox.max_x, 250.0);
        assert_eq!(img.bbox.max_y, 700.0);
        assert_eq!(img.bbox.width(), 200.0);
        assert_eq!(img.bbox.height(), 100.0);

        // 2. Fetch binary (should synthesize PNG for browser viewing)
        let (bin_bytes, mime) = get_image_binary(&mut doc, img.object_id).expect("Fetch image binary");
        assert_eq!(mime, "image/png");
        assert!(bin_bytes.starts_with(&png::PNG_SIGNATURE));

        // 3. Replace image with a new PNG logo (4x2 pixels)
        let new_png_pixels = vec![
            10, 20, 30,  40, 50, 60,  70, 80, 90,  100, 110, 120,
            130, 140, 150, 160, 170, 180, 190, 200, 210, 220, 230, 240,
        ];
        let replacement_png = encode_png(4, 2, &new_png_pixels, true).expect("Encode replacement PNG");

        replace_image_content(&mut doc, img.object_id, &replacement_png).expect("Replace image in-place");

        // 4. Re-extract and verify updated metadata
        let updated_images = extract_page_images(&mut doc, page_ids[0]).expect("Extract updated images");
        assert_eq!(updated_images.len(), 1);
        assert_eq!(updated_images[0].width_px, 4);
        assert_eq!(updated_images[0].height_px, 2);
        assert_eq!(updated_images[0].filter.as_deref(), Some("FlateDecode"));

        // Placement BBox remains 100% identical!
        assert_eq!(updated_images[0].bbox, img.bbox);

        // 5. Serialize PDF and ensure roundtrip validity
        let saved_bytes = doc.save_to_vec().expect("Save document to byte vector");
        assert!(saved_bytes.starts_with(b"%PDF-1.7\n"));
        assert!(saved_bytes.ends_with(b"%%EOF\n"));

        let mut roundtrip_doc = PdfDocument::load(&saved_bytes).expect("Reload modified PDF");
        let re_images = extract_page_images(&mut roundtrip_doc, page_ids[0]).expect("Extract reloaded images");
        assert_eq!(re_images.len(), 1);
        assert_eq!(re_images[0].width_px, 4);
        assert_eq!(re_images[0].height_px, 2);
    }
}

