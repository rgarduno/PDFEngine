//! ISO 32000-1 §12.7 Interactive Forms (AcroForms) and Form Flattening module.
//!
//! Provides inspection, filling, and permanent flattening of PDF interactive form fields.

pub mod builder;
pub mod filler;
pub mod flatten;
pub mod reader;
pub mod types;

pub use builder::{
    create_form_field, delete_form_field, get_or_create_acroform, update_form_field,
    FormFieldCreateOptions, FormFieldUpdateOptions,
};
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

    #[test]
    fn test_create_and_delete_form_fields() {
        use crate::layout::geometry::Rect;

        // Create a blank PDF without any AcroForm
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        let off1 = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let off2 = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        let off3 = pdf.len();
        pdf.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n");
        let off4 = pdf.len();
        pdf.extend_from_slice(b"4 0 obj\n<< /Length 12 >>\nstream\nq\n(Blank) Tj\nQ\nendstream\nendobj\n");

        let xref_off = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off3).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off4).as_bytes());
        pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n");
        pdf.extend_from_slice(format!("{}\n%%EOF\n", xref_off).as_bytes());

        let mut doc = PdfDocument::load(&pdf).expect("Failed to load PDF");

        // 1. Create a Text field
        let text_opts = FormFieldCreateOptions {
            name: "UserEmail".to_string(),
            field_type: FormFieldType::Text,
            rect: Rect::new(72.0, 700.0, 300.0, 724.0),
            value: Some("dev@example.com".to_string()),
            default_value: Some("placeholder@test.com".to_string()),
            alt_name: Some("Corporate Email".to_string()),
            options: None,
            is_read_only: false,
            is_required: true,
            is_multiline: false,
            max_length: Some(100),
            font_size: Some(11.0),
        };
        let created_text = create_form_field(&mut doc, 1, &text_opts).expect("Failed to create text field");
        assert_eq!(created_text.name, "UserEmail");
        assert_eq!(created_text.value, "dev@example.com");
        assert!(created_text.is_required);

        // 2. Create a Checkbox field
        let check_opts = FormFieldCreateOptions {
            name: "OptInNewsletter".to_string(),
            field_type: FormFieldType::Checkbox,
            rect: Rect::new(72.0, 660.0, 92.0, 680.0),
            value: Some("Yes".to_string()),
            default_value: None,
            alt_name: Some("Subscribe to newsletter".to_string()),
            options: None,
            is_read_only: false,
            is_required: false,
            is_multiline: false,
            max_length: None,
            font_size: None,
        };
        let created_check = create_form_field(&mut doc, 1, &check_opts).expect("Failed to create checkbox");
        assert!(created_check.is_checked());

        // 3. Create a Choice dropdown field
        let choice_opts = FormFieldCreateOptions {
            name: "Department".to_string(),
            field_type: FormFieldType::Choice,
            rect: Rect::new(72.0, 620.0, 250.0, 644.0),
            value: Some("Engineering".to_string()),
            default_value: None,
            alt_name: Some("Work department".to_string()),
            options: Some(vec!["Sales".to_string(), "Engineering".to_string(), "Legal".to_string()]),
            is_read_only: false,
            is_required: false,
            is_multiline: false,
            max_length: None,
            font_size: Some(10.0),
        };
        let created_choice = create_form_field(&mut doc, 1, &choice_opts).expect("Failed to create choice field");
        assert_eq!(created_choice.options.len(), 3);
        assert_eq!(created_choice.value, "Engineering");

        // 4. Verify all 3 fields exist in document
        let all_fields = extract_document_forms(&mut doc).expect("Failed to extract forms");
        assert_eq!(all_fields.len(), 3);

        // 5. Update field properties
        let update_opts = FormFieldUpdateOptions {
            rect: Some(Rect::new(80.0, 705.0, 320.0, 730.0)),
            alt_name: Some("Updated Corporate Email".to_string()),
            is_read_only: Some(true),
            is_required: Some(false),
            is_multiline: None,
        };
        let updated = update_form_field(&mut doc, "UserEmail", &update_opts)
            .expect("Failed to update field")
            .expect("Field not returned");
        assert_eq!(updated.rect.min_x, 80.0);
        assert_eq!(updated.alt_name.as_deref(), Some("Updated Corporate Email"));
        assert!(updated.is_read_only);
        assert!(!updated.is_required);

        // 6. Delete a field
        let deleted = delete_form_field(&mut doc, "OptInNewsletter").expect("Failed to delete field");
        assert!(deleted);

        // Verify count dropped to 2
        let fields_remaining = extract_document_forms(&mut doc).expect("Failed to extract forms");
        assert_eq!(fields_remaining.len(), 2);
        assert!(!fields_remaining.iter().any(|f| f.name == "OptInNewsletter"));
        assert!(fields_remaining.iter().any(|f| f.name == "UserEmail"));
        assert!(fields_remaining.iter().any(|f| f.name == "Department"));
    }
}

