//! Security policies, encryption, permissions, and SHA-256 byte-range attestations.
//! Encryption follows ISO 32000-1 §7.6. Attestations reuse the `/Sig` and `/ByteRange`
//! dictionaries from §12.8 to record a file digest. They are not CMS signatures.

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
