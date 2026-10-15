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
//! After a user password opens the file, `/P` blocks print, copy, and edit in this
//! process. The owner password is unrestricted. Bit 12 is stored and is not a second
//! raster. Bit 10 is not a separate extract path. Saving the decrypted file drops `/P`.

pub mod active;
pub mod cms;
pub mod handler;
pub mod limits;
pub mod permissions;
pub mod signatures;

pub use cms::{cms_timestamp_request, embed_cms_timestamp, sign_document_cms, CmsSigningMaterial};
pub use handler::{
    authenticate_document, compute_hash_rev5, compute_hash_rev6, decrypt_document,
    encrypt_document, get_encryption_revision, prepare_utf8_password, EncryptionOptions,
    EncryptionRevision,
};
pub use limits::SecurityLimits;
pub use permissions::PdfPermissions;
pub use signatures::{
    sign_document, verify_document_signatures, DigitalSignatureConfig, VerifiedSignature,
};
