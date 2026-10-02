//! ISO 32000-1 §12.7 Interactive Forms (AcroForms) and Form Flattening module.
//!
//! Provides inspection, filling, and permanent flattening of PDF interactive form fields.

pub mod filler;
pub mod flatten;
pub mod reader;
pub mod types;

pub use filler::{fill_field_value, fill_fields_batch};
pub use flatten::flatten_document_forms;
pub use reader::extract_document_forms;
pub use types::{FormField, FormFieldType};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::cos::PdfDocument;

    /// Constructs a valid minimal PDF with an AcroForm containing text, checkbox, and choice fields.
    fn create_test_acroform_pdf() -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

        // Object 1: Catalog
        let offset1 = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>\nendobj\n");

        // Object 2: Pages
        let offset2 = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

        // Object 3: Page with Annots [6 0 R, 7 0 R, 8 0 R]
        let offset3 = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [6 0 R 7 0 R 8 0 R] >>\nendobj\n",
        );

        // Object 4: Contents
        let offset4 = pdf.len();
        pdf.extend_from_slice(
            b"4 0 obj\n<< /Length 44 >>\nstream\nq\nBT /F1 12 Tf 72 700 Td (Sample Form) Tj ET\nQ\nendstream\nendobj\n",
        );

        // Object 5: AcroForm
        let offset5 = pdf.len();
        pdf.extend_from_slice(
            b"5 0 obj\n<< /Fields [6 0 R 7 0 R 8 0 R] /NeedAppearances true >>\nendobj\n",
        );

        // Object 6: Text Field (CustomerName)
        let offset6 = pdf.len();
        pdf.extend_from_slice(
            b"6 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Tx /T (CustomerName) /V (Initial Name) /Rect [72 650 300 670] /P 3 0 R >>\nendobj\n",
        );

        // Object 7: Checkbox Field (AcceptTerms)
        let offset7 = pdf.len();
        pdf.extend_from_slice(
            b"7 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Btn /T (AcceptTerms) /V /Off /AS /Off /Rect [72 620 92 640] /P 3 0 R >>\nendobj\n",
        );

        // Object 8: Choice Field (Country)
        let offset8 = pdf.len();
        pdf.extend_from_slice(
            b"8 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Ch /T (Country) /V (Mexico) /Opt [(USA) (Mexico) (Canada)] /Rect [72 580 200 600] /P 3 0 R >>\nendobj\n",
        );

        // Classic XRef table
        let xref_offset = pdf.len();
        pdf.extend_from_slice(b"xref\n0 9\n");
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        pdf.extend_from_slice(format!("{:010} 00000 n \n", offset1).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", offset2).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", offset3).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", offset4).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", offset5).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", offset6).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", offset7).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", offset8).as_bytes());

        // Trailer
        pdf.extend_from_slice(b"trailer\n<< /Size 9 /Root 1 0 R >>\nstartxref\n");
        pdf.extend_from_slice(format!("{}\n%%EOF\n", xref_offset).as_bytes());

        pdf
    }

    #[test]
    fn test_extract_acroform_fields() {
        let bytes = create_test_acroform_pdf();
        let mut doc = PdfDocument::load(&bytes).expect("Failed to load PDF");

        let fields = extract_document_forms(&mut doc).expect("Failed to extract forms");
        assert_eq!(fields.len(), 3);

        let text_field = fields.iter().find(|f| f.name == "CustomerName").expect("CustomerName not found");
        assert_eq!(text_field.field_type, FormFieldType::Text);
        assert_eq!(text_field.value, "Initial Name");
        assert_eq!(text_field.page_number, 1);
        assert_eq!(text_field.rect.min_x, 72.0);
        assert_eq!(text_field.rect.min_y, 650.0);

        let check_field = fields.iter().find(|f| f.name == "AcceptTerms").expect("AcceptTerms not found");
        assert_eq!(check_field.field_type, FormFieldType::Checkbox);
        assert!(!check_field.is_checked());

        let choice_field = fields.iter().find(|f| f.name == "Country").expect("Country not found");
        assert_eq!(choice_field.field_type, FormFieldType::Choice);
        assert_eq!(choice_field.value, "Mexico");
        assert_eq!(choice_field.options, vec!["USA", "Mexico", "Canada"]);
    }

    #[test]
    fn test_fill_and_flatten_acroform_roundtrip() {
        let bytes = create_test_acroform_pdf();
        let mut doc = PdfDocument::load(&bytes).expect("Failed to load PDF");

        // 1. Batch fill fields
        let mut values = HashMap::new();
        values.insert("CustomerName".to_string(), "Acme Corporation".to_string());
        values.insert("AcceptTerms".to_string(), "Yes".to_string());
        values.insert("Country".to_string(), "Canada".to_string());

        let updated_count = fill_fields_batch(&mut doc, &values).expect("Failed to batch fill");
        assert_eq!(updated_count, 3);

        // Verify values were updated in the AST
        let fields_after_fill = extract_document_forms(&mut doc).expect("Failed to re-extract forms");
        let name_field = fields_after_fill.iter().find(|f| f.name == "CustomerName").unwrap();
        assert_eq!(name_field.value, "Acme Corporation");

        let terms_field = fields_after_fill.iter().find(|f| f.name == "AcceptTerms").unwrap();
        assert!(terms_field.is_checked());

        // 2. Flatten document forms
        let flattened_count = flatten_document_forms(&mut doc).expect("Failed to flatten forms");
        assert_eq!(flattened_count, 3);

        // Verify /AcroForm is gone from catalog
        let catalog = doc.catalog().expect("Catalog not found");
        assert!(!catalog.contains_key("AcroForm"));

        // Verify /Annots is stripped or empty on the page
        let page_ids = doc.get_pages().expect("Failed to get pages");
        let page_obj = doc.get_object(page_ids[0]).expect("Page object not found");
        let page_dict = page_obj.as_dict().expect("Page not a dict");
        assert!(!page_dict.contains_key("Annots"));

        // Verify page contents now contains the burned text
        let content_bytes = doc.get_page_content_bytes(page_ids[0]).expect("Failed to get page content");
        let content_str = String::from_utf8_lossy(&content_bytes);
        assert!(content_str.contains("Acme Corporation"));
        assert!(content_str.contains("Flattened AcroForm Fields"));
    }
}
