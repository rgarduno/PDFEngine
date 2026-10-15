//! Standard Security Handler for ISO 32000-1 §7.6.3 (Revision 4, AES-128) and
//! ISO 32000-2 §7.6.4 (Revisions 5 and 6, AES-256).
//! After a user password opens the file, the `/P` integer blocks print, copy, and edit.
//! `/Perms` is checked only for the AES-256 `adb` marker. The owner password is unrestricted.

use crate::cos::{
    ObjectId, PdfArray, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfString, StringFormat,
    XRefEntry,
};
use crate::crypto::{
    aes_cbc_decrypt, aes_cbc_decrypt_nopad, aes_cbc_encrypt, aes_cbc_encrypt_nopad, md5,
};
use crate::error::{PdfError, PdfResult};
use crate::security::permissions::PdfPermissions;
use sha2::{Digest, Sha256, Sha384, Sha512};

/// Standard 32-byte padding string specified by ISO 32000-1 §7.6.3.3 Algorithm 2 Step 1.
pub const STANDARD_PADDING: [u8; 32] = [
    0x28, 0xbf, 0x4e, 0x5e, 0x4e, 0x75, 0x8a, 0x41, 0x64, 0x00, 0x4e, 0x56, 0xff, 0xfa, 0x01, 0x08,
    0x2e, 0x2e, 0x00, 0xb6, 0xd0, 0x68, 0x3e, 0x80, 0x2f, 0x0c, 0xa9, 0xfe, 0x64, 0x53, 0x69, 0x7a,
];

/// Supported encryption revision levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionRevision {
    /// Revision 4 (128-bit AES in CBC mode, `/V 4`, `/R 4`, ISO 32000-1 §7.6.3).
    Aes128 = 4,
    /// Revision 5 (256-bit AES in CBC mode, `/V 5`, `/R 5`, Adobe Extension Level 3).
    Aes256Rev5 = 5,
    /// Revision 6 (256-bit AES in CBC mode with hardened iterative KDF, `/V 5`, `/R 6`, ISO 32000-2 §7.6.4).
    Aes256 = 6,
}

impl EncryptionRevision {
    /// Parses an encryption algorithm identifier (`"aes128"`, `"aes256"`, `"aes256-r5"`, etc.).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "aes128" | "aes-128" | "aes128-r4" | "aes-128-r4" | "rev4" | "r4" => Some(Self::Aes128),
            "aes256-r5" | "aes-256-r5" | "rev5" | "r5" => Some(Self::Aes256Rev5),
            "aes256" | "aes-256" | "aes256-r6" | "aes-256-r6" | "rev6" | "r6" => Some(Self::Aes256),
            _ => None,
        }
    }

    /// Returns the canonical algorithm label (`"AES-128-R4"`, `"AES-256-R5"`, or `"AES-256-R6"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Aes128 => "AES-128-R4",
            Self::Aes256Rev5 => "AES-256-R5",
            Self::Aes256 => "AES-256-R6",
        }
    }
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
    /// Requested encryption revision (`Aes128`, `Aes256Rev5`, or `Aes256`).
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

/// Draws 32 bytes from the operating system random source.
fn random_bytes_32() -> PdfResult<[u8; 32]> {
    let mut buf = [0u8; 32];
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

/// Prepares a UTF-8 password according to SASLprep (RFC 4013 / RFC 3454) and truncates
/// to a maximum of 127 bytes as required by ISO 32000-2 §7.6.4.3.2.
pub fn prepare_utf8_password(password: &str) -> Vec<u8> {
    let mut normalized = String::with_capacity(password.len());
    for ch in password.chars() {
        // RFC 3454 Table B.1: Commonly mapped to nothing
        if matches!(
            ch,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{1806}'
                | '\u{180B}'..='\u{180D}'
                | '\u{200B}'..='\u{200D}'
                | '\u{2060}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
        ) {
            continue;
        }
        // RFC 3454 Table C.1.2: Non-ASCII space characters mapped to U+0020
        if matches!(
            ch,
            '\u{00A0}' | '\u{1680}' | '\u{2000}'
                ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
        ) {
            normalized.push(' ');
        } else {
            normalized.push(ch);
        }
    }
    let mut bytes = normalized.into_bytes();
    if bytes.len() > 127 {
        bytes.truncate(127);
    }
    bytes
}

/// Computes a 32-byte hash for Revision 5 using single-pass SHA-256 over `password || salt || user_key`.
pub fn compute_hash_rev5(password: &[u8], salt: &[u8], user_key: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(password);
    hasher.update(salt);
    hasher.update(user_key);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Computes a 32-byte hash using Algorithm 2.B (ISO 32000-2 §7.6.4.3.4) for Revision 6.
///
/// Performs at least 64 rounds (and continues until `E[last] <= round - 32`) of
/// repeating `password || K || user_key` 64 times, encrypting with AES-128-CBC (no padding)
/// using `K[0..16]` as key and `K[16..32]` as IV, and hashing the ciphertext `E` with
/// SHA-256, SHA-384, or SHA-512 selected by `BigEndianUint128(E[0..16]) mod 3`.
pub fn compute_hash_rev6(password: &[u8], salt: &[u8], user_key: &[u8]) -> PdfResult<[u8; 32]> {
    let mut k = compute_hash_rev5(password, salt, user_key).to_vec();
    let mut round: usize = 0;

    loop {
        let single_len = password.len() + k.len() + user_key.len();
        let mut k1 = Vec::with_capacity(single_len * 64);
        for _ in 0..64 {
            k1.extend_from_slice(password);
            k1.extend_from_slice(&k);
            k1.extend_from_slice(user_key);
        }

        let aes_key = &k[0..16];
        let mut aes_iv = [0u8; 16];
        aes_iv.copy_from_slice(&k[16..32]);

        let e = aes_cbc_encrypt_nopad(aes_key, &aes_iv, &k1)?;

        let mut first_16 = [0u8; 16];
        first_16.copy_from_slice(&e[0..16]);
        let rem = (u128::from_be_bytes(first_16) % 3) as u8;

        k = match rem {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };

        round += 1;
        let last_byte = e.last().copied().unwrap_or(0) as usize;
        if round >= 64 && last_byte <= round - 32 {
            break;
        }
    }

    let mut out = [0u8; 32];
    out.copy_from_slice(&k[0..32]);
    Ok(out)
}

fn compute_rev5_or_6_hash(
    revision: EncryptionRevision,
    password: &[u8],
    salt: &[u8],
    user_key: &[u8],
) -> PdfResult<[u8; 32]> {
    match revision {
        EncryptionRevision::Aes256 => compute_hash_rev6(password, salt, user_key),
        EncryptionRevision::Aes256Rev5 => Ok(compute_hash_rev5(password, salt, user_key)),
        EncryptionRevision::Aes128 => Err(PdfError::CryptographyError(
            "Internal error: AES-128 passed to AES-256 hash helper".into(),
        )),
    }
}

struct Aes256EncryptEntries {
    doc_key: [u8; 32],
    u_value: [u8; 48],
    ue_value: [u8; 32],
    o_value: [u8; 48],
    oe_value: [u8; 32],
    perms_value: [u8; 16],
}

/// Computes the 256-bit document encryption key and `/U`, `/UE`, `/O`, `/OE`, `/Perms`
/// entries according to ISO 32000-2 §7.6.4.4 Algorithms 8, 9, and 10.
fn compute_aes256_entries(
    options: &EncryptionOptions,
    p_value: i32,
) -> PdfResult<Aes256EncryptEntries> {
    let doc_key = random_bytes_32()?;
    let zero_iv = [0u8; 16];

    // Algorithm 8: Computing /U and /UE
    let user_pw = prepare_utf8_password(&options.user_password);
    let u_salts = random_bytes_16()?;
    let u_val_salt = &u_salts[0..8];
    let u_key_salt = &u_salts[8..16];

    let u_hash = compute_rev5_or_6_hash(options.revision, &user_pw, u_val_salt, &[])?;
    let mut u_value = [0u8; 48];
    u_value[0..32].copy_from_slice(&u_hash);
    u_value[32..40].copy_from_slice(u_val_salt);
    u_value[40..48].copy_from_slice(u_key_salt);

    let u_inter_key = compute_rev5_or_6_hash(options.revision, &user_pw, u_key_salt, &[])?;
    let ue_vec = aes_cbc_encrypt_nopad(&u_inter_key, &zero_iv, &doc_key)?;
    let mut ue_value = [0u8; 32];
    ue_value.copy_from_slice(&ue_vec);

    // Algorithm 9: Computing /O and /OE
    let owner_pw = prepare_utf8_password(&options.owner_password);
    let o_salts = random_bytes_16()?;
    let o_val_salt = &o_salts[0..8];
    let o_key_salt = &o_salts[8..16];

    let o_hash = compute_rev5_or_6_hash(options.revision, &owner_pw, o_val_salt, &u_value)?;
    let mut o_value = [0u8; 48];
    o_value[0..32].copy_from_slice(&o_hash);
    o_value[32..40].copy_from_slice(o_val_salt);
    o_value[40..48].copy_from_slice(o_key_salt);

    let o_inter_key = compute_rev5_or_6_hash(options.revision, &owner_pw, o_key_salt, &u_value)?;
    let oe_vec = aes_cbc_encrypt_nopad(&o_inter_key, &zero_iv, &doc_key)?;
    let mut oe_value = [0u8; 32];
    oe_value.copy_from_slice(&oe_vec);

    // Algorithm 10: Computing /Perms
    let perms_rand = random_bytes_16()?;
    let mut perms_plain = [0u8; 16];
    perms_plain[0..4].copy_from_slice(&(p_value as u32).to_le_bytes());
    perms_plain[4..8].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
    perms_plain[8] = if options.encrypt_metadata { b'T' } else { b'F' };
    perms_plain[9..12].copy_from_slice(b"adb");
    perms_plain[12..16].copy_from_slice(&perms_rand[0..4]);

    let perms_vec = aes_cbc_encrypt_nopad(&doc_key, &zero_iv, &perms_plain)?;
    let mut perms_value = [0u8; 16];
    perms_value.copy_from_slice(&perms_vec);

    Ok(Aes256EncryptEntries {
        doc_key,
        u_value,
        ue_value,
        o_value,
        oe_value,
        perms_value,
    })
}

/// Computes the owner value (`/O`) for Revision 4.
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
/// ciphertext, which is what AESV2 and AESV3 require. Dictionary names and the structural
/// keys `/Type`, `/Subtype`, `/Filter`, and `/Length` stay in the clear.
pub fn encrypt_indirect_object(
    obj: &mut PdfObject,
    id: u32,
    gen: u16,
    doc_key: &[u8],
) -> PdfResult<()> {
    let derived;
    let obj_key: &[u8] = if doc_key.len() == 32 {
        doc_key
    } else {
        derived = derive_object_key(doc_key, id, gen);
        &derived
    };

    match obj {
        PdfObject::Stream(stream) => {
            let iv = random_bytes_16()?;
            let encrypted = aes_cbc_encrypt(obj_key, &iv, &stream.content)?;
            stream.content = encrypted;
            stream
                .dict
                .insert("Length", PdfObject::Integer(stream.content.len() as i64));
        }
        PdfObject::String(value) => {
            let iv = random_bytes_16()?;
            let encrypted = aes_cbc_encrypt(obj_key, &iv, &value.bytes)?;
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
    let derived;
    let obj_key: &[u8] = if doc_key.len() == 32 {
        doc_key
    } else {
        derived = derive_object_key(doc_key, id, gen);
        &derived
    };

    match obj {
        PdfObject::Stream(stream) => {
            if stream.content.len() >= 32 {
                let decrypted = aes_cbc_decrypt(obj_key, &stream.content)?;
                stream.content = decrypted;
                stream
                    .dict
                    .insert("Length", PdfObject::Integer(stream.content.len() as i64));
            }
        }
        PdfObject::String(value) => {
            if value.bytes.len() >= 32 {
                value.bytes = aes_cbc_decrypt(obj_key, &value.bytes)?;
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

/// Encrypts an entire `PdfDocument` in place with Revision 4 (AES-128) or Revisions 5/6 (AES-256).
pub fn encrypt_document(doc: &mut PdfDocument, options: &EncryptionOptions) -> PdfResult<()> {
    if options.owner_password.is_empty() {
        return Err(PdfError::CryptographyError(
            "Owner password is required.".into(),
        ));
    }
    doc.require_security_change()?;

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
    let mut encrypt_dict = PdfDictionary::new();
    encrypt_dict.insert("Filter", PdfObject::Name(PdfName::new("Standard")));
    encrypt_dict.insert("P", PdfObject::Integer(p_value as i64));

    let doc_key: Vec<u8> = match options.revision {
        EncryptionRevision::Aes128 => {
            let o_value = compute_owner_hash(&options.owner_password, &options.user_password, 16)?;
            let key = compute_encryption_key_rev4(
                &options.user_password,
                &o_value,
                p_value,
                &doc_id,
                options.encrypt_metadata,
            );
            let u_value = compute_user_hash_rev4(&key, &doc_id)?;

            encrypt_dict.insert("V", PdfObject::Integer(4));
            encrypt_dict.insert("R", PdfObject::Integer(4));
            encrypt_dict.insert("Length", PdfObject::Integer(128));
            encrypt_dict.insert("O", PdfObject::String(PdfString::hex(o_value.to_vec())));
            encrypt_dict.insert("U", PdfObject::String(PdfString::hex(u_value.to_vec())));

            let mut cf_dict = PdfDictionary::new();
            let mut std_cf = PdfDictionary::new();
            std_cf.insert("CFM", PdfObject::Name(PdfName::new("AESV2")));
            std_cf.insert("AuthEvent", PdfObject::Name(PdfName::new("DocOpen")));
            std_cf.insert("Length", PdfObject::Integer(16));
            cf_dict.insert("StdCF", PdfObject::Dictionary(std_cf));
            encrypt_dict.insert("CF", PdfObject::Dictionary(cf_dict));

            key.to_vec()
        }
        EncryptionRevision::Aes256Rev5 | EncryptionRevision::Aes256 => {
            let entries = compute_aes256_entries(options, p_value)?;
            let r_num = if options.revision == EncryptionRevision::Aes256 {
                6
            } else {
                5
            };

            encrypt_dict.insert("V", PdfObject::Integer(5));
            encrypt_dict.insert("R", PdfObject::Integer(r_num));
            encrypt_dict.insert("Length", PdfObject::Integer(256));
            encrypt_dict.insert(
                "O",
                PdfObject::String(PdfString::hex(entries.o_value.to_vec())),
            );
            encrypt_dict.insert(
                "U",
                PdfObject::String(PdfString::hex(entries.u_value.to_vec())),
            );
            encrypt_dict.insert(
                "OE",
                PdfObject::String(PdfString::hex(entries.oe_value.to_vec())),
            );
            encrypt_dict.insert(
                "UE",
                PdfObject::String(PdfString::hex(entries.ue_value.to_vec())),
            );
            encrypt_dict.insert(
                "Perms",
                PdfObject::String(PdfString::hex(entries.perms_value.to_vec())),
            );

            let mut cf_dict = PdfDictionary::new();
            let mut std_cf = PdfDictionary::new();
            std_cf.insert("CFM", PdfObject::Name(PdfName::new("AESV3")));
            std_cf.insert("AuthEvent", PdfObject::Name(PdfName::new("DocOpen")));
            std_cf.insert("Length", PdfObject::Integer(32));
            cf_dict.insert("StdCF", PdfObject::Dictionary(std_cf));
            encrypt_dict.insert("CF", PdfObject::Dictionary(cf_dict));

            entries.doc_key.to_vec()
        }
    };

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

/// Inspects the document's `/Encrypt` dictionary and returns its `EncryptionRevision` if encrypted.
pub fn get_encryption_revision(doc: &mut PdfDocument) -> Option<EncryptionRevision> {
    let dict = load_encrypt_dictionary(doc).ok()?;
    let r = dict.get("R").and_then(|o| o.as_integer()).unwrap_or(4);
    match r {
        6 => Some(EncryptionRevision::Aes256),
        5 => Some(EncryptionRevision::Aes256Rev5),
        _ => Some(EncryptionRevision::Aes128),
    }
}

fn verify_aes256_perms(encrypt_dict: &PdfDictionary, doc_key: &[u8]) -> PdfResult<()> {
    if let Some(perms_bytes) = encrypt_dict
        .get("Perms")
        .and_then(|obj| obj.as_string())
        .map(|s| &s.bytes)
    {
        if perms_bytes.len() != 16 {
            return Err(PdfError::CryptographyError(
                "Encryption dictionary /Perms has an invalid length.".into(),
            ));
        }
        let perms_plain = aes_cbc_decrypt_nopad(doc_key, &[0u8; 16], perms_bytes)?;
        if &perms_plain[9..12] != b"adb" {
            return Err(PdfError::CryptographyError(
                "Encryption dictionary /Perms marker verification failed.".into(),
            ));
        }
    }
    Ok(())
}

/// Authenticates a password against an encrypted document's `/Encrypt` dictionary.
/// Returns `(is_owner, doc_encryption_key)` on success, or an error on an invalid password.
pub fn authenticate_document(doc: &mut PdfDocument, password: &str) -> PdfResult<(bool, Vec<u8>)> {
    let encrypt_dict = load_encrypt_dictionary(doc)?;

    let r_val = encrypt_dict
        .get("R")
        .and_then(|obj| obj.as_integer())
        .unwrap_or(4);

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

    if r_val == 5 || r_val == 6 {
        let revision = if r_val == 6 {
            EncryptionRevision::Aes256
        } else {
            EncryptionRevision::Aes256Rev5
        };

        if o_bytes.len() < 48 || u_bytes.len() < 48 {
            return Err(PdfError::CryptographyError(
                "AES-256 /Encrypt dictionary /O or /U is shorter than 48 bytes.".into(),
            ));
        }
        let oe_bytes = encrypt_dict
            .get("OE")
            .and_then(|obj| obj.as_string())
            .map(|value| value.bytes.clone())
            .ok_or_else(|| PdfError::CryptographyError("Missing /OE in /Encrypt".to_string()))?;
        let ue_bytes = encrypt_dict
            .get("UE")
            .and_then(|obj| obj.as_string())
            .map(|value| value.bytes.clone())
            .ok_or_else(|| PdfError::CryptographyError("Missing /UE in /Encrypt".to_string()))?;

        if oe_bytes.len() != 32 || ue_bytes.len() != 32 {
            return Err(PdfError::CryptographyError(
                "AES-256 /Encrypt dictionary /OE or /UE must be 32 bytes.".into(),
            ));
        }

        let pw_bytes = prepare_utf8_password(password);
        let zero_iv = [0u8; 16];

        // Algorithm 12: Authenticating the Owner Password
        let o_test =
            compute_rev5_or_6_hash(revision, &pw_bytes, &o_bytes[32..40], &u_bytes[0..48])?;
        if o_test == o_bytes[0..32] {
            let o_inter_key =
                compute_rev5_or_6_hash(revision, &pw_bytes, &o_bytes[40..48], &u_bytes[0..48])?;
            let doc_key = aes_cbc_decrypt_nopad(&o_inter_key, &zero_iv, &oe_bytes)?;
            verify_aes256_perms(&encrypt_dict, &doc_key)?;
            return Ok((true, doc_key));
        }

        // Algorithm 11: Authenticating the User Password
        let u_test = compute_rev5_or_6_hash(revision, &pw_bytes, &u_bytes[32..40], &[])?;
        if u_test == u_bytes[0..32] {
            let u_inter_key = compute_rev5_or_6_hash(revision, &pw_bytes, &u_bytes[40..48], &[])?;
            let doc_key = aes_cbc_decrypt_nopad(&u_inter_key, &zero_iv, &ue_bytes)?;
            verify_aes256_perms(&encrypt_dict, &doc_key)?;
            return Ok((false, doc_key));
        }

        return Err(PdfError::CryptographyError(
            "Incorrect password for encrypted document".to_string(),
        ));
    }

    let p_val = encrypt_dict
        .get("P")
        .and_then(|obj| obj.as_integer())
        .unwrap_or(-4) as i32;
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
        return Ok((false, doc_key.to_vec()));
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
                return Ok((true, owner_key.to_vec()));
            }
        }
    }

    Err(PdfError::CryptographyError(
        "Incorrect password for encrypted document".to_string(),
    ))
}

/// Reads `/P` before `/Encrypt` is removed. A missing integer is the unrestricted value `-4`.
fn opened_permissions(doc: &mut PdfDocument) -> PdfPermissions {
    let p_value = load_encrypt_dictionary(doc)
        .ok()
        .and_then(|dict| dict.get("P").and_then(|obj| obj.as_integer()))
        .unwrap_or(-4);
    PdfPermissions::from_p_value(p_value as i32)
}

/// Decrypts an encrypted document in place and removes `/Encrypt` from the trailer.
///
/// Decrypt itself is allowed. A user password whose enforced bits are not all set
/// remembers those bits for later print, copy, and edit checks. The owner password
/// does not. The written file no longer carries `/P`.
pub fn decrypt_document(doc: &mut PdfDocument, password: &str) -> PdfResult<()> {
    let (is_owner, doc_key) = authenticate_document(doc, password)?;
    let permissions = opened_permissions(doc);
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

    // Ciphertext hid URI schemes. The decrypted graph is clear, so strip actions now.
    super::active::neutralize_active_content(doc)?;
    doc.apply_opened_permissions(is_owner, permissions);

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
        assert_eq!(
            get_encryption_revision(&mut doc),
            Some(EncryptionRevision::Aes128)
        );
        let file_id = existing_file_id(&doc).unwrap();
        assert_ne!(file_id, vec![0x42u8; 16]);

        assert!(authenticate_document(&mut doc, "wrong_pass").is_err());

        let (is_owner_user, key_user) = authenticate_document(&mut doc, "user123").unwrap();
        assert!(!is_owner_user);
        assert_eq!(key_user.len(), 16);
        assert_ne!(key_user, vec![0u8; 16]);

        let (is_owner, owner_key) = authenticate_document(&mut doc, "secret_owner").unwrap();
        assert!(is_owner);
        assert_eq!(owner_key.len(), 16);
        assert_ne!(owner_key, vec![0u8; 16]);

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
    fn test_saslprep_utf8_password_normalization_and_truncation() {
        // Soft hyphen (U+00AD) and zero-width space (U+200B) are stripped; NBSP (U+00A0) -> ' '
        let raw = "p\u{00AD}a\u{200B}ss\u{00A0}word";
        assert_eq!(prepare_utf8_password(raw), b"pass word");

        // Passwords longer than 127 UTF-8 bytes are truncated to 127 bytes
        let long_pw = "A".repeat(200);
        let prepared = prepare_utf8_password(&long_pw);
        assert_eq!(prepared.len(), 127);
    }

    #[test]
    fn test_aes256_rev6_encrypt_and_authenticate_roundtrip() {
        let plain = document_with_secret().save_to_vec().unwrap();
        let mut doc = PdfDocument::load(&plain).unwrap();

        let opts = EncryptionOptions {
            user_password: "user_rev6_pass".to_string(),
            owner_password: "owner_rev6_master".to_string(),
            permissions: PdfPermissions::read_only(),
            revision: EncryptionRevision::Aes256,
            encrypt_metadata: true,
        };
        encrypt_document(&mut doc, &opts).unwrap();
        assert!(doc.xref.trailer.contains_key("Encrypt"));
        assert_eq!(
            get_encryption_revision(&mut doc),
            Some(EncryptionRevision::Aes256)
        );

        let encrypted_bytes = doc.save_to_vec().unwrap();
        assert!(!encrypted_bytes
            .windows(b"classified-note".len())
            .any(|w| w == b"classified-note"));

        let mut loaded = PdfDocument::load(&encrypted_bytes).unwrap();
        assert!(authenticate_document(&mut loaded, "wrong_password").is_err());

        let (is_owner_u, user_dek) = authenticate_document(&mut loaded, "user_rev6_pass").unwrap();
        assert!(!is_owner_u);
        assert_eq!(user_dek.len(), 32);

        let (is_owner_o, owner_dek) =
            authenticate_document(&mut loaded, "owner_rev6_master").unwrap();
        assert!(is_owner_o);
        assert_eq!(owner_dek.len(), 32);
        assert_eq!(user_dek, owner_dek);

        decrypt_document(&mut loaded, "user_rev6_pass").unwrap();
        let restored = loaded.save_to_vec().unwrap();
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

    #[test]
    fn test_aes256_rev5_encrypt_and_authenticate_roundtrip() {
        let plain = document_with_secret().save_to_vec().unwrap();
        let mut doc = PdfDocument::load(&plain).unwrap();

        let opts = EncryptionOptions {
            user_password: "user_rev5".to_string(),
            owner_password: "owner_rev5".to_string(),
            permissions: PdfPermissions::read_only(),
            revision: EncryptionRevision::Aes256Rev5,
            encrypt_metadata: false,
        };
        encrypt_document(&mut doc, &opts).unwrap();
        assert_eq!(
            get_encryption_revision(&mut doc),
            Some(EncryptionRevision::Aes256Rev5)
        );

        let (is_owner, dek) = authenticate_document(&mut doc, "owner_rev5").unwrap();
        assert!(is_owner);
        assert_eq!(dek.len(), 32);

        decrypt_document(&mut doc, "user_rev5").unwrap();
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

    fn locked_permissions() -> PdfPermissions {
        PdfPermissions {
            print_low_res: false,
            print_high_res: false,
            modify_contents: false,
            copy_extract: false,
            modify_annotations: false,
            fill_forms: false,
            accessibility_extract: true,
            assemble_document: false,
        }
    }

    fn denies(result: PdfResult<impl Sized>) -> bool {
        match result {
            Err(err) => err
                .to_string()
                .contains(crate::security::permissions::PERMISSION_DENIED),
            Ok(_) => false,
        }
    }

    fn allows(result: PdfResult<impl Sized>) {
        if let Err(err) = result {
            assert!(
                !err.to_string()
                    .contains(crate::security::permissions::PERMISSION_DENIED),
                "{err}"
            );
        }
    }

    #[test]
    fn user_password_enforces_stored_permissions() {
        let plain = document_with_secret().save_to_vec().unwrap();
        let mut source = PdfDocument::load(&plain).unwrap();
        let locked = EncryptionOptions {
            user_password: "user_locked".to_string(),
            owner_password: "owner_locked".to_string(),
            permissions: locked_permissions(),
            revision: EncryptionRevision::Aes128,
            encrypt_metadata: true,
        };
        encrypt_document(&mut source, &locked).unwrap();
        let encrypted = source.save_to_vec().unwrap();

        let mut user_doc = PdfDocument::load(&encrypted).unwrap();
        decrypt_document(&mut user_doc, "user_locked").unwrap();
        assert!(denies(crate::ops::search_document(
            &mut user_doc,
            "classified",
            false,
            10
        )));
        assert!(denies(crate::ops::replace_document_text(
            &mut user_doc,
            "classified",
            "open",
            false,
            10
        )));
        assert!(denies(crate::ops::render_page(
            &mut user_doc,
            0,
            crate::ops::RasterFormat::Png,
            72.0
        )));
        assert!(user_doc.save_to_vec().is_ok());
        assert!(denies(encrypt_document(&mut user_doc, &sample_options())));
        assert!(!user_doc.xref.trailer.contains_key("Encrypt"));

        let mut read_only = PdfDocument::load(&plain).unwrap();
        encrypt_document(&mut read_only, &sample_options()).unwrap();
        let read_only_bytes = read_only.save_to_vec().unwrap();
        let mut read_only_user = PdfDocument::load(&read_only_bytes).unwrap();
        decrypt_document(&mut read_only_user, "user123").unwrap();
        assert!(denies(read_only_user.require_copy()));
        assert!(denies(read_only_user.require_modify()));
        allows(crate::ops::render_page(
            &mut read_only_user,
            0,
            crate::ops::RasterFormat::Png,
            72.0,
        ));

        let mut owner_doc = PdfDocument::load(&encrypted).unwrap();
        decrypt_document(&mut owner_doc, "owner_locked").unwrap();
        allows(crate::ops::search_document(
            &mut owner_doc,
            "classified",
            false,
            10,
        ));
        allows(crate::ops::replace_document_text(
            &mut owner_doc,
            "classified",
            "open",
            false,
            10,
        ));
        allows(crate::ops::render_page(
            &mut owner_doc,
            0,
            crate::ops::RasterFormat::Png,
            72.0,
        ));

        let mut open = PdfDocument::load(&plain).unwrap();
        let open_options = EncryptionOptions {
            user_password: "user_open".to_string(),
            owner_password: "owner_open".to_string(),
            permissions: PdfPermissions::default(),
            revision: EncryptionRevision::Aes128,
            encrypt_metadata: true,
        };
        encrypt_document(&mut open, &open_options).unwrap();
        let open_bytes = open.save_to_vec().unwrap();
        let mut open_user = PdfDocument::load(&open_bytes).unwrap();
        decrypt_document(&mut open_user, "user_open").unwrap();
        allows(crate::ops::search_document(
            &mut open_user,
            "classified",
            false,
            10,
        ));
        assert!(open_user.require_security_change().is_ok());
    }
}
