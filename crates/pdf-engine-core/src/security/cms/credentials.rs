//! X.509 credentials for one detached signature.
//!
//! Accepts a PEM certificate with a PKCS#8, PKCS#1, or SEC1 private key, or a
//! PKCS#12 file opened with the supplied password. PKCS#12 integrity is checked
//! before any bag is decrypted. Legacy RC2 and 3DES bags, scrypt, and a missing
//! MAC are rejected. Passwords and private keys are not copied into errors.

use der::asn1::{ContextSpecific, OctetString};
use der::{Any, Decode, Encode, TagNumber, Tagged};
use hmac::{Hmac, Mac};
use pkcs12::kdf::Pkcs12KeyType;
use pkcs12::pfx::{Pfx, Version};
use pkcs12::safe_bag::SafeBag;
use pkcs12::{
    PKCS_12_CERT_BAG_OID, PKCS_12_KEY_BAG_OID, PKCS_12_PKCS8_KEY_BAG_OID,
    PKCS_12_SAFE_CONTENTS_BAG_OID, PKCS_12_X509_CERT_OID,
};
use p256::pkcs8::DecodePrivateKey as DecodeP256Key;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use sha1::Sha1;
use sha2::Sha256;
use spki::DecodePublicKey;
use x509_cert::Certificate;
use zeroize::{Zeroize, Zeroizing};

use super::{crypto, CmsSigningMaterial};
use crate::error::PdfResult;

const MAX_PEM: usize = 64 * 1024;
const MAX_PKCS12: usize = 96 * 1024;
const MAX_PASSWORD: usize = 256;
const MAX_CHAIN: usize = 16;
const MAX_CONTENT_INFOS: usize = 32;
const MAX_BAGS: usize = 64;
const MAX_PLAIN: usize = 256 * 1024;
const MAX_KDF_ITERATIONS: i32 = 250_000;
const MAX_SALT: usize = 64;
const SHA1_OID: der::asn1::ObjectIdentifier = der::asn1::ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
const RSA_OID: der::asn1::ObjectIdentifier =
    der::asn1::ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const EC_OID: der::asn1::ObjectIdentifier = der::asn1::ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const P256_OID: der::asn1::ObjectIdentifier =
    der::asn1::ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");

/// Private key that matches the leaf certificate. Not printable.
pub(crate) enum KeyMaterial {
    Rsa(RsaPrivateKey),
    P256(p256::SecretKey),
}

/// Leaf, optional extra certificates, and the matching private key.
pub(crate) struct Loaded {
    pub(crate) key: KeyMaterial,
    pub(crate) leaf: Certificate,
    pub(crate) chain: Vec<Certificate>,
}

impl Loaded {
    pub(crate) fn key_is_p256(&self) -> bool {
        matches!(self.key, KeyMaterial::P256(_))
    }
}

pub(crate) fn load(material: &CmsSigningMaterial<'_>) -> PdfResult<Loaded> {
    let cert = optional_pem(material.certificate_pem)?;
    let key = optional_pem(material.private_key_pem)?;
    let chain = optional_pem(material.chain_pem)?;
    let has_pem = cert.is_some() || key.is_some() || chain.is_some();
    let has_p12 = material.pkcs12_der.is_some();
    let has_password = material.pkcs12_password.is_some();
    if has_pem && (has_p12 || has_password) {
        return Err(crypto("Provide either PEM credentials or a PKCS#12 file."));
    }
    if has_password && !has_p12 {
        return Err(crypto("Provide either PEM credentials or a PKCS#12 file."));
    }
    if let Some(der) = material.pkcs12_der {
        let password = material
            .pkcs12_password
            .ok_or_else(|| crypto("PKCS#12 password is required."))?;
        return load_pkcs12(der, password);
    }
    match (cert, key) {
        (Some(cert), Some(key)) => load_pem(cert, key, chain),
        _ => Err(crypto("Certificate and private key are required.")),
    }
}

fn optional_pem(value: Option<&str>) -> PdfResult<Option<&str>> {
    let Some(text) = value else {
        return Ok(None);
    };
    if text.len() > MAX_PEM {
        return Err(crypto("Signing material is too large."));
    }
    if text.trim().is_empty() {
        return Ok(None);
    }
    if text.contains("ENCRYPTED") {
        return Err(crypto("Certificate or private key could not be read."));
    }
    Ok(Some(text))
}

fn load_pem(cert_pem: &str, key_pem: &str, chain_pem: Option<&str>) -> PdfResult<Loaded> {
    let key = parse_pem_key(key_pem)?;
    reject_weak_rsa(&key)?;
    let certs = Certificate::load_pem_chain(cert_pem.as_bytes()).map_err(|_| read())?;
    let mut certs = certs.into_iter();
    let leaf = certs.next().ok_or_else(read)?;
    assert_key_matches(&key, &leaf)?;
    let leaf_der = leaf.to_der().map_err(|_| read())?;
    let mut chain = Vec::new();
    for extra in certs {
        push_chain(&mut chain, &leaf_der, extra)?;
    }
    if let Some(chain_pem) = chain_pem {
        let extras = Certificate::load_pem_chain(chain_pem.as_bytes()).map_err(|_| read())?;
        for extra in extras {
            push_chain(&mut chain, &leaf_der, extra)?;
        }
    }
    Ok(Loaded { key, leaf, chain })
}

fn parse_pem_key(pem: &str) -> PdfResult<KeyMaterial> {
    if pem.contains("BEGIN EC PRIVATE KEY") {
        let secret = p256::SecretKey::from_sec1_pem(pem).map_err(|_| read())?;
        return Ok(KeyMaterial::P256(secret));
    }
    if pem.contains("BEGIN RSA PRIVATE KEY") {
        let key = RsaPrivateKey::from_pkcs1_pem(pem).map_err(|_| read())?;
        return Ok(KeyMaterial::Rsa(key));
    }
    if let Ok(key) = RsaPrivateKey::from_pkcs8_pem(pem) {
        return Ok(KeyMaterial::Rsa(key));
    }
    if let Ok(secret) = p256::SecretKey::from_pkcs8_pem(pem) {
        return Ok(KeyMaterial::P256(secret));
    }
    Err(read())
}

fn load_pkcs12(der: &[u8], password: &[u8]) -> PdfResult<Loaded> {
    if der.len() > MAX_PKCS12 || password.len() > MAX_PASSWORD {
        return Err(crypto("Signing material is too large."));
    }
    let password = Zeroizing::new(
        std::str::from_utf8(password)
            .map_err(|_| open())?
            .to_string(),
    );
    let pfx = Pfx::from_der(der).map_err(|_| open())?;
    if pfx.version != Version::V3 || pfx.auth_safe.content_type != const_oid::db::rfc5911::ID_DATA {
        return Err(open());
    }
    let mac = pfx.mac_data.as_ref().ok_or_else(open)?;
    let auth_octet = OctetString::from_der(&pfx.auth_safe.content.to_der().map_err(|_| open())?)
        .map_err(|_| open())?;
    verify_mac(auth_octet.as_bytes(), password.as_str(), mac)?;
    let infos = Vec::<::cms::content_info::ContentInfo>::from_der(auth_octet.as_bytes())
        .map_err(|_| open())?;
    if infos.len() > MAX_CONTENT_INFOS {
        return Err(crypto("Signing material is too large."));
    }
    let mut keys = Vec::new();
    let mut certs = Vec::new();
    for info in &infos {
        let bags = safe_contents(info, password.as_bytes())?;
        walk_bags(&bags, password.as_bytes(), &mut keys, &mut certs, 0)?;
    }
    if keys.len() > 1 {
        return Err(crypto("PKCS#12 contains more than one private key."));
    }
    let key = keys.pop().ok_or_else(|| crypto("PKCS#12 does not contain a private key."))?;
    reject_weak_rsa(&key)?;
    let mut matched = None;
    let mut chain = Vec::new();
    for cert in certs {
        if matched.is_none() && key_matches(&key, &cert) {
            matched = Some(cert);
            continue;
        }
        if let Some(leaf) = &matched {
            let leaf_der = leaf.to_der().map_err(|_| read())?;
            push_chain(&mut chain, &leaf_der, cert)?;
        } else {
            chain.push(cert);
        }
    }
    let leaf = matched.ok_or_else(|| crypto("Private key does not match the certificate."))?;
    assert_key_matches(&key, &leaf)?;
    let leaf_der = leaf.to_der().map_err(|_| read())?;
    chain.retain(|cert| cert.to_der().ok().as_deref() != Some(leaf_der.as_slice()));
    if chain.len() > MAX_CHAIN {
        return Err(crypto("Signing material is too large."));
    }
    Ok(Loaded { key, leaf, chain })
}

fn safe_contents(info: &::cms::content_info::ContentInfo, password: &[u8]) -> PdfResult<Vec<SafeBag>> {
    if info.content_type == const_oid::db::rfc5911::ID_DATA {
        let octet = OctetString::from_der(&info.content.to_der().map_err(|_| open())?).map_err(|_| open())?;
        return Vec::<SafeBag>::from_der(octet.as_bytes()).map_err(|_| open());
    }
    if info.content_type == const_oid::db::rfc5911::ID_ENCRYPTED_DATA {
        let encrypted = ::cms::encrypted_data::EncryptedData::from_der(
            &info.content.to_der().map_err(|_| cipher())?,
        )
        .map_err(|_| cipher())?;
        let plain = decrypt_scheme(
            &encrypted.enc_content_info.content_enc_alg,
            encrypted
                .enc_content_info
                .encrypted_content
                .as_ref()
                .map(|value| value.as_bytes())
                .ok_or_else(cipher)?,
            password,
        )?;
        return Vec::<SafeBag>::from_der(&plain).map_err(|_| open());
    }
    Err(open())
}

fn walk_bags(
    bags: &[SafeBag],
    password: &[u8],
    keys: &mut Vec<KeyMaterial>,
    certs: &mut Vec<Certificate>,
    depth: u8,
) -> PdfResult<()> {
    if bags.len() > MAX_BAGS {
        return Err(crypto("Signing material is too large."));
    }
    for bag in bags {
        if bag.bag_id == PKCS_12_SAFE_CONTENTS_BAG_OID {
            if depth >= 1 {
                return Err(open());
            }
            let inner = explicit_inner(&bag.bag_value)?;
            let nested = Vec::<SafeBag>::from_der(&inner).map_err(|_| open())?;
            walk_bags(&nested, password, keys, certs, depth + 1)?;
            continue;
        }
        if bag.bag_id == PKCS_12_CERT_BAG_OID {
            let inner = explicit_inner(&bag.bag_value)?;
            let cert_bag = pkcs12::cert_type::CertBag::from_der(&inner).map_err(|_| read())?;
            if cert_bag.cert_id != PKCS_12_X509_CERT_OID {
                continue;
            }
            if certs.len() > MAX_CHAIN {
                return Err(crypto("Signing material is too large."));
            }
            let cert = Certificate::from_der(cert_bag.cert_value.as_bytes()).map_err(|_| read())?;
            certs.push(cert);
            continue;
        }
        if bag.bag_id == PKCS_12_KEY_BAG_OID || bag.bag_id == PKCS_12_PKCS8_KEY_BAG_OID {
            if !keys.is_empty() {
                return Err(crypto("PKCS#12 contains more than one private key."));
            }
            let inner = explicit_inner(&bag.bag_value)?;
            let key = if bag.bag_id == PKCS_12_PKCS8_KEY_BAG_OID {
                decrypt_key_bag(&inner, password)?
            } else {
                parse_pki(&inner)?
            };
            keys.push(key);
        }
    }
    Ok(())
}

fn decrypt_key_bag(inner: &[u8], password: &[u8]) -> PdfResult<KeyMaterial> {
    let epki = pkcs8::EncryptedPrivateKeyInfo::from_der(inner).map_err(|_| cipher())?;
    reject_unsupported_scheme(&epki.encryption_algorithm)?;
    let document = epki.decrypt(password).map_err(|_| open())?;
    let mut plain = document.as_bytes().to_vec();
    let parsed = parse_pki(&plain);
    plain.zeroize();
    parsed
}

fn decrypt_scheme(
    algorithm: &spki::AlgorithmIdentifierOwned,
    ciphertext: &[u8],
    password: &[u8],
) -> PdfResult<Vec<u8>> {
    let encoded = algorithm.to_der().map_err(|_| cipher())?;
    let scheme = pkcs5::EncryptionScheme::from_der(&encoded).map_err(|_| cipher())?;
    reject_unsupported_scheme(&scheme)?;
    if ciphertext.len() > MAX_PLAIN + 32 {
        return Err(crypto("Signing material is too large."));
    }
    let pkcs5::EncryptionScheme::Pbes2(params) = &scheme else {
        return Err(cipher());
    };
    let mut plain = params.decrypt(password, ciphertext).map_err(|_| open())?;
    if plain.len() > MAX_PLAIN {
        plain.zeroize();
        return Err(crypto("Signing material is too large."));
    }
    Ok(plain)
}

fn reject_unsupported_scheme(scheme: &pkcs5::EncryptionScheme<'_>) -> PdfResult<()> {
    let pkcs5::EncryptionScheme::Pbes2(params) = scheme else {
        return Err(cipher());
    };
    match &params.kdf {
        pkcs5::pbes2::Kdf::Pbkdf2(kdf) => {
            if kdf.iteration_count == 0
                || kdf.iteration_count > MAX_KDF_ITERATIONS as u32
                || kdf.salt.is_empty()
                || kdf.salt.len() > MAX_SALT
                || kdf.key_length.is_some_and(|len| len == 0 || usize::from(len) > MAX_SALT)
            {
                return Err(cipher());
            }
        }
        _ => return Err(cipher()),
    }
    match params.encryption {
        pkcs5::pbes2::EncryptionScheme::Aes128Cbc { .. }
        | pkcs5::pbes2::EncryptionScheme::Aes192Cbc { .. }
        | pkcs5::pbes2::EncryptionScheme::Aes256Cbc { .. } => Ok(()),
        _ => Err(cipher()),
    }
}

fn verify_mac(authenticated: &[u8], password: &str, mac: &pkcs12::mac_data::MacData) -> PdfResult<()> {
    if mac.iterations < 1 || mac.iterations > MAX_KDF_ITERATIONS {
        return Err(open());
    }
    let salt = mac.mac_salt.as_bytes();
    if salt.is_empty() || salt.len() > MAX_SALT || !null_or_absent(&mac.mac.algorithm.parameters) {
        return Err(open());
    }
    let digest = mac.mac.digest.as_bytes();
    if mac.mac.algorithm.oid == const_oid::db::rfc5912::ID_SHA_256 && digest.len() == 32 {
        let key = Zeroizing::new(
            pkcs12::kdf::derive_key_utf8::<Sha256>(password, salt, Pkcs12KeyType::Mac, mac.iterations, 32)
                .map_err(|_| open())?,
        );
        let mut hasher = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| open())?;
        hasher.update(authenticated);
        hasher.verify_slice(digest).map_err(|_| open())?;
        return Ok(());
    }
    if mac.mac.algorithm.oid == SHA1_OID && digest.len() == 20 {
        let key = Zeroizing::new(
            pkcs12::kdf::derive_key_utf8::<Sha1>(password, salt, Pkcs12KeyType::Mac, mac.iterations, 20)
                .map_err(|_| open())?,
        );
        let mut hasher = Hmac::<Sha1>::new_from_slice(&key).map_err(|_| open())?;
        hasher.update(authenticated);
        hasher.verify_slice(digest).map_err(|_| open())?;
        return Ok(());
    }
    Err(open())
}

fn explicit_inner(bytes: &[u8]) -> PdfResult<Vec<u8>> {
    let ctx = ContextSpecific::<Any>::from_der(bytes).map_err(|_| open())?;
    if ctx.tag_number != TagNumber::N0 {
        return Err(open());
    }
    ctx.value.to_der().map_err(|_| open())
}

fn parse_pki(der: &[u8]) -> PdfResult<KeyMaterial> {
    if let Ok(key) = RsaPrivateKey::from_pkcs8_der(der) {
        return Ok(KeyMaterial::Rsa(key));
    }
    if let Ok(secret) = p256::SecretKey::from_pkcs8_der(der) {
        return Ok(KeyMaterial::P256(secret));
    }
    Err(read())
}

fn push_chain(chain: &mut Vec<Certificate>, leaf_der: &[u8], extra: Certificate) -> PdfResult<()> {
    let der = extra.to_der().map_err(|_| read())?;
    if der.as_slice() == leaf_der {
        return Ok(());
    }
    if chain.iter().any(|cert| cert.to_der().ok().as_deref() == Some(der.as_slice())) {
        return Ok(());
    }
    if chain.len() >= MAX_CHAIN {
        return Err(crypto("Signing material is too large."));
    }
    chain.push(extra);
    Ok(())
}

fn reject_weak_rsa(key: &KeyMaterial) -> PdfResult<()> {
    let KeyMaterial::Rsa(private) = key else {
        return Ok(());
    };
    let public = rsa::RsaPublicKey::from(private);
    if public.n().bits() < 2048 {
        return Err(crypto("Unsupported certificate public key."));
    }
    Ok(())
}

fn assert_key_matches(key: &KeyMaterial, cert: &Certificate) -> PdfResult<()> {
    if key_matches(key, cert) {
        Ok(())
    } else if key_kind(cert).is_err() {
        Err(crypto("Unsupported certificate public key."))
    } else {
        Err(crypto("Private key does not match the certificate."))
    }
}

fn key_matches(key: &KeyMaterial, cert: &Certificate) -> bool {
    match (key, key_kind(cert).ok()) {
        (KeyMaterial::Rsa(private), Some(Kind::Rsa)) => {
            let Some(public) = rsa_public(cert) else {
                return false;
            };
            if public.n().bits() < 2048 {
                return false;
            }
            let mine = rsa::RsaPublicKey::from(private);
            public.n() == mine.n() && public.e() == mine.e()
        }
        (KeyMaterial::P256(secret), Some(Kind::P256)) => {
            let Some(cert_key) = p256_public(cert) else {
                return false;
            };
            let mine = p256::ecdsa::SigningKey::from(secret.clone());
            cert_key == *mine.verifying_key()
        }
        _ => false,
    }
}

enum Kind {
    Rsa,
    P256,
}

fn key_kind(cert: &Certificate) -> Result<Kind, ()> {
    let algorithm = &cert.tbs_certificate.subject_public_key_info.algorithm;
    if algorithm.oid == RSA_OID {
        return Ok(Kind::Rsa);
    }
    if algorithm.oid == EC_OID {
        let Some(params) = &algorithm.parameters else {
            return Err(());
        };
        let Ok(encoded) = params.to_der() else {
            return Err(());
        };
        let Ok(curve) = der::asn1::ObjectIdentifier::from_der(&encoded) else {
            return Err(());
        };
        if curve == P256_OID {
            return Ok(Kind::P256);
        }
    }
    Err(())
}

fn rsa_public(cert: &Certificate) -> Option<rsa::RsaPublicKey> {
    let der = cert.tbs_certificate.subject_public_key_info.to_der().ok()?;
    rsa::RsaPublicKey::from_public_key_der(&der).ok()
}

fn p256_public(cert: &Certificate) -> Option<p256::ecdsa::VerifyingKey> {
    let der = cert.tbs_certificate.subject_public_key_info.to_der().ok()?;
    p256::ecdsa::VerifyingKey::from_public_key_der(&der).ok()
}

fn null_or_absent(params: &Option<Any>) -> bool {
    match params {
        None => true,
        Some(any) => any.tag() == der::Tag::Null,
    }
}

fn read() -> crate::error::PdfError {
    crypto("Certificate or private key could not be read.")
}

fn open() -> crate::error::PdfError {
    crypto("PKCS#12 could not be opened.")
}

fn cipher() -> crate::error::PdfError {
    crypto("PKCS#12 bag uses an unsupported cipher.")
}
