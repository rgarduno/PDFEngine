//! Security policies, encryption, permissions, and digital signatures.
//! Conforming to ISO 32000-1 §7.6 (Security), §12.8 (Digital Signatures),
//! and resource hardening limits.

pub mod handler;
pub mod limits;
pub mod permissions;
pub mod signatures;

pub use handler::{
    authenticate_document, decrypt_document, encrypt_document, EncryptionOptions,
    EncryptionRevision,
};
pub use limits::SecurityLimits;
pub use permissions::PdfPermissions;
pub use signatures::{
    sign_document, verify_document_signatures, DigitalSignatureConfig, VerifiedSignature,
};
