//! Permission bits stored in the encryption dictionary `/P` entry.
//!
//! The flags follow the bit positions in ISO 32000-1 §7.6.3.2 Table 22.
//! This process writes and reads the integer. It does not block printing,
//! copying, modification, or form filling.

/// Bits written to `/P`. They are not enforced by this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfPermissions {
    /// Bit 3: Print the document (possibly at low resolution).
    pub print_low_res: bool,
    /// Bit 4: Modify contents of the document other than annotations and forms.
    pub modify_contents: bool,
    /// Bit 5: Copy or extract text and graphics from the document.
    pub copy_extract: bool,
    /// Bit 6: Add or modify text annotations and fill in form fields.
    pub modify_annotations: bool,
    /// Bit 9: Fill in existing interactive form fields (even if modify_annotations is false).
    pub fill_forms: bool,
    /// Bit 10: Extract text and graphics for accessibility tools.
    pub accessibility_extract: bool,
    /// Bit 11: Assemble document (insert, rotate, or delete pages).
    pub assemble_document: bool,
    /// Bit 12: High-fidelity printing.
    pub print_high_res: bool,
}

impl Default for PdfPermissions {
    /// All bits set. The stored value does not grant or deny an action in this process.
    fn default() -> Self {
        Self {
            print_low_res: true,
            modify_contents: true,
            copy_extract: true,
            modify_annotations: true,
            fill_forms: true,
            accessibility_extract: true,
            assemble_document: true,
            print_high_res: true,
        }
    }
}

impl PdfPermissions {
    /// Stored bitmask with print and accessibility bits set and the modification bits clear.
    /// This process does not consult the bitmask before an edit.
    pub fn read_only() -> Self {
        Self {
            print_low_res: true,
            modify_contents: false,
            copy_extract: false,
            modify_annotations: false,
            fill_forms: false,
            accessibility_extract: true,
            assemble_document: false,
            print_high_res: true,
        }
    }

    /// Stored bitmask with the form-filling bit set and the other modification bits clear.
    /// This process does not consult the bitmask before an edit.
    pub fn forms_only() -> Self {
        Self {
            print_low_res: true,
            modify_contents: false,
            copy_extract: false,
            modify_annotations: false,
            fill_forms: true,
            accessibility_extract: true,
            assemble_document: false,
            print_high_res: true,
        }
    }

    /// Converts these permissions into the 32-bit signed integer required by the PDF `/P` entry
    /// in the `/Encrypt` dictionary per ISO 32000-1 §7.6.3.2 Table 22.
    pub fn to_p_value(&self) -> i32 {
        // Reserved bits 7, 8, and 13..32 MUST be 1.
        // 0xFFFFF0C0 has bits 7, 8 and 13..31 set.
        let mut flags: u32 = 0xFFFFF0C0;

        if self.print_low_res {
            flags |= 1 << 2; // Bit 3
        }
        if self.modify_contents {
            flags |= 1 << 3; // Bit 4
        }
        if self.copy_extract {
            flags |= 1 << 4; // Bit 5
        }
        if self.modify_annotations {
            flags |= 1 << 5; // Bit 6
        }
        if self.fill_forms {
            flags |= 1 << 8; // Bit 9
        }
        if self.accessibility_extract {
            flags |= 1 << 9; // Bit 10
        }
        if self.assemble_document {
            flags |= 1 << 10; // Bit 11
        }
        if self.print_high_res {
            flags |= 1 << 11; // Bit 12
        }

        flags as i32
    }

    /// Parses permissions from a 32-bit signed integer `/P` entry.
    pub fn from_p_value(p: i32) -> Self {
        let flags = p as u32;
        Self {
            print_low_res: (flags & (1 << 2)) != 0,
            modify_contents: (flags & (1 << 3)) != 0,
            copy_extract: (flags & (1 << 4)) != 0,
            modify_annotations: (flags & (1 << 5)) != 0,
            fill_forms: (flags & (1 << 8)) != 0,
            accessibility_extract: (flags & (1 << 9)) != 0,
            assemble_document: (flags & (1 << 10)) != 0,
            print_high_res: (flags & (1 << 11)) != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permissions_roundtrip() {
        let default_perm = PdfPermissions::default();
        let p_val = default_perm.to_p_value();
        // Unrestricted permissions equals -4 (0xFFFFFFFC)
        assert_eq!(p_val, -4);
        let parsed = PdfPermissions::from_p_value(p_val);
        assert_eq!(parsed, default_perm);

        let read_only = PdfPermissions::read_only();
        let ro_val = read_only.to_p_value();
        let parsed_ro = PdfPermissions::from_p_value(ro_val);
        assert_eq!(parsed_ro, read_only);
        assert!(!parsed_ro.modify_contents);
        assert!(!parsed_ro.copy_extract);
        assert!(parsed_ro.print_low_res);
    }
}
