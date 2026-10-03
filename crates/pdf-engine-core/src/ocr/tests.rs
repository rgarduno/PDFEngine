use crate::cos::{ObjectId, PdfDocument};
use crate::fonts::ToUnicodeMap;
use crate::images::{extract_page_images, parse_png_pixels};
use crate::layout::geometry::Rect;
use crate::security::SecurityLimits;
use crate::stream::Matrix;

use super::layer::map_word;
use super::recognize::{accept_language, parse_tsv, OcrWord};
use super::add_searchable_text_layer;

#[test]
fn tsv_keeps_word_rows_and_drops_blocks() {
    let tsv = b"level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
1\t1\t0\t0\t0\t0\t0\t0\t100\t20\t-1\t\n\
5\t1\t1\t1\t1\t1\t10\t4\t40\t12\t90.5\tHELLO\n\
5\t1\t1\t1\t1\t2\t-1\t4\t10\t12\t80\tNO\n";
    let words = parse_tsv(tsv);
    assert_eq!(words.len(), 1);
    assert_eq!(words[0].text, "HELLO");
    assert_eq!(words[0].left, 10);
    assert!((words[0].confidence - 90.5).abs() < 0.01);
}

#[test]
fn language_names_are_a_traineddata_stem() {
    assert!(accept_language("eng").is_ok());
    assert!(accept_language("chi_sim").is_ok());
    assert!(accept_language("-psm").is_err());
    assert!(accept_language("eng;id").is_err());
    assert!(accept_language("e").is_err());
}

#[test]
fn word_box_maps_through_the_image_matrix() {
    let image = crate::images::ImageInfo {
        object_id: ObjectId::new(5),
        resource_name: "Im1".to_string(),
        width_px: 1000,
        height_px: 240,
        color_space: "DeviceRGB".to_string(),
        bits_per_component: 8,
        filter: None,
        byte_size: 0,
        bbox: Rect::new(0.0, 0.0, 612.0, 792.0),
        ctm: Matrix {
            a: 612.0,
            b: 0.0,
            c: 0.0,
            d: 792.0,
            e: 0.0,
            f: 0.0,
        },
    };
    let word = OcrWord {
        text: "HELLO".to_string(),
        left: 67,
        top: 73,
        width: 438,
        height: 92,
        confidence: 94.0,
    };
    let mapped = map_word(&image, &word).expect("mapped word");
    assert_eq!(mapped.bytes, b"HELLO");
    let font_size = 792.0 * (92.0 / 240.0);
    assert!((mapped.font_size - font_size).abs() < 0.01);
    let origin_y = 792.0 * (1.0 - 165.0 / 240.0);
    assert!((mapped.text_matrix[5] - origin_y).abs() < 0.01);
    assert!((mapped.text_matrix[4] - 612.0 * 0.067).abs() < 0.01);
    assert!((mapped.text_matrix[0] - 1.0).abs() < 0.001);
    assert!(mapped.horizontal_scale > 1.0 && mapped.horizontal_scale < 800.0);
}

#[test]
fn scanned_page_gains_invisible_text_and_a_text_page_does_not() {
    let png = include_bytes!("fixtures/scan-hello.png");
    let (width, height, pixels, _) =
        parse_png_pixels(png, &SecurityLimits::default()).expect("fixture png");
    let mut scan = PdfDocument::load(&image_pdf(&pixels, width, height, false)).expect("scan pdf");
    let page_ids = scan.get_pages().expect("pages");
    let before_images = extract_page_images(&mut scan, page_ids[0]).expect("images");
    assert_eq!(before_images.len(), 1);

    let report = add_searchable_text_layer(&mut scan, Some("eng")).expect("recognize");
    assert_eq!(report.pages_seen, 1);
    assert_eq!(report.pages_recognized, 1);
    assert!(report.words_inserted >= 1, "tesseract found no words");

    let saved = scan.save_to_vec().expect("save");
    assert!(saved.windows(4).any(|window| window == b"3 Tr"));
    assert!(saved.windows(7).any(|window| window == b"(HELLO)"));
    assert!(saved.windows(9).any(|window| window == b"ToUnicode"));
    let cmap_at = saved
        .windows(11)
        .position(|window| window == b"beginbfchar")
        .expect("cmap");
    let cmap = &saved[cmap_at.saturating_sub(64)..];
    let parsed = ToUnicodeMap::parse(cmap).expect("parse cmap");
    let decoded = parsed.decode_bytes(b"HELLO", 1);
    assert_eq!(decoded, "HELLO");

    let mut reloaded = PdfDocument::load(&saved).expect("reload");
    let images = extract_page_images(&mut reloaded, page_ids[0]).expect("images after");
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].width_px, width);

    let again = add_searchable_text_layer(&mut reloaded, None).expect("second pass");
    assert_eq!(again.pages_seen, 0);
    assert_eq!(again.words_inserted, 0);

    let mut textual = PdfDocument::load(&image_pdf(&pixels, width, height, true)).expect("text pdf");
    let text_pages = textual.get_pages().expect("pages");
    let before = textual
        .get_page_content_bytes(text_pages[0])
        .expect("content");
    let skipped = add_searchable_text_layer(&mut textual, Some("eng")).expect("skip text");
    assert_eq!(skipped.pages_seen, 0);
    assert_eq!(skipped.words_inserted, 0);
    let after = textual
        .get_page_content_bytes(text_pages[0])
        .expect("content after");
    assert_eq!(before, after);
}

fn image_pdf(pixels: &[u8], width: u32, height: u32, with_text: bool) -> Vec<u8> {
    let content = if with_text {
        format!("q\n612 0 0 792 0 0 cm\n/Im1 Do\nQ\nBT\n/F1 12 Tf\n(Already) Tj\nET\n")
    } else {
        "q\n612 0 0 792 0 0 cm\n/Im1 Do\nQ\n".to_string()
    };
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
    let offsets = [
        write_obj(&mut pdf, b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n"),
        write_obj(&mut pdf, b"2 0 obj\n<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>\nendobj\n"),
        write_obj(
            &mut pdf,
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 612 792 ] /Resources << /XObject << /Im1 5 0 R >> >> /Contents 4 0 R >>\nendobj\n",
        ),
        write_stream(&mut pdf, content.as_bytes()),
        write_image(&mut pdf, pixels, width, height),
    ];
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\n");
    pdf.extend_from_slice(format!("startxref\n{xref}\n%%EOF\n").as_bytes());
    pdf
}

fn write_obj(pdf: &mut Vec<u8>, bytes: &[u8]) -> usize {
    let offset = pdf.len();
    pdf.extend_from_slice(bytes);
    offset
}

fn write_stream(pdf: &mut Vec<u8>, content: &[u8]) -> usize {
    let offset = pdf.len();
    pdf.extend_from_slice(
        format!("4 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes(),
    );
    pdf.extend_from_slice(content);
    pdf.extend_from_slice(b"endstream\nendobj\n");
    offset
}

fn write_image(pdf: &mut Vec<u8>, pixels: &[u8], width: u32, height: u32) -> usize {
    let offset = pdf.len();
    pdf.extend_from_slice(
        format!(
            "5 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
            pixels.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(pixels);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
    offset
}
