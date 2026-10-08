//! Security policies, encryption, permissions, active-content removal, SHA-256
//! byte-range attestations, and PKCS#7 detached signatures.
//! Encryption follows ISO 32000-1 §7.6. Attestations reuse the `/Sig` and `/ByteRange`
//! dictionaries from §12.8 to record a file digest. They are not CMS signatures.
//! A PKCS#7 signature is produced only when a certificate and private key are supplied.
//! Verification checks that signature against the certificate embedded in the CMS.
//! It does not decide whether a viewer trusts the certificate.
//! Saving a document removes `/JavaScript`, `/JS`, `/Launch`, `/SubmitForm`,
//! `/OpenAction`, and `/AA`. A `/URI` action is kept unless its scheme is
//! `javascript`, `vbscript`, `file`, or `data`.
//! The `/P` bitmask is stored and is not enforced by this process.

pub mod active;
pub mod cms;
pub mod handler;
pub mod limits;
pub mod permissions;
pub mod signatures;

pub use cms::{cms_timestamp_request, embed_cms_timestamp, sign_document_cms, CmsSigningMaterial};
pub use handler::{
    authenticate_document, decrypt_document, encrypt_document, EncryptionOptions,
    EncryptionRevision,
};
pub use limits::SecurityLimits;
pub use permissions::PdfPermissions;
pub use signatures::{
    sign_document, verify_document_signatures, DigitalSignatureConfig, VerifiedSignature,
};
