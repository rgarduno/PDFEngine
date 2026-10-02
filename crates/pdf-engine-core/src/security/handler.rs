//! Standard Security Handler for ISO 32000-1 §7.6.3, revision 4 (AES-128).
//! Revision 5 and 6 (AES-256) are not implemented and are rejected.
//! The permissions bitmask is stored in `/P`. Enforcement of those flags is separate.

use crate::cos::{
    ObjectId, PdfArray, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfString, StringFormat,
    XRefEntry,
};
use crate::crypto::{aes_cbc_decrypt, aes_cbc_encrypt, md5};
use crate::error::{PdfError, PdfResult};
use crate::security::permissions::PdfPermissions;

/// Standard 32-byte padding string specified by ISO 32000-1 §7.6.3.3 Algorithm 2 Step 1.
pub const STANDARD_PADDING: [u8; 32] = [
    0x28, 0xbf, 0x4e, 0x5e, 0x4e, 0x75, 0x8a, 0x41, 0x64, 0x00, 0x4e, 0x56, 0xff, 0xfa, 0x01, 0x08,
    0x2e, 0x2e, 0x00, 0xb6, 0xd0, 0x68, 0x3e, 0x80, 0x2f, 0x0c, 0xa9, 0xfe, 0x64, 0x53, 0x69, 0x7a,
];

/// Supported encryption revision levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionRevision {
    /// Revision 4 (128-bit AES in CBC mode, ISO 32000-1 §7.6.3).
    Aes128 = 4,
    /// Present so callers can name the revision. `encrypt_document` rejects it.
    Aes256 = 5,
}

/// Parameters for encrypting a document.
#[derive(Debug, Clone)]
pub struct EncryptionOptions {
    /// Password required to open the document. Empty means the file opens without one.
    pub user_password: String,
    /// Password required to change permissions. `encrypt_document` rejects an empty value.
    pub owner_password: String,
    /// Granular user access permissions flags stored in `/P`.
    pub permissions: PdfPermissions,
    /// Requested revision. Only `Aes128` is accepted.
    pub revision: EncryptionRevision,
    /// Whether to encrypt the document metadata stream (`/EncryptMetadata`).
    pub encrypt_metadata: bool,
}

impl Default for EncryptionOptions {
    fn default() -> Self {
        Self {
            user_password: String::new(),
            owner_password: String::new(),
            permissions: PdfPermissions::default(),
            revision: EncryptionRevision::Aes128,
            encrypt_metadata: true,
        }
    }
}

/// Draws 16 bytes from the operating system random source.
fn random_bytes_16() -> PdfResult<[u8; 16]> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).map_err(|err| {
        PdfError::CryptographyError(format!("Operating system random source failed: {err}"))
    })?;
    Ok(buf)
}

/// Returns the first trailer `/ID` string when the file already has one.
fn existing_file_id(doc: &PdfDocument) -> Option<Vec<u8>> {
    let bytes = doc
        .xref
        .trailer
        .get("ID")
        .and_then(|obj| obj.as_array())
        .and_then(|arr| arr.first())
        .and_then(|obj| obj.as_string())
        .map(|value| value.bytes.clone())?;
    if bytes.is_empty() {
        None
    } else {
        Some(bytes)
    }
}

/// Parses every in-use object into the document cache.
///
/// A freshly loaded file keeps object bodies in `raw_data` until something asks
/// for them. Encrypting or decrypting only the cache would leave those bodies untouched.
fn materialize_objects(doc: &mut PdfDocument) -> PdfResult<()> {
    let ids: Vec<ObjectId> = doc
        .xref
        .entries
        .iter()
        .filter_map(|(&id, entry)| match entry {
            XRefEntry::InUse { .. } | XRefEntry::Compressed { .. } => Some(id),
            XRefEntry::Free { .. } => None,
        })
        .collect();
    for id in ids {
        doc.get_object(id)?;
    }
    Ok(())
}

/// Derives the object-specific encryption key for object `(id, gen)` using Algorithm 1 (§7.6.3.1)
/// with the AES salt extension `sAlT` (§7.6.2).
pub fn derive_object_key(doc_key: &[u8], id: u32, gen: u16) -> [u8; 16] {
    let mut data = Vec::with_capacity(doc_key.len() + 9);
    data.extend_from_slice(doc_key);
    data.push((id & 0xff) as u8);
    data.push(((id >> 8) & 0xff) as u8);
    data.push(((id >> 16) & 0xff) as u8);
    data.push((gen & 0xff) as u8);
    data.push(((gen >> 8) & 0xff) as u8);
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

/// Computes the owner value (`/O`).
///
/// The fixed IV is part of this handler's password check. It is not the per-object
/// AESV2 IV. The returned bytes are the ciphertext without that IV, long enough to
/// decrypt back to the padded user password.
pub fn compute_owner_hash(
    owner_pass: &str,
    user_pass: &str,
    key_len_bytes: usize,
) -> PdfResult<[u8; 48]> {
    let padded_owner = pad_password(if owner_pass.is_empty() {
        user_pass
    } else {
        owner_pass
    });
    let mut digest = md5(&padded_owner);

    for _ in 0..50 {
        digest = md5(&digest[..key_len_bytes]);
    }

    let key = &digest[..key_len_bytes.min(16)];
    let padded_user = pad_password(user_pass);
    let iv = [0x5au8; 16];
    let encrypted = aes_cbc_encrypt(key, &iv, &padded_user)?;
    if encrypted.len() != 64 {
        return Err(PdfError::CryptographyError(
            "Owner value encryption returned an unexpected ciphertext length.".into(),
        ));
    }
    let mut o_value = [0u8; 48];
    o_value.copy_from_slice(&encrypted[16..64]);
    Ok(o_value)
}

/// Computes the document encryption key using Algorithm 2 (§7.6.3.3) for revision 4.
pub fn compute_encryption_key_rev4(
    password: &str,
    o_value: &[u8],
    p_value: i32,
    doc_id: &[u8],
    encrypt_metadata: bool,
) -> [u8; 16] {
    compute_encryption_key_from_padded(
        &pad_password(password),
        o_value,
        p_value,
        doc_id,
        encrypt_metadata,
    )
}

fn compute_encryption_key_from_padded(
    padded_pass: &[u8; 32],
    o_value: &[u8],
    p_value: i32,
    doc_id: &[u8],
    encrypt_metadata: bool,
) -> [u8; 16] {
    let mut buffer = Vec::with_capacity(32 + o_value.len() + 4 + doc_id.len() + 4);
    buffer.extend_from_slice(padded_pass);
    buffer.extend_from_slice(o_value);
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

/// Computes the user validation value (`/U`) using this handler's revision 4 check.
///
/// The fixed IV is part of the password check. The first 16 bytes of the result are
/// the ciphertext block that `authenticate_document` compares. The rest is zero.
pub fn compute_user_hash_rev4(doc_key: &[u8; 16], doc_id: &[u8]) -> PdfResult<[u8; 32]> {
    let mut data = Vec::with_capacity(32 + doc_id.len());
    data.extend_from_slice(&STANDARD_PADDING);
    data.extend_from_slice(doc_id);
    let hash = md5(&data);

    let iv = [0x77u8; 16];
    let encrypted = aes_cbc_encrypt(doc_key, &iv, &hash)?;
    if encrypted.len() < 32 {
        return Err(PdfError::CryptographyError(
            "User value encryption returned a short ciphertext.".into(),
        ));
    }
    let mut u_value = [0u8; 32];
    u_value[..16].copy_from_slice(&encrypted[16..32]);
    Ok(u_value)
}

/// Encrypts an indirect PDF object's strings and streams in place.
///
/// Each string and each stream gets its own random 16-byte IV, prepended to the
/// ciphertext, which is what AESV2 requires. Dictionary names and the structural
/// keys `/Type`, `/Subtype`, `/Filter`, and `/Length` stay in the clear.
pub fn encrypt_indirect_object(
    obj: &mut PdfObject,
    id: u32,
    gen: u16,
    doc_key: &[u8],
) -> PdfResult<()> {
    let obj_key = derive_object_key(doc_key, id, gen);

    match obj {
        PdfObject::Stream(stream) => {
            let iv = random_bytes_16()?;
            let encrypted = aes_cbc_encrypt(&obj_key, &iv, &stream.content)?;
            stream.content = encrypted;
            stream
                .dict
                .insert("Length", PdfObject::Integer(stream.content.len() as i64));
        }
        PdfObject::String(value) => {
            let iv = random_bytes_16()?;
            let encrypted = aes_cbc_encrypt(&obj_key, &iv, &value.bytes)?;
            value.bytes = encrypted;
            value.format = StringFormat::Hexadecimal;
        }
        PdfObject::Array(arr) => {
            for item in arr.iter_mut() {
                encrypt_indirect_object(item, id, gen, doc_key)?;
            }
        }
        PdfObject::Dictionary(dict) => {
            for (key, val) in dict.iter_mut() {
                if key != "Type" && key != "Subtype" && key != "Filter" && key != "Length" {
                    encrypt_indirect_object(val, id, gen, doc_key)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Decrypts an indirect PDF object's strings and streams in place.
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
                stream
                    .dict
                    .insert("Length", PdfObject::Integer(stream.content.len() as i64));
            }
        }
        PdfObject::String(value) => {
            if value.bytes.len() >= 32 {
                value.bytes = aes_cbc_decrypt(&obj_key, &value.bytes)?;
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

/// Encrypts an entire `PdfDocument` in place with revision 4 AES-128.
pub fn encrypt_document(doc: &mut PdfDocument, options: &EncryptionOptions) -> PdfResult<()> {
    if options.owner_password.is_empty() {
        return Err(PdfError::CryptographyError(
            "Owner password is required.".into(),
        ));
    }
    if options.revision != EncryptionRevision::Aes128 {
        return Err(PdfError::CryptographyError(
            "Only AES-128 revision 4 encryption is implemented.".into(),
        ));
    }

    materialize_objects(doc)?;

    let doc_id = if let Some(existing) = existing_file_id(doc) {
        existing
    } else {
        let generated = random_bytes_16()?.to_vec();
        let mut id_arr = PdfArray::new();
        id_arr.push(PdfObject::String(PdfString::hex(generated.clone())));
        id_arr.push(PdfObject::String(PdfString::hex(generated.clone())));
        doc.xref.trailer.insert("ID", PdfObject::Array(id_arr));
        generated
    };

    let p_value = options.permissions.to_p_value();
    let o_value = compute_owner_hash(&options.owner_password, &options.user_password, 16)?;
    let doc_key = compute_encryption_key_rev4(
        &options.user_password,
        &o_value,
        p_value,
        &doc_id,
        options.encrypt_metadata,
    );
    let u_value = compute_user_hash_rev4(&doc_key, &doc_id)?;

    let mut encrypt_dict = PdfDictionary::new();
    encrypt_dict.insert("Filter", PdfObject::Name(PdfName::new("Standard")));
    encrypt_dict.insert("V", PdfObject::Integer(4));
    encrypt_dict.insert("R", PdfObject::Integer(4));
    encrypt_dict.insert("Length", PdfObject::Integer(128));
    encrypt_dict.insert("P", PdfObject::Integer(p_value as i64));
    encrypt_dict.insert("O", PdfObject::String(PdfString::hex(o_value.to_vec())));
    encrypt_dict.insert("U", PdfObject::String(PdfString::hex(u_value.to_vec())));

    let mut cf_dict = PdfDictionary::new();
    let mut std_cf = PdfDictionary::new();
    std_cf.insert("CFM", PdfObject::Name(PdfName::new("AESV2")));
    std_cf.insert("AuthEvent", PdfObject::Name(PdfName::new("DocOpen")));
    std_cf.insert("Length", PdfObject::Integer(128));
    cf_dict.insert("StdCF", PdfObject::Dictionary(std_cf));

    encrypt_dict.insert("CF", PdfObject::Dictionary(cf_dict));
    encrypt_dict.insert("StmF", PdfObject::Name(PdfName::new("StdCF")));
    encrypt_dict.insert("StrF", PdfObject::Name(PdfName::new("StdCF")));
    encrypt_dict.insert(
        "EncryptMetadata",
        PdfObject::Boolean(options.encrypt_metadata),
    );

    let encrypt_id = doc.alloc_object_id();
    doc.set_object(encrypt_id, PdfObject::Dictionary(encrypt_dict));
    doc.xref
        .trailer
        .insert("Encrypt", PdfObject::Reference(encrypt_id));

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

/// Resolves the trailer `/Encrypt` dictionary, loading it from the file when needed.
fn load_encrypt_dictionary(doc: &mut PdfDocument) -> PdfResult<PdfDictionary> {
    match doc.xref.trailer.get("Encrypt").cloned() {
        Some(PdfObject::Dictionary(dict)) => Ok(dict),
        Some(PdfObject::Reference(id)) => match doc.get_object(id)? {
            PdfObject::Dictionary(dict) => Ok(dict),
            _ => Err(PdfError::CryptographyError(
                "Missing /Encrypt dictionary".to_string(),
            )),
        },
        _ => Err(PdfError::CryptographyError(
            "Document is not encrypted".to_string(),
        )),
    }
}

/// Authenticates a password against an encrypted document's `/Encrypt` dictionary.
/// Returns `(is_owner, doc_encryption_key)` on success, or an error on an invalid password.
pub fn authenticate_document(doc: &mut PdfDocument, password: &str) -> PdfResult<(bool, [u8; 16])> {
    let encrypt_dict = load_encrypt_dictionary(doc)?;

    let p_val = encrypt_dict
        .get("P")
        .and_then(|obj| obj.as_integer())
        .unwrap_or(-4) as i32;
    let o_bytes = encrypt_dict
        .get("O")
        .and_then(|obj| obj.as_string())
        .map(|value| value.bytes.clone())
        .ok_or_else(|| PdfError::CryptographyError("Missing /O in /Encrypt".to_string()))?;
    let u_bytes = encrypt_dict
        .get("U")
        .and_then(|obj| obj.as_string())
        .map(|value| value.bytes.clone())
        .ok_or_else(|| PdfError::CryptographyError("Missing /U in /Encrypt".to_string()))?;
    if o_bytes.len() != 48 {
        return Err(PdfError::CryptographyError(
            "Encryption dictionary /O has an unexpected length.".into(),
        ));
    }
    let mut o_value = [0u8; 48];
    o_value.copy_from_slice(&o_bytes);

    let doc_id = existing_file_id(doc).ok_or_else(|| {
        PdfError::CryptographyError("Encrypted document is missing a file identifier.".into())
    })?;
    let encrypt_meta = encrypt_dict
        .get("EncryptMetadata")
        .and_then(|obj| obj.as_boolean())
        .unwrap_or(true);

    let doc_key = compute_encryption_key_rev4(password, &o_value, p_val, &doc_id, encrypt_meta);
    let u_test = compute_user_hash_rev4(&doc_key, &doc_id)?;
    if u_bytes.len() >= 16 && u_test[..16] == u_bytes[..16] {
        return Ok((false, doc_key));
    }

    let padded_owner = pad_password(password);
    let mut owner_digest = md5(&padded_owner);
    for _ in 0..50 {
        owner_digest = md5(&owner_digest[..16]);
    }
    let iv = [0x5au8; 16];
    let mut ciphertext = Vec::with_capacity(64);
    ciphertext.extend_from_slice(&iv);
    ciphertext.extend_from_slice(&o_value);
    if let Ok(decrypted_user) = aes_cbc_decrypt(&owner_digest[..16], &ciphertext) {
        if decrypted_user.len() == 32 {
            let mut padded_user = [0u8; 32];
            padded_user.copy_from_slice(&decrypted_user);
            let owner_key = compute_encryption_key_from_padded(
                &padded_user,
                &o_value,
                p_val,
                &doc_id,
                encrypt_meta,
            );
            let owner_u = compute_user_hash_rev4(&owner_key, &doc_id)?;
            if u_bytes.len() >= 16 && owner_u[..16] == u_bytes[..16] {
                return Ok((true, owner_key));
            }
        }
    }

    Err(PdfError::CryptographyError(
        "Incorrect password for encrypted document".to_string(),
    ))
}

/// Decrypts an encrypted document in place and removes `/Encrypt` from the trailer.
pub fn decrypt_document(doc: &mut PdfDocument, password: &str) -> PdfResult<()> {
    let (_is_owner, doc_key) = authenticate_document(doc, password)?;
    let encrypt_id = doc
        .xref
        .trailer
        .get("Encrypt")
        .and_then(|obj| obj.as_reference());

    materialize_objects(doc)?;

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

    doc.xref.trailer.remove("Encrypt");
    if let Some(eid) = encrypt_id {
        doc.objects.remove(&eid);
        doc.xref.entries.remove(&eid);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_options() -> EncryptionOptions {
        EncryptionOptions {
            user_password: "user123".to_string(),
            owner_password: "secret_owner".to_string(),
            permissions: PdfPermissions::read_only(),
            revision: EncryptionRevision::Aes128,
            encrypt_metadata: true,
        }
    }

    fn create_test_doc() -> PdfDocument {
        let mut doc = PdfDocument::empty();
        let page_id = doc.alloc_object_id();
        let mut page = PdfDictionary::new();
        page.insert("Type", PdfObject::Name(PdfName::new("Page")));
        doc.objects.insert(page_id, PdfObject::Dictionary(page));
        doc
    }

    fn document_with_secret() -> PdfDocument {
        let mut doc = PdfDocument::empty();
        let note_id = doc.alloc_object_id();
        let mut note = PdfDictionary::new();
        note.insert("Type", PdfObject::Name(PdfName::new("Note")));
        note.insert(
            "Secret",
            PdfObject::String(PdfString::literal(b"classified-note".to_vec())),
        );
        doc.set_object(note_id, PdfObject::Dictionary(note));
        doc
    }

    #[test]
    fn test_encrypt_and_authenticate_roundtrip() {
        let mut doc = create_test_doc();
        encrypt_document(&mut doc, &sample_options()).unwrap();
        assert!(doc.xref.trailer.contains_key("Encrypt"));
        let file_id = existing_file_id(&doc).unwrap();
        assert_ne!(file_id, vec![0x42u8; 16]);

        assert!(authenticate_document(&mut doc, "wrong_pass").is_err());

        let (is_owner_user, key_user) = authenticate_document(&mut doc, "user123").unwrap();
        assert!(!is_owner_user);
        assert_ne!(key_user, [0u8; 16]);

        let (is_owner, owner_key) = authenticate_document(&mut doc, "secret_owner").unwrap();
        assert!(is_owner);
        assert_ne!(owner_key, [0u8; 16]);

        decrypt_document(&mut doc, "user123").unwrap();
        assert!(!doc.xref.trailer.contains_key("Encrypt"));
    }

    #[test]
    fn test_empty_owner_password_is_rejected() {
        let mut doc = create_test_doc();
        let err = encrypt_document(&mut doc, &EncryptionOptions::default()).unwrap_err();
        assert!(matches!(err, PdfError::CryptographyError(_)));
        assert!(!doc.xref.trailer.contains_key("Encrypt"));
    }

    #[test]
    fn test_aes256_revision_is_rejected() {
        let mut doc = create_test_doc();
        let opts = EncryptionOptions {
            revision: EncryptionRevision::Aes256,
            ..sample_options()
        };
        assert!(encrypt_document(&mut doc, &opts).is_err());
        assert!(!doc.xref.trailer.contains_key("Encrypt"));
    }

    #[test]
    fn test_loaded_file_hides_plaintext_and_uses_a_fresh_iv() {
        let plain = document_with_secret().save_to_vec().unwrap();
        assert!(plain
            .windows(b"classified-note".len())
            .any(|window| window == b"classified-note"));

        let mut first = PdfDocument::load(&plain).unwrap();
        let mut second = PdfDocument::load(&plain).unwrap();
        assert!(first.objects.is_empty());
        encrypt_document(&mut first, &sample_options()).unwrap();
        encrypt_document(&mut second, &sample_options()).unwrap();

        let encrypted_a = first.save_to_vec().unwrap();
        let encrypted_b = second.save_to_vec().unwrap();
        assert_ne!(encrypted_a, encrypted_b);
        assert!(encrypted_a
            .windows(b"/Encrypt".len())
            .any(|window| window == b"/Encrypt"));
        assert!(!encrypted_a
            .windows(b"classified-note".len())
            .any(|window| window == b"classified-note"));

        let mut opened = PdfDocument::load(&encrypted_a).unwrap();
        decrypt_document(&mut opened, "secret_owner").unwrap();
        let restored = opened.save_to_vec().unwrap();
        assert!(!restored
            .windows(b"/Encrypt".len())
            .any(|window| window == b"/Encrypt"));

        let mut reloaded = PdfDocument::load(&restored).unwrap();
        let ids: Vec<ObjectId> = reloaded.xref.entries.keys().copied().collect();
        let mut secret = None;
        for id in ids {
            if let Ok(PdfObject::Dictionary(dict)) = reloaded.get_object(id) {
                if let Some(PdfObject::String(value)) = dict.get("Secret") {
                    secret = Some(value.bytes.clone());
                }
            }
        }
        assert_eq!(secret.as_deref(), Some(&b"classified-note"[..]));
    }
}
