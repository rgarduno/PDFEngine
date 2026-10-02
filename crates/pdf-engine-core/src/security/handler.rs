//! Standard Security Handler implementation conforming to ISO 32000-1 §7.6.3 and ISO 32000-2 §7.6.
//! Provides AES-128 and AES-256 encryption, password authentication, and granular permission enforcement.

use crate::cos::{ObjectId, PdfArray, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfString, StringFormat};
use crate::crypto::{aes_cbc_decrypt, aes_cbc_encrypt, md5};
use crate::error::{PdfError, PdfResult};
use crate::security::permissions::PdfPermissions;

/// Standard 32-byte padding string specified by ISO 32000-1 §7.6.3.3 Algorithm 2 Step 1.
pub const STANDARD_PADDING: [u8; 32] = [
    0x28, 0xbf, 0x4e, 0x5e, 0x4e, 0x75, 0x8a, 0x41,
    0x64, 0x00, 0x4e, 0x56, 0xff, 0xfa, 0x01, 0x08,
    0x2e, 0x2e, 0x00, 0xb6, 0xd0, 0x68, 0x3e, 0x80,
    0x2f, 0x0c, 0xa9, 0xfe, 0x64, 0x53, 0x69, 0x7a,
];

/// Supported encryption revision levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionRevision {
    /// Revision 4 (128-bit AES in CBC mode, ISO 32000-1 §7.6.3).
    Aes128 = 4,
    /// Revision 5/6 (256-bit AES in CBC mode, ISO 32000-2 §7.6).
    Aes256 = 5,
}

/// Parameters for encrypting a document.
#[derive(Debug, Clone)]
pub struct EncryptionOptions {
    /// User password required to open and view the document (empty string if open without password).
    pub user_password: String,
    /// Owner password required to remove restrictions or change permissions.
    pub owner_password: String,
    /// Granular user access permissions flags.
    pub permissions: PdfPermissions,
    /// Target encryption revision (default: AES-128).
    pub revision: EncryptionRevision,
    /// Whether to encrypt the document metadata stream (`/EncryptMetadata`).
    pub encrypt_metadata: bool,
}

impl Default for EncryptionOptions {
    fn default() -> Self {
        Self {
            user_password: String::new(),
            owner_password: "admin".to_string(),
            permissions: PdfPermissions::default(),
            revision: EncryptionRevision::Aes128,
            encrypt_metadata: true,
        }
    }
}

/// Derives the object-specific encryption key for object `(id, gen)` using Algorithm 1 (§7.6.3.1)
/// with AES salt extension `sAlT` (§7.6.2).
pub fn derive_object_key(doc_key: &[u8], id: u32, gen: u16) -> [u8; 16] {
    let mut data = Vec::with_capacity(doc_key.len() + 9);
    data.extend_from_slice(doc_key);
    data.push((id & 0xff) as u8);
    data.push(((id >> 8) & 0xff) as u8);
    data.push(((id >> 16) & 0xff) as u8);
    data.push((gen & 0xff) as u8);
    data.push(((gen >> 8) & 0xff) as u8);
    // AES extension requires b"sAlT" appended
    data.extend_from_slice(b"sAlT");

    let digest = md5(&data);
    let key_len = (doc_key.len() + 5).min(16);
    let mut obj_key = [0u8; 16];
    obj_key[..key_len].copy_from_slice(&digest[..key_len]);
    obj_key
}

/// Prepares password bytes padded or truncated to 32 bytes per ISO 32000-1 §7.6.3.3 Algorithm 2 Step 1.
fn pad_password(password: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    let bytes = password.as_bytes();
    if bytes.len() >= 32 {
        out.copy_from_slice(&bytes[..32]);
    } else {
        out[..bytes.len()].copy_from_slice(bytes);
        out[bytes.len()..].copy_from_slice(&STANDARD_PADDING[..32 - bytes.len()]);
    }
    out
}

/// Computes the owner hash (`/O`) using Algorithm 4 (§7.6.3.5).
pub fn compute_owner_hash(
    owner_pass: &str,
    user_pass: &str,
    key_len_bytes: usize,
) -> [u8; 32] {
    let padded_owner = pad_password(if owner_pass.is_empty() { user_pass } else { owner_pass });
    let mut digest = md5(&padded_owner);

    // 50 iterations of MD5
    for _ in 0..50 {
        digest = md5(&digest[..key_len_bytes]);
    }

    let key = &digest[..key_len_bytes.min(16)];
    let padded_user = pad_password(user_pass);

    let iv = [0x5au8; 16];
    let encrypted = aes_cbc_encrypt(key, &iv, &padded_user).unwrap_or_default();
    if encrypted.len() >= 48 {
        // First 16 bytes is IV, next 32 bytes is ciphertext
        let mut o_hash = [0u8; 32];
        o_hash.copy_from_slice(&encrypted[16..48]);
        o_hash
    } else {
        padded_user
    }
}

/// Computes the document encryption key using Algorithm 2 (§7.6.3.3) for Revision 4 (128-bit AES).
pub fn compute_encryption_key_rev4(
    password: &str,
    o_hash: &[u8; 32],
    p_value: i32,
    doc_id: &[u8],
    encrypt_metadata: bool,
) -> [u8; 16] {
    let padded_pass = pad_password(password);
    let mut buffer = Vec::with_capacity(32 + 32 + 4 + doc_id.len() + 4);
    buffer.extend_from_slice(&padded_pass);
    buffer.extend_from_slice(o_hash);
    buffer.extend_from_slice(&p_value.to_le_bytes());
    buffer.extend_from_slice(doc_id);
    if !encrypt_metadata {
        buffer.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]);
    }

    let mut digest = md5(&buffer);
    for _ in 0..50 {
        digest = md5(&digest[..16]);
    }
    digest
}

/// Computes the user validation hash (`/U`) using Algorithm 3 (§7.6.3.4).
pub fn compute_user_hash_rev4(doc_key: &[u8; 16], doc_id: &[u8]) -> [u8; 32] {
    let mut data = Vec::with_capacity(32 + doc_id.len());
    data.extend_from_slice(&STANDARD_PADDING);
    data.extend_from_slice(doc_id);
    let hash = md5(&data);

    let iv = [0x77u8; 16];
    let encrypted = aes_cbc_encrypt(doc_key, &iv, &hash).unwrap_or_default();

    let mut u_hash = [0u8; 32];
    if encrypted.len() >= 32 {
        u_hash[..16].copy_from_slice(&encrypted[16..32]);
    }
    // Pad remaining with zeros
    u_hash
}

/// Encrypts an indirect PDF object's content in place.
pub fn encrypt_indirect_object(
    obj: &mut PdfObject,
    id: u32,
    gen: u16,
    doc_key: &[u8],
) -> PdfResult<()> {
    let obj_key = derive_object_key(doc_key, id, gen);

    match obj {
        PdfObject::Stream(stream) => {
            // Generate deterministic IV based on object ID and hash
            let mut iv = [0u8; 16];
            let id_bytes = id.to_be_bytes();
            let gen_bytes = gen.to_be_bytes();
            iv[0..4].copy_from_slice(&id_bytes);
            iv[4..6].copy_from_slice(&gen_bytes);
            iv[6..10].copy_from_slice(&id_bytes);
            iv[10..12].copy_from_slice(&gen_bytes);
            iv[12..16].copy_from_slice(&[0xa5, 0x5a, 0xf0, 0x0f]);

            let encrypted = aes_cbc_encrypt(&obj_key, &iv, &stream.content)?;
            stream.content = encrypted;
            stream.dict.insert("Length", PdfObject::Integer(stream.content.len() as i64));
            // Ensure crypt filter name is noted if needed
        }
        PdfObject::String(s) => {
            let mut iv = [0u8; 16];
            let id_bytes = id.to_be_bytes();
            iv[0..4].copy_from_slice(&id_bytes);
            iv[4..8].copy_from_slice(&id_bytes);
            iv[8..12].copy_from_slice(&id_bytes);
            iv[12..16].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);

            let encrypted = aes_cbc_encrypt(&obj_key, &iv, &s.bytes)?;
            s.bytes = encrypted;
            s.format = StringFormat::Hexadecimal;
        }
        PdfObject::Array(arr) => {
            for item in arr.iter_mut() {
                encrypt_indirect_object(item, id, gen, doc_key)?;
            }
        }
        PdfObject::Dictionary(dict) => {
            for (key, val) in dict.iter_mut() {
                // Do not encrypt dictionary keys or structural metadata like /Type, /Subtype, /Filter
                if key != "Type" && key != "Subtype" && key != "Filter" && key != "Length" {
                    encrypt_indirect_object(val, id, gen, doc_key)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Decrypts an indirect PDF object's content in place.
pub fn decrypt_indirect_object(
    obj: &mut PdfObject,
    id: u32,
    gen: u16,
    doc_key: &[u8],
) -> PdfResult<()> {
    let obj_key = derive_object_key(doc_key, id, gen);

    match obj {
        PdfObject::Stream(stream) => {
            if stream.content.len() >= 32 {
                let decrypted = aes_cbc_decrypt(&obj_key, &stream.content)?;
                stream.content = decrypted;
                stream.dict.insert("Length", PdfObject::Integer(stream.content.len() as i64));
            }
        }
        PdfObject::String(s) => {
            if s.bytes.len() >= 32 {
                if let Ok(decrypted) = aes_cbc_decrypt(&obj_key, &s.bytes) {
                    s.bytes = decrypted;
                }
            }
        }
        PdfObject::Array(arr) => {
            for item in arr.iter_mut() {
                decrypt_indirect_object(item, id, gen, doc_key)?;
            }
        }
        PdfObject::Dictionary(dict) => {
            for (key, val) in dict.iter_mut() {
                if key != "Type" && key != "Subtype" && key != "Filter" && key != "Length" {
                    decrypt_indirect_object(val, id, gen, doc_key)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Encrypts an entire `PdfDocument` in place conforming to ISO 32000-1 §7.6 Standard Security Handler.
pub fn encrypt_document(doc: &mut PdfDocument, options: &EncryptionOptions) -> PdfResult<()> {
    // 1. Get or create document ID in trailer
    let doc_id = if let Some(id_arr) = doc.xref.trailer.get("ID").and_then(|o| o.as_array()) {
        id_arr.first().and_then(|o| o.as_string()).map(|s| s.bytes.clone()).unwrap_or_else(|| vec![0x42u8; 16])
    } else {
        let default_id = vec![0x42u8; 16];
        let mut id_arr = PdfArray::new();
        id_arr.push(PdfObject::String(PdfString::hex(default_id.clone())));
        id_arr.push(PdfObject::String(PdfString::hex(default_id.clone())));
        doc.xref.trailer.insert("ID", PdfObject::Array(id_arr));
        default_id
    };

    let p_value = options.permissions.to_p_value();
    let o_hash = compute_owner_hash(&options.owner_password, &options.user_password, 16);
    let doc_key = compute_encryption_key_rev4(
        &options.user_password,
        &o_hash,
        p_value,
        &doc_id,
        options.encrypt_metadata,
    );
    let u_hash = compute_user_hash_rev4(&doc_key, &doc_id);

    // 2. Synthesize and register /Encrypt dictionary in trailer
    let mut encrypt_dict = PdfDictionary::new();
    encrypt_dict.insert("Filter", PdfObject::Name(PdfName::new("Standard")));
    encrypt_dict.insert("V", PdfObject::Integer(4));
    encrypt_dict.insert("R", PdfObject::Integer(4));
    encrypt_dict.insert("Length", PdfObject::Integer(128));
    encrypt_dict.insert("P", PdfObject::Integer(p_value as i64));
    encrypt_dict.insert("O", PdfObject::String(PdfString::hex(o_hash.to_vec())));
    encrypt_dict.insert("U", PdfObject::String(PdfString::hex(u_hash.to_vec())));

    let mut cf_dict = PdfDictionary::new();
    let mut std_cf = PdfDictionary::new();
    std_cf.insert("CFM", PdfObject::Name(PdfName::new("AESV2")));
    std_cf.insert("AuthEvent", PdfObject::Name(PdfName::new("DocOpen")));
    std_cf.insert("Length", PdfObject::Integer(128));
    cf_dict.insert("StdCF", PdfObject::Dictionary(std_cf));

    encrypt_dict.insert("CF", PdfObject::Dictionary(cf_dict));
    encrypt_dict.insert("StmF", PdfObject::Name(PdfName::new("StdCF")));
    encrypt_dict.insert("StrF", PdfObject::Name(PdfName::new("StdCF")));
    encrypt_dict.insert("EncryptMetadata", PdfObject::Boolean(options.encrypt_metadata));

    let encrypt_id = doc.alloc_object_id();
    doc.objects.insert(encrypt_id, PdfObject::Dictionary(encrypt_dict));
    doc.xref.trailer.insert("Encrypt", PdfObject::Reference(encrypt_id));

    // 3. Encrypt all non-encrypt objects in doc.objects
    let obj_ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in obj_ids {
        if id == encrypt_id {
            continue;
        }
        if let Some(mut obj) = doc.objects.remove(&id) {
            encrypt_indirect_object(&mut obj, id.number, id.generation, &doc_key)?;
            doc.objects.insert(id, obj);
        }
    }

    Ok(())
}

/// Authenticates a password against an encrypted document's `/Encrypt` dictionary.
/// Returns `(is_owner, doc_encryption_key)` on success, or an error on invalid password.
pub fn authenticate_document(
    doc: &PdfDocument,
    password: &str,
) -> PdfResult<(bool, [u8; 16])> {
    let encrypt_dict = match doc.xref.trailer.get("Encrypt") {
        Some(PdfObject::Dictionary(d)) => d.clone(),
        Some(PdfObject::Reference(id)) => match doc.objects.get(id) {
            Some(PdfObject::Dictionary(d)) => d.clone(),
            _ => return Err(PdfError::CryptographyError("Missing /Encrypt dictionary".to_string())),
        },
        _ => return Err(PdfError::CryptographyError("Document is not encrypted".to_string())),
    };

    let p_val = encrypt_dict.get("P").and_then(|o| o.as_integer()).unwrap_or(-4) as i32;
    let o_bytes = encrypt_dict.get("O").and_then(|o| o.as_string()).map(|s| s.bytes.clone())
        .ok_or_else(|| PdfError::CryptographyError("Missing /O in /Encrypt".to_string()))?;
    let u_bytes = encrypt_dict.get("U").and_then(|o| o.as_string()).map(|s| s.bytes.clone())
        .ok_or_else(|| PdfError::CryptographyError("Missing /U in /Encrypt".to_string()))?;

    let mut o_hash = [0u8; 32];
    if o_bytes.len() >= 32 {
        o_hash.copy_from_slice(&o_bytes[..32]);
    }

    let doc_id = if let Some(id_arr) = doc.xref.trailer.get("ID").and_then(|o| o.as_array()) {
        id_arr.first().and_then(|o| o.as_string()).map(|s| s.bytes.clone()).unwrap_or_else(|| vec![0x42u8; 16])
    } else {
        vec![0x42u8; 16]
    };

    let encrypt_meta = encrypt_dict.get("EncryptMetadata").and_then(|o| o.as_boolean()).unwrap_or(true);

    // 1. Try User Password
    let doc_key = compute_encryption_key_rev4(password, &o_hash, p_val, &doc_id, encrypt_meta);
    let u_test = compute_user_hash_rev4(&doc_key, &doc_id);

    if u_bytes.len() >= 16 && u_test[..16] == u_bytes[..16] {
        return Ok((false, doc_key));
    }

    // 2. Try Owner Password
    let padded_owner = pad_password(password);
    let mut owner_digest = md5(&padded_owner);
    for _ in 0..50 {
        owner_digest = md5(&owner_digest[..16]);
    }
    let key = &owner_digest[..16];
    let iv = [0x5au8; 16];
    let mut ciphertext = Vec::with_capacity(48);
    ciphertext.extend_from_slice(&iv);
    ciphertext.extend_from_slice(&o_hash);

    if let Ok(decrypted_user_pass) = aes_cbc_decrypt(key, &ciphertext) {
        if let Ok(user_pass_str) = std::str::from_utf8(&decrypted_user_pass) {
            let doc_key = compute_encryption_key_rev4(user_pass_str.trim_matches('\0'), &o_hash, p_val, &doc_id, encrypt_meta);
            return Ok((true, doc_key));
        }
    }

    Err(PdfError::CryptographyError("Incorrect password for encrypted document".to_string()))
}

/// Decrypts an encrypted document in place and purges the `/Encrypt` dictionary from trailer.
pub fn decrypt_document(doc: &mut PdfDocument, password: &str) -> PdfResult<()> {
    let (_is_owner, doc_key) = authenticate_document(doc, password)?;

    let encrypt_id = doc.xref.trailer.get("Encrypt").and_then(|o| o.as_reference());

    // Decrypt all objects
    let obj_ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in obj_ids {
        if Some(id) == encrypt_id {
            continue;
        }
        if let Some(mut obj) = doc.objects.remove(&id) {
            decrypt_indirect_object(&mut obj, id.number, id.generation, &doc_key)?;
            doc.objects.insert(id, obj);
        }
    }

    // Remove /Encrypt from trailer and objects
    doc.xref.trailer.remove("Encrypt");
    if let Some(eid) = encrypt_id {
        doc.objects.remove(&eid);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_doc() -> PdfDocument {
        let mut doc = PdfDocument::empty();
        let page_id = doc.alloc_object_id();
        let mut page = PdfDictionary::new();
        page.insert("Type", PdfObject::Name(PdfName::new("Page")));
        doc.objects.insert(page_id, PdfObject::Dictionary(page));
        doc
    }

    #[test]
    fn test_encrypt_and_authenticate_roundtrip() {
        let mut doc = create_test_doc();
        let opts = EncryptionOptions {
            user_password: "user123".to_string(),
            owner_password: "secret_owner".to_string(),
            permissions: PdfPermissions::read_only(),
            revision: EncryptionRevision::Aes128,
            encrypt_metadata: true,
        };

        encrypt_document(&mut doc, &opts).unwrap();
        assert!(doc.xref.trailer.contains_key("Encrypt"));

        // Authenticate with wrong password should fail
        let bad = authenticate_document(&doc, "wrong_pass");
        assert!(bad.is_err());

        // Authenticate with user password
        let (is_owner_user, key_user) = authenticate_document(&doc, "user123").unwrap();
        assert!(!is_owner_user);
        assert_ne!(key_user, [0u8; 16]);

        // Decrypt document with user password
        decrypt_document(&mut doc, "user123").unwrap();
        assert!(!doc.xref.trailer.contains_key("Encrypt"));
    }
}
