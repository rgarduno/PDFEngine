//! PKCS#7 detached signatures for PDF (`/SubFilter /adbe.pkcs7.detached`).
//!
//! A signature is produced only when a certificate and a matching private key are
//! supplied. Verification checks that CMS against the certificate embedded in it.
//! It does not decide whether a viewer trusts the certificate, and it does not
//! claim that a particular reader or authority accepts the file.
//!
//! The byte range covers the saved file except the hex digits of `/Contents`.
//! A later full save rewrites offsets and leaves this signature invalid.
//! An RFC 3161 token is stored as an unsigned attribute, so embedding it does
//! not move `/ByteRange`.

mod credentials;
mod signed;
mod timestamp;

use sha2::Digest;

use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::security::signatures::{
    contents_hex_span, stamp_signature, verify_document_signatures, widen_byte_range_placeholder,
    write_contents_hex, write_covered_byte_range, DigitalSignatureConfig, VerifiedSignature,
};

pub(crate) use signed::cms_byte_range_valid;

const PKCS7_FILTER: &str = "Adobe.PPKLite";
const PKCS7_SUBFILTER: &str = "adbe.pkcs7.detached";
pub(crate) const PKCS7_MARK: &[u8] = b"/SubFilter /adbe.pkcs7.detached";
const MAX_TOKEN: usize = 64 * 1024;
const MAX_HOLE: usize = 64 * 1024;
const ECDSA_SLACK: usize = 128;
const TIMESTAMP_RESERVE: usize = 12_288;

/// PEM or PKCS#12 material used to build one detached signature.
///
/// The lifetime borrows the caller's buffers. This type does not implement
/// `Debug`, so a log of the value cannot print a key or a password.
pub struct CmsSigningMaterial<'a> {
    /// PEM certificate. The leaf whose public key matches the private key.
    pub certificate_pem: Option<&'a str>,
    /// PEM PKCS#8 or PKCS#1 private key. Encrypted PEM is rejected.
    pub private_key_pem: Option<&'a str>,
    /// Extra PEM certificates stored beside the leaf. They are not a trust path.
    pub chain_pem: Option<&'a str>,
    /// PKCS#12 / PFX bytes (`.p12` or `.pfx`).
    pub pkcs12_der: Option<&'a [u8]>,
    /// Password for `pkcs12_der`. An empty slice is a password that was supplied.
    pub pkcs12_password: Option<&'a [u8]>,
    /// RFC 3161 token to embed after the signature value is known.
    pub timestamp_token: Option<&'a [u8]>,
    /// Reserve room in `/Contents` for a token that will be embedded later.
    pub reserve_timestamp: bool,
}

/// Builds a detached PKCS#7 signature and assigns it only when it verifies.
pub fn sign_document_cms(
    doc: &mut PdfDocument,
    config: &DigitalSignatureConfig,
    material: &CmsSigningMaterial<'_>,
) -> PdfResult<VerifiedSignature> {
    if let Some(token) = material.timestamp_token {
        if token.is_empty() || token.len() > MAX_TOKEN {
            return Err(crypto("Signing material is too large."));
        }
    }
    let creds = credentials::load(material)?;
    let hole = hole_size(&creds, material)?;
    let mut working = doc.clone();
    let field_name = stamp_signature(
        &mut working,
        config,
        PKCS7_FILTER,
        PKCS7_SUBFILTER,
        &vec![0u8; hole],
        "PKCS#7 detached",
    )?;
    let limits = working.limits.clone();
    let mut saved = working.save_to_vec()?;
    widen_byte_range_placeholder(&mut saved, PKCS7_MARK)?;
    let (hex_start, hex_end) = write_covered_byte_range(&mut saved, PKCS7_MARK)?;
    let digest = covered_digest(&saved, hex_start, hex_end);
    let mut cms_der = signed::build_detached(&creds, digest.as_slice())?;
    if let Some(token) = material.timestamp_token {
        cms_der = signed::insert_timestamp(&cms_der, token)?;
    }
    write_contents_hex(&mut saved, hex_start, hex_end, &cms_der)?;
    let loaded = PdfDocument::load_with_limits(&saved, limits)?;
    let verified = verify_document_signatures(&loaded)
        .into_iter()
        .rev()
        .find(|sig| sig.field_name == field_name)
        .ok_or_else(|| crypto("Signature could not be verified."))?;
    if !verified.byte_range_valid {
        return Err(crypto("Signature could not be verified."));
    }
    *doc = loaded;
    Ok(verified)
}

/// DER `TimeStampReq` over the signature value of the last valid PKCS#7.
///
/// Read-only. The caller sends the bytes to a time-stamping authority.
pub fn cms_timestamp_request(doc: &PdfDocument) -> PdfResult<Vec<u8>> {
    let signature = verify_document_signatures(doc)
        .into_iter()
        .rev()
        .find(|sig| sig.sub_filter == PKCS7_SUBFILTER && sig.byte_range_valid)
        .ok_or_else(|| crypto("Signature could not be verified."))?;
    let contents = decode_hex(&signature.contents_hex).map_err(|_| rejected())?;
    let der_len = signed::der_len(&contents).ok_or_else(rejected)?;
    if contents[der_len..].iter().any(|byte| *byte != 0) {
        return Err(rejected());
    }
    let value = signed::signature_value(&contents[..der_len])?;
    timestamp::request_for(&value)
}

/// Writes an RFC 3161 token into the existing `/Contents` hole.
///
/// Does not rewrite the file. The document is replaced only when the token
/// matches this signature and the byte range still verifies.
pub fn embed_cms_timestamp(doc: &mut PdfDocument, token: &[u8]) -> PdfResult<VerifiedSignature> {
    if token.is_empty() || token.len() > MAX_TOKEN {
        return Err(rejected());
    }
    let limits = doc.limits.clone();
    let mut file = doc.raw_data().to_vec();
    let (hex_start, hex_end) = contents_hex_span(&file, PKCS7_MARK)?;
    let mut contents = decode_hex(std::str::from_utf8(&file[hex_start..hex_end]).map_err(|_| rejected())?)
        .map_err(|_| rejected())?;
    let der_len = signed::der_len(&contents).ok_or_else(rejected)?;
    if contents[der_len..].iter().any(|byte| *byte != 0) {
        return Err(rejected());
    }
    contents.truncate(der_len);
    let updated = signed::insert_timestamp(&contents, token)?;
    write_contents_hex(&mut file, hex_start, hex_end, &updated)?;
    let loaded = PdfDocument::load_with_limits(&file, limits)?;
    let verified = verify_document_signatures(&loaded)
        .into_iter()
        .rev()
        .find(|sig| sig.sub_filter == PKCS7_SUBFILTER && sig.byte_range_valid)
        .ok_or_else(rejected)?;
    *doc = loaded;
    Ok(verified)
}

fn hole_size(creds: &credentials::Loaded, material: &CmsSigningMaterial<'_>) -> PdfResult<usize> {
    let probe = signed::build_detached(creds, &[0u8; 32])?;
    let mut hole = probe.len();
    if creds.key_is_p256() {
        hole = hole.saturating_add(ECDSA_SLACK);
    }
    if material.reserve_timestamp || material.timestamp_token.is_some() {
        let mut reserve = 0usize;
        if material.reserve_timestamp {
            reserve = TIMESTAMP_RESERVE;
        }
        if let Some(token) = material.timestamp_token {
            reserve = reserve.max(token.len().saturating_add(512));
        }
        hole = hole.saturating_add(reserve);
    }
    if hole == 0 || hole > MAX_HOLE {
        return Err(crypto("Signing material is too large."));
    }
    Ok(hole)
}

fn covered_digest(file: &[u8], hex_start: usize, hex_end: usize) -> [u8; 32] {
    let mut covered = Vec::with_capacity(hex_start + file.len().saturating_sub(hex_end));
    covered.extend_from_slice(&file[..hex_start]);
    covered.extend_from_slice(&file[hex_end..]);
    let digest = sha2::Sha256::digest(&covered);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub(crate) fn decode_hex(text: &str) -> Result<Vec<u8>, ()> {
    if text.len() % 2 != 0 {
        return Err(());
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let hi = hex_val(bytes[index])?;
        let lo = hex_val(bytes[index + 1])?;
        out.push((hi << 4) | lo);
        index += 2;
    }
    Ok(out)
}

fn hex_val(byte: u8) -> Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(()),
    }
}

pub(crate) fn crypto(message: &str) -> PdfError {
    PdfError::CryptographyError(message.to_string())
}

pub(crate) fn rejected() -> PdfError {
    crypto("Timestamp token was rejected.")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::time::Duration;

    use der::EncodePem;
    use pem_rfc7468::LineEnding;
    use pkcs8::EncodePrivateKey;
    use rand::rngs::OsRng;
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::RsaPrivateKey;
    use sha2::Sha256;
    use signature::Keypair;
    use spki::SubjectPublicKeyInfoOwned;
    use x509_cert::builder::{Builder, CertificateBuilder, Profile};
    use x509_cert::name::Name;
    use x509_cert::serial_number::SerialNumber;
    use x509_cert::time::Validity;
    use x509_cert::Certificate;

    use super::*;
    use crate::security::signatures::sign_document;

    fn config() -> DigitalSignatureConfig {
        DigitalSignatureConfig {
            signer_name: "Lic. Roberto Garduño".to_string(),
            ..DigitalSignatureConfig::default()
        }
    }

    fn page() -> PdfDocument {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        let off1 = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let off2 = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        let off3 = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n",
        );
        let off4 = pdf.len();
        pdf.extend_from_slice(
            b"4 0 obj\n<< /Length 20 >>\nstream\nq\n(Page 1) Tj\nQ\nendstream\nendobj\n",
        );
        let xref_offset = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n");
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off3).as_bytes());
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off4).as_bytes());
        pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
        pdf.extend_from_slice(format!("startxref\n{}\n%%EOF\n", xref_offset).as_bytes());
        PdfDocument::load(&pdf).expect("fixture")
    }

    fn rsa_key() -> RsaPrivateKey {
        RsaPrivateKey::new(&mut OsRng, 2048).expect("rsa")
    }

    fn issue_rsa(key: &RsaPrivateKey, cn: &str, serial: u16) -> Certificate {
        let signer = rsa::pkcs1v15::SigningKey::<Sha256>::new(key.clone());
        let spki = SubjectPublicKeyInfoOwned::from_key(signer.verifying_key()).unwrap();
        let validity = Validity::from_now(Duration::from_secs(86_400)).unwrap();
        let name = Name::from_str(cn).unwrap();
        CertificateBuilder::new(
            Profile::Root,
            SerialNumber::from(serial),
            validity,
            name,
            spki,
            &signer,
        )
        .unwrap()
        .build::<rsa::pkcs1v15::Signature>()
        .unwrap()
    }

    fn issue_p256(secret: &p256::SecretKey, cn: &str) -> (p256::ecdsa::SigningKey, Certificate) {
        let signer = p256::ecdsa::SigningKey::from(secret.clone());
        let spki = SubjectPublicKeyInfoOwned::from_key(*signer.verifying_key()).unwrap();
        let validity = Validity::from_now(Duration::from_secs(86_400)).unwrap();
        let name = Name::from_str(cn).unwrap();
        let cert = CertificateBuilder::new(
            Profile::Root,
            SerialNumber::from(7u16),
            validity,
            name,
            spki,
            &signer,
        )
        .unwrap()
        .build::<p256::ecdsa::DerSignature>()
        .unwrap();
        (signer, cert)
    }

    fn pem_material<'a>(
        cert: &'a str,
        key: &'a str,
        chain: Option<&'a str>,
    ) -> CmsSigningMaterial<'a> {
        CmsSigningMaterial {
            certificate_pem: Some(cert),
            private_key_pem: Some(key),
            chain_pem: chain,
            pkcs12_der: None,
            pkcs12_password: None,
            timestamp_token: None,
            reserve_timestamp: false,
        }
    }

    fn err_text(err: PdfError) -> String {
        match err {
            PdfError::CryptographyError(text) => text,
            other => panic!("unexpected {other}"),
        }
    }

    fn must_fail<T>(result: PdfResult<T>) -> String {
        match result {
            Err(err) => err_text(err),
            Ok(_) => panic!("expected a cryptography error"),
        }
    }

    #[test]
    fn rsa_pem_signature_verifies_and_a_rewrite_invalidates_it() {
        let key = rsa_key();
        let cert = issue_rsa(&key, "CN=PDFEngine", 1);
        let other = issue_rsa(&rsa_key(), "CN=Other", 2);
        let cert_pem = cert.to_pem(LineEnding::LF).unwrap();
        let key_pem = key.to_pkcs8_pem(LineEnding::LF).unwrap();
        let chain_pem = other.to_pem(LineEnding::LF).unwrap();
        let key_der = key.to_pkcs8_der().unwrap();
        let mut doc = page();
        let sig = sign_document_cms(
            &mut doc,
            &config(),
            &pem_material(&cert_pem, key_pem.as_str(), Some(chain_pem.as_str())),
        )
        .unwrap();
        assert_eq!(sig.sub_filter, "adbe.pkcs7.detached");
        assert_eq!(sig.signer_name, "Lic. Roberto Garduño");
        assert!(sig.byte_range_valid);
        assert!(sig.contents_hex.len() > 64);
        let raw = doc.raw_data();
        assert!(raw.windows(PKCS7_MARK.len()).any(|w| w == PKCS7_MARK));
        assert!(raw.windows(b"/Filter /Adobe.PPKLite".len()).any(|w| w == b"/Filter /Adobe.PPKLite"));
        assert!(!raw.windows(b"/PDFEngine.sha256".len()).any(|w| w == b"/PDFEngine.sha256"));
        assert!(!raw.windows(key_der.as_bytes().len()).any(|w| w == key_der.as_bytes()));
        assert!(raw.windows(b"PKCS#7 detached".len()).any(|w| w == b"PKCS#7 detached"));

        let mut tampered = raw.to_vec();
        tampered[10] ^= 0x01;
        let tampered_doc = PdfDocument::load(&tampered).unwrap();
        assert!(!verify_document_signatures(&tampered_doc)[0].byte_range_valid);

        let rewritten = PdfDocument::load(&doc.save_to_vec().unwrap()).unwrap();
        assert!(!verify_document_signatures(&rewritten)[0].byte_range_valid);
    }

    #[test]
    fn pkcs1_pem_and_sec1_are_accepted() {
        let key = rsa_key();
        let cert = issue_rsa(&key, "CN=PDFEngine", 3);
        let cert_pem = cert.to_pem(LineEnding::LF).unwrap();
        let pkcs1 = key.to_pkcs1_pem(LineEnding::LF).unwrap();
        let mut doc = page();
        let sig = sign_document_cms(&mut doc, &config(), &pem_material(&cert_pem, pkcs1.as_str(), None)).unwrap();
        assert!(sig.byte_range_valid);

        let secret = p256::SecretKey::random(&mut OsRng);
        let sec1 = secret.to_sec1_pem(LineEnding::LF).unwrap();
        let (_signer, cert) = issue_p256(&secret, "CN=PDFEngineP256");
        let cert_pem = cert.to_pem(LineEnding::LF).unwrap();
        let mut doc = page();
        let sig = sign_document_cms(&mut doc, &config(), &pem_material(&cert_pem, sec1.as_str(), None)).unwrap();
        assert!(sig.byte_range_valid);
        assert_eq!(sig.sub_filter, "adbe.pkcs7.detached");
    }

    #[test]
    fn mismatched_key_and_encrypted_pem_are_rejected() {
        let key = rsa_key();
        let cert = issue_rsa(&rsa_key(), "CN=Other", 4);
        let cert_pem = cert.to_pem(LineEnding::LF).unwrap();
        let key_pem = key.to_pkcs8_pem(LineEnding::LF).unwrap();
        let err = must_fail(credentials::load(&pem_material(&cert_pem, key_pem.as_str(), None)));
        assert_eq!(err, "Private key does not match the certificate.");

        let encrypted = "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n";
        let err = must_fail(credentials::load(&pem_material(&cert_pem, encrypted, None)));
        assert_eq!(err, "Certificate or private key could not be read.");
        assert!(!err.contains("AAAA"));
    }

    #[test]
    fn pkcs12_roundtrip_rejects_a_wrong_password_without_echoing_it() {
        let password = b"correct-horse";
        let key = rsa_key();
        let cert = issue_rsa(&key, "CN=PDFEngine", 5);
        let p12 = build_pfx(&key, &cert, std::str::from_utf8(password).unwrap(), true, Some(2048));
        let mut doc = page();
        let sig = sign_document_cms(
            &mut doc,
            &config(),
            &CmsSigningMaterial {
                certificate_pem: None,
                private_key_pem: None,
                chain_pem: None,
                pkcs12_der: Some(&p12),
                pkcs12_password: Some(password),
                timestamp_token: None,
                reserve_timestamp: false,
            },
        )
        .unwrap();
        assert!(sig.byte_range_valid);

        let err = must_fail(credentials::load(&CmsSigningMaterial {
            certificate_pem: None,
            private_key_pem: None,
            chain_pem: None,
            pkcs12_der: Some(&p12),
            pkcs12_password: Some(b"wrong-password"),
            timestamp_token: None,
            reserve_timestamp: false,
        }));
        assert_eq!(err, "PKCS#12 could not be opened.");
        assert!(!err.contains("wrong-password"));
        assert!(!err.contains("correct-horse"));
    }

    #[test]
    fn pkcs12_without_a_mac_or_with_a_huge_iteration_count_fails_closed() {
        let key = rsa_key();
        let cert = issue_rsa(&key, "CN=PDFEngine", 6);
        let p12 = build_pfx(&key, &cert, "pw", true, None);
        let err = must_fail(credentials::load(&CmsSigningMaterial {
            certificate_pem: None,
            private_key_pem: None,
            chain_pem: None,
            pkcs12_der: Some(&p12),
            pkcs12_password: Some(b"pw"),
            timestamp_token: None,
            reserve_timestamp: false,
        }));
        assert_eq!(err, "PKCS#12 could not be opened.");

        let slow = build_pfx(&key, &cert, "pw", true, Some(9_000_000));
        let err = must_fail(credentials::load(&CmsSigningMaterial {
            certificate_pem: None,
            private_key_pem: None,
            chain_pem: None,
            pkcs12_der: Some(&slow),
            pkcs12_password: Some(b"pw"),
            timestamp_token: None,
            reserve_timestamp: false,
        }));
        assert_eq!(err, "PKCS#12 could not be opened.");
    }

    #[test]
    fn empty_pkcs12_password_opens() {
        let key = rsa_key();
        let cert = issue_rsa(&key, "CN=PDFEngine", 8);
        let p12 = build_pfx(&key, &cert, "", true, Some(1000));
        credentials::load(&CmsSigningMaterial {
            certificate_pem: None,
            private_key_pem: None,
            chain_pem: None,
            pkcs12_der: Some(&p12),
            pkcs12_password: Some(b""),
            timestamp_token: None,
            reserve_timestamp: false,
        })
        .unwrap();
    }

    #[test]
    fn timestamp_reserve_embed_and_imprint_mismatch() {
        let secret = p256::SecretKey::random(&mut OsRng);
        let key_pem = secret.to_pkcs8_pem(LineEnding::LF).unwrap();
        let (signer, cert) = issue_p256(&secret, "CN=PDFEngineP256");
        let cert_pem = cert.to_pem(LineEnding::LF).unwrap();
        let mut doc = page();
        let material = CmsSigningMaterial {
            certificate_pem: Some(cert_pem.as_str()),
            private_key_pem: Some(key_pem.as_str()),
            chain_pem: None,
            pkcs12_der: None,
            pkcs12_password: None,
            timestamp_token: None,
            reserve_timestamp: true,
        };
        let signed_sig = sign_document_cms(&mut doc, &config(), &material).unwrap();
        assert!(signed_sig.byte_range_valid);
        let request = cms_timestamp_request(&doc).unwrap();
        assert!(!request.is_empty());

        let contents = decode_hex(&signed_sig.contents_hex).unwrap();
        let der_len = signed::der_len(&contents).unwrap();
        let signature = signed::signature_value(&contents[..der_len]).unwrap();
        let tsa_secret = p256::SecretKey::random(&mut OsRng);
        let (tsa_signer, tsa_cert) = issue_p256(&tsa_secret, "CN=PDFEngineTSA");
        let token = timestamp::mint_token(&tsa_signer, &tsa_cert, &signature).unwrap();
        let stamped = embed_cms_timestamp(&mut doc, &token).unwrap();
        assert!(stamped.byte_range_valid);
        assert!(stamped.contents_hex.len() > signed_sig.contents_hex.trim_end_matches('0').len());

        let mut other = config();
        other.signer_name = "Otra persona".to_string();
        let mut fresh = page();
        sign_document_cms(&mut fresh, &other, &material).unwrap();
        let err = must_fail(embed_cms_timestamp(&mut fresh, &token));
        assert_eq!(err, "Timestamp token was rejected.");
        let _ = signer;
    }

    #[test]
    fn attestation_path_stays_free_of_pkcs7() {
        let mut doc = page();
        let sig = sign_document(&mut doc, &config()).unwrap();
        assert_eq!(sig.sub_filter, "PDFEngine.sha256");
        assert!(sig.byte_range_valid);
        assert!(!doc.raw_data().windows(b"adbe.pkcs7.detached".len()).any(|w| w == b"adbe.pkcs7.detached"));
    }

    fn build_pfx(
        key: &RsaPrivateKey,
        cert: &Certificate,
        password: &str,
        shroud: bool,
        mac_iterations: Option<i32>,
    ) -> Vec<u8> {
        use der::asn1::{ContextSpecificRef, OctetString};
        use der::{AnyRef, Encode, TagMode, TagNumber};
        use hmac::{Hmac, Mac};
        use pkcs12::cert_type::CertBag;
        use pkcs12::digest_info::DigestInfo;
        use pkcs12::kdf::{derive_key_utf8, Pkcs12KeyType};
        use pkcs12::mac_data::MacData;
        use pkcs12::pfx::{Pfx, Version};
        use pkcs12::safe_bag::SafeBag;
        use pkcs12::{PKCS_12_CERT_BAG_OID, PKCS_12_KEY_BAG_OID, PKCS_12_PKCS8_KEY_BAG_OID, PKCS_12_X509_CERT_OID};
        use pkcs5::pbes2::Parameters;
        use spki::AlgorithmIdentifierOwned;

        let key_der = key.to_pkcs8_der().unwrap();
        let key_bag_der;
        let key_bag_id;
        let ciphertext;
        let salt = [9u8; 8];
        let iv = [4u8; 16];
        if shroud {
            let params = Parameters::pbkdf2_sha256_aes256cbc(1000, &salt, &iv).unwrap();
            ciphertext = params.encrypt(password.as_bytes(), key_der.as_bytes()).unwrap();
            let scheme = pkcs5::EncryptionScheme::from(params);
            let epki = pkcs8::EncryptedPrivateKeyInfo {
                encryption_algorithm: scheme,
                encrypted_data: ciphertext.as_slice(),
            };
            key_bag_der = epki.to_der().unwrap();
            key_bag_id = PKCS_12_PKCS8_KEY_BAG_OID;
        } else {
            ciphertext = Vec::new();
            key_bag_der = key_der.as_bytes().to_vec();
            key_bag_id = PKCS_12_KEY_BAG_OID;
        }
        let _ = ciphertext;
        let cert_bag = CertBag {
            cert_id: PKCS_12_X509_CERT_OID,
            cert_value: OctetString::new(cert.to_der().unwrap()).unwrap(),
        };
        let bags = vec![
            SafeBag {
                bag_id: key_bag_id,
                bag_value: key_bag_der,
                bag_attributes: None,
            },
            SafeBag {
                bag_id: PKCS_12_CERT_BAG_OID,
                bag_value: cert_bag.to_der().unwrap(),
                bag_attributes: None,
            },
        ];
        let safe_der = bags.to_der().unwrap();
        let inner = data_info(&safe_der);
        let auth_der = vec![inner].to_der().unwrap();
        let outer = data_info(&auth_der);
        let mac_data = mac_iterations.map(|iterations| {
            let mac_salt = [3u8; 8];
            let mac_key = derive_key_utf8::<Sha256>(password, &mac_salt, Pkcs12KeyType::Mac, iterations.max(1), 32).unwrap();
            let mut mac = Hmac::<Sha256>::new_from_slice(&mac_key).unwrap();
            mac.update(&auth_der);
            let digest = mac.finalize().into_bytes();
            MacData {
                mac: DigestInfo {
                    algorithm: AlgorithmIdentifierOwned {
                        oid: const_oid::db::rfc5912::ID_SHA_256,
                        parameters: None,
                    },
                    digest: OctetString::new(digest.as_slice()).unwrap(),
                },
                mac_salt: OctetString::new(&mac_salt).unwrap(),
                iterations,
            }
        });
        let pfx = Pfx {
            version: Version::V3,
            auth_safe: outer,
            mac_data,
        };
        let _ = (ContextSpecificRef::<AnyRef>::from, TagMode::Explicit, TagNumber::N0);
        pfx.to_der().unwrap()
    }

    fn data_info(inner: &[u8]) -> ::cms::content_info::ContentInfo {
        use der::asn1::OctetString;
        use der::{Any, Decode, Encode};
        let octet = OctetString::new(inner).unwrap().to_der().unwrap();
        ::cms::content_info::ContentInfo {
            content_type: const_oid::db::rfc5911::ID_DATA,
            content: Any::from_der(&octet).unwrap(),
        }
    }
}
