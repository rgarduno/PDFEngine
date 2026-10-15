//! Permission bits stored in the encryption dictionary `/P` entry.
//!
//! The flags follow the bit positions in ISO 32000-1 §7.6.3.2 Table 22.
//! After a user password opens the file, this process refuses print, copy, and
//! edit when the stored bits disallow them. The owner password is unrestricted.
//! Bit 12 is stored and is not a second raster. Bit 10 is not a separate extract
//! path. Saving the decrypted file drops `/P`.

/// Client-facing sentence when a user-password session may not print, copy, or edit.
pub const PERMISSION_DENIED: &str =
    "The stored document permissions do not allow this operation.";

/// Bits written to `/P` and consulted after a user password opens the file.
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
    /// All operational bits set. A user password with these bits is not restricted.
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
    /// Print stays allowed. Copy, modification, annotations, filling, and assembly are clear.
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

    /// Filling an existing field stays allowed. Creating a field does not.
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

    /// Print, copy, modify, annotate, fill, and assemble are all set.
    ///
    /// Bit 10 and bit 12 are ignored. This process has one raster, and ISO 32000-2
    /// says a processor should ignore the accessibility bit.
    pub fn allows_all_enforced(self) -> bool {
        self.print_low_res
            && self.modify_contents
            && self.copy_extract
            && self.modify_annotations
            && self.fill_forms
            && self.assemble_document
    }

    /// Bit 3. Bit 12 does not select a second raster and does not block this one.
    pub fn allows_print(self) -> bool {
        self.print_low_res
    }

    /// Bit 5. Bit 10 does not enable or block extraction.
    pub fn allows_copy(self) -> bool {
        self.copy_extract
    }

    /// Bit 4.
    pub fn allows_modify(self) -> bool {
        self.modify_contents
    }

    /// Bit 6.
    pub fn allows_annotate(self) -> bool {
        self.modify_annotations
    }

    /// Bit 9, or bit 6 when annotations may be modified.
    pub fn allows_fill(self) -> bool {
        self.fill_forms || self.modify_annotations
    }

    /// Bits 4 and 6. Creating a field or burning annotations needs both.
    pub fn allows_content_and_annotations(self) -> bool {
        self.modify_contents && self.modify_annotations
    }

    /// Bit 11, or bit 4 when page contents may be modified.
    pub fn allows_assemble(self) -> bool {
        self.assemble_document || self.modify_contents
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

    #[test]
    fn enforced_bits_ignore_accessibility_and_high_resolution_print() {
        let mut perms = PdfPermissions::default();
        perms.accessibility_extract = false;
        perms.print_high_res = false;
        assert!(perms.allows_all_enforced());
        assert!(perms.allows_print());
        perms.copy_extract = false;
        assert!(!perms.allows_all_enforced());
        assert!(!perms.allows_copy());
        assert!(PdfPermissions::read_only().allows_print());
        assert!(!PdfPermissions::read_only().allows_modify());
        assert!(PdfPermissions::forms_only().allows_fill());
        assert!(!PdfPermissions::forms_only().allows_content_and_annotations());
    }
}
