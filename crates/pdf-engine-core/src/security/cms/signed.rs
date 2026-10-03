//! Detached CMS (`SignedData`) for one PDF byte range.
//!
//! The signature covers the DER of the signed attributes with the SET tag.
//! An RFC 3161 token is an unsigned attribute, so adding it does not change
//! the bytes that were signed. Verification checks the certificate embedded
//! in this CMS. It does not decide trust.

use der::asn1::{ObjectIdentifier, OctetString, SetOfVec};
use der::{Any, AnyRef, Decode, Encode, Sequence, Tagged};
use rsa::traits::PublicKeyParts;
use sha2::Digest;
use signature::{Keypair, Signer, Verifier};
use spki::{AlgorithmIdentifierOwned, DecodePublicKey, DynSignatureAlgorithmIdentifier, SignatureBitStringEncoding};
use x509_cert::attr::Attribute;
use x509_cert::Certificate;

use super::credentials::{KeyMaterial, Loaded};
use super::timestamp;
use super::{crypto, rejected};
use crate::error::PdfResult;

const SHA256_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const ECDSA_WITH_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const ESS_CERT_V2: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");
const TIME_STAMP_TOKEN: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.14");

#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct EssCertIdV2 {
    #[asn1(optional = "true")]
    hash_algorithm: Option<AlgorithmIdentifierOwned>,
    cert_hash: OctetString,
    #[asn1(optional = "true")]
    issuer_serial: Option<Any>,
}

#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct SigningCertificateV2 {
    certs: Vec<EssCertIdV2>,
    #[asn1(optional = "true")]
    policies: Option<Any>,
}

/// CMS for a detached PDF byte range. `digest` is SHA-256 of the covered bytes.
pub(crate) fn build_detached(creds: &Loaded, digest: &[u8]) -> PdfResult<Vec<u8>> {
    if digest.len() != 32 {
        return Err(failed());
    }
    let eci = ::cms::signed_data::EncapsulatedContentInfo {
        econtent_type: const_oid::db::rfc5911::ID_DATA,
        econtent: None,
    };
    match &creds.key {
        KeyMaterial::Rsa(key) => {
            let signer = rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new(key.clone());
            sign_eci::<_, rsa::pkcs1v15::Signature>(&signer, &creds.leaf, &creds.chain, &eci, Some(digest))
        }
        KeyMaterial::P256(secret) => {
            let signer = p256::ecdsa::SigningKey::from(secret.clone());
            sign_eci::<_, p256::ecdsa::DerSignature>(&signer, &creds.leaf, &creds.chain, &eci, Some(digest))
        }
    }
}

/// Signs `eci`. When `external_digest` is set, `econtent` must be absent.
pub(crate) fn sign_eci<S, Sig>(
    signer: &S,
    cert: &Certificate,
    chain: &[Certificate],
    eci: &::cms::signed_data::EncapsulatedContentInfo,
    external_digest: Option<&[u8]>,
) -> PdfResult<Vec<u8>>
where
    S: Keypair + DynSignatureAlgorithmIdentifier + Signer<Sig>,
    Sig: SignatureBitStringEncoding,
{
    if let Some(digest) = external_digest {
        if digest.len() != 32 {
            return Err(failed());
        }
    }
    let digest_alg = sha256_algorithm();
    let sid = ::cms::signed_data::SignerIdentifier::IssuerAndSerialNumber(
        ::cms::cert::IssuerAndSerialNumber {
            issuer: cert.tbs_certificate.issuer.clone(),
            serial_number: cert.tbs_certificate.serial_number.clone(),
        },
    );
    // Do not pre-insert content-type or message-digest. cms 0.2 compares a
    // pre-inserted content-type attribute OID with eContentType and rejects it.
    let mut info_builder = ::cms::builder::SignerInfoBuilder::new(
        signer,
        sid,
        digest_alg.clone(),
        eci,
        external_digest,
    )
    .map_err(|_| failed())?;
    let signing_time = ::cms::builder::create_signing_time_attribute().map_err(|_| failed())?;
    info_builder
        .add_signed_attribute(signing_time)
        .map_err(|_| failed())?;
    info_builder
        .add_signed_attribute(ess_attribute(cert)?)
        .map_err(|_| failed())?;

    let mut builder = ::cms::builder::SignedDataBuilder::new(eci);
    builder.add_digest_algorithm(digest_alg).map_err(|_| failed())?;
    builder
        .add_certificate(::cms::cert::CertificateChoices::Certificate(cert.clone()))
        .map_err(|_| failed())?;
    for extra in chain {
        builder
            .add_certificate(::cms::cert::CertificateChoices::Certificate(extra.clone()))
            .map_err(|_| failed())?;
    }
    builder
        .add_signer_info::<S, Sig>(info_builder)
        .map_err(|_| failed())?;
    builder.build().map_err(|_| failed())?.to_der().map_err(|_| failed())
}

/// Length of the leading DER value, ignoring trailing zero padding.
pub(crate) fn der_len(contents: &[u8]) -> Option<usize> {
    if contents.first() != Some(&0x30) {
        return None;
    }
    let len_byte = *contents.get(1)?;
    if len_byte == 0x80 {
        return None;
    }
    let (header, body) = if len_byte < 0x80 {
        (2usize, usize::from(len_byte))
    } else {
        let nbytes = usize::from(len_byte & 0x7f);
        if nbytes == 0 || nbytes > 3 || contents.len() < 2 + nbytes {
            return None;
        }
        let mut body = 0usize;
        for byte in &contents[2..2 + nbytes] {
            body = body.checked_shl(8)?.checked_add(usize::from(*byte))?;
        }
        if body < 0x80 || contents[2] == 0 {
            return None;
        }
        (2 + nbytes, body)
    };
    let total = header.checked_add(body)?;
    if total > contents.len() {
        None
    } else {
        Some(total)
    }
}

/// Signature value octet string from a CMS `ContentInfo`.
pub(crate) fn signature_value(cms_der: &[u8]) -> PdfResult<Vec<u8>> {
    let (_, signer) = one_signer(cms_der).ok_or_else(rejected)?;
    Ok(signer.signature.as_bytes().to_vec())
}

/// Stores `token` as an unsigned timestamp attribute inside the existing CMS.
pub(crate) fn insert_timestamp(cms_der: &[u8], token: &[u8]) -> PdfResult<Vec<u8>> {
    if token.is_empty() || token.len() > 64 * 1024 {
        return Err(rejected());
    }
    let (mut signed, mut signer) = one_signer(cms_der).ok_or_else(rejected)?;
    if has_timestamp(&signer) {
        return Err(crypto("A timestamp is already present."));
    }
    if !timestamp::covers(token, signer.signature.as_bytes()) {
        return Err(rejected());
    }
    let token_der = timestamp::content_info_der(token).ok_or_else(rejected)?;
    let value = Any::from_der(&token_der).map_err(|_| rejected())?;
    let mut values = SetOfVec::new();
    values.insert(value).map_err(|_| rejected())?;
    let mut attrs = signer
        .unsigned_attrs
        .take()
        .map(SetOfVec::into_vec)
        .unwrap_or_default();
    attrs.push(Attribute {
        oid: TIME_STAMP_TOKEN,
        values,
    });
    signer.unsigned_attrs = Some(SetOfVec::try_from(attrs).map_err(|_| rejected())?);
    signed.signer_infos = ::cms::signed_data::SignerInfos(
        SetOfVec::try_from(vec![signer]).map_err(|_| rejected())?,
    );
    encode_signed_data(&signed)
}

/// The byte range covers `file` except the hex digits of `/Contents`.
pub(crate) fn cms_byte_range_valid(file: &[u8], byte_range: &[usize], contents: &[u8]) -> bool {
    if byte_range.len() != 4 {
        return false;
    }
    let (start1, len1, start2, len2) = (byte_range[0], byte_range[1], byte_range[2], byte_range[3]);
    if start1 != 0 || len1 == 0 {
        return false;
    }
    let Some(end2) = start2.checked_add(len2) else {
        return false;
    };
    if end2 != file.len() {
        return false;
    }
    let Some(hex_len) = contents.len().checked_mul(2) else {
        return false;
    };
    if start2 != len1.saturating_add(hex_len) || len1.saturating_add(hex_len) < len1 {
        return false;
    }
    let Some(marker) = len1.checked_sub(1) else {
        return false;
    };
    if file.get(marker) != Some(&b'<') || file.get(start2) != Some(&b'>') {
        return false;
    }
    let Some(encoded_len) = der_len(contents) else {
        return false;
    };
    if contents[encoded_len..].iter().any(|byte| *byte != 0) {
        return false;
    }
    let Ok(info) = ::cms::content_info::ContentInfo::from_der(&contents[..encoded_len]) else {
        return false;
    };
    if info.content_type != const_oid::db::rfc5911::ID_SIGNED_DATA {
        return false;
    }
    let Ok(signed) = info.content.decode_as::<::cms::signed_data::SignedData>() else {
        return false;
    };
    if signed.encap_content_info.econtent.is_some()
        || signed.encap_content_info.econtent_type != const_oid::db::rfc5911::ID_DATA
    {
        return false;
    }
    let mut hasher = sha2::Sha256::new();
    hasher.update(&file[..len1]);
    hasher.update(&file[start2..]);
    message_digest_matches(
        &signed,
        hasher.finalize().as_slice(),
        const_oid::db::rfc5911::ID_DATA,
        0,
    )
}

/// One signer, SHA-256 message-digest, and a signature that verifies.
pub(crate) fn message_digest_matches(
    signed: &::cms::signed_data::SignedData,
    expected: &[u8],
    content_type: ObjectIdentifier,
    depth: u8,
) -> bool {
    if depth > 2 || signed.encap_content_info.econtent_type != content_type {
        return false;
    }
    let infos = signed.signer_infos.0.as_slice();
    if infos.len() != 1 {
        return false;
    }
    let signer = &infos[0];
    if !sha256_id(&signer.digest_alg) {
        return false;
    }
    let Some(signed_attrs) = signer.signed_attrs.as_ref() else {
        return false;
    };
    if !content_type_matches(signed_attrs, content_type) || !digest_matches(signed_attrs, expected) {
        return false;
    }
    let Some(cert) = signer_cert(signed, signer) else {
        return false;
    };
    let Ok(signed_der) = signed_attrs.to_der() else {
        return false;
    };
    if !signature_ok(cert, &signed_der, signer.signature.as_bytes(), &signer.signature_algorithm) {
        return false;
    }
    ess_ok(signed_attrs, cert) && timestamps_ok(signer, depth)
}

fn one_signer(cms_der: &[u8]) -> Option<(::cms::signed_data::SignedData, ::cms::signed_data::SignerInfo)> {
    let info = ::cms::content_info::ContentInfo::from_der(cms_der).ok()?;
    if info.content_type != const_oid::db::rfc5911::ID_SIGNED_DATA {
        return None;
    }
    let signed = info.content.decode_as::<::cms::signed_data::SignedData>().ok()?;
    let signer = {
        let infos = signed.signer_infos.0.as_slice();
        if infos.len() != 1 {
            return None;
        }
        infos[0].clone()
    };
    Some((signed, signer))
}

fn encode_signed_data(signed: &::cms::signed_data::SignedData) -> PdfResult<Vec<u8>> {
    let der = signed.to_der().map_err(|_| rejected())?;
    let content = AnyRef::try_from(der.as_slice()).map_err(|_| rejected())?;
    ::cms::content_info::ContentInfo {
        content_type: const_oid::db::rfc5911::ID_SIGNED_DATA,
        content: Any::from(content),
    }
    .to_der()
    .map_err(|_| rejected())
}

fn ess_attribute(cert: &Certificate) -> PdfResult<Attribute> {
    let der = cert.to_der().map_err(|_| failed())?;
    let hash = sha2::Sha256::digest(&der);
    let ess = SigningCertificateV2 {
        certs: vec![EssCertIdV2 {
            hash_algorithm: None,
            cert_hash: OctetString::new(hash.as_slice()).map_err(|_| failed())?,
            issuer_serial: None,
        }],
        policies: None,
    };
    let encoded = ess.to_der().map_err(|_| failed())?;
    let mut values = SetOfVec::new();
    values
        .insert(Any::from_der(&encoded).map_err(|_| failed())?)
        .map_err(|_| failed())?;
    Ok(Attribute {
        oid: ESS_CERT_V2,
        values,
    })
}

fn signer_cert<'a>(
    signed: &'a ::cms::signed_data::SignedData,
    signer: &::cms::signed_data::SignerInfo,
) -> Option<&'a Certificate> {
    let ::cms::signed_data::SignerIdentifier::IssuerAndSerialNumber(id) = &signer.sid else {
        return None;
    };
    let choices = signed.certificates.as_ref()?.0.as_slice();
    for choice in choices {
        let ::cms::cert::CertificateChoices::Certificate(cert) = choice else {
            continue;
        };
        if cert.tbs_certificate.issuer == id.issuer
            && cert.tbs_certificate.serial_number == id.serial_number
        {
            return Some(cert);
        }
    }
    None
}

fn signature_ok(
    cert: &Certificate,
    signed_der: &[u8],
    signature: &[u8],
    algorithm: &AlgorithmIdentifierOwned,
) -> bool {
    if !null_or_absent(&algorithm.parameters) {
        return false;
    }
    if algorithm.oid == SHA256_WITH_RSA {
        let Some(public) = rsa_public(cert) else {
            return false;
        };
        if public.n().bits() < 2048 {
            return false;
        }
        let Ok(signature) = rsa::pkcs1v15::Signature::try_from(signature) else {
            return false;
        };
        return rsa::pkcs1v15::VerifyingKey::<sha2::Sha256>::new(public)
            .verify(signed_der, &signature)
            .is_ok();
    }
    if algorithm.oid == ECDSA_WITH_SHA256 {
        let Some(public) = p256_public(cert) else {
            return false;
        };
        let Ok(signature) = p256::ecdsa::DerSignature::from_bytes(signature) else {
            return false;
        };
        return public.verify(signed_der, &signature).is_ok();
    }
    false
}

fn content_type_matches(attrs: &SetOfVec<Attribute>, expected: ObjectIdentifier) -> bool {
    let Some(value) = one_value(attrs, const_oid::db::rfc5911::ID_CONTENT_TYPE) else {
        return false;
    };
    let Ok(encoded) = value.to_der() else {
        return false;
    };
    ObjectIdentifier::from_der(&encoded).ok() == Some(expected)
}

fn digest_matches(attrs: &SetOfVec<Attribute>, expected: &[u8]) -> bool {
    let Some(value) = one_value(attrs, const_oid::db::rfc5911::ID_MESSAGE_DIGEST) else {
        return false;
    };
    let Ok(encoded) = value.to_der() else {
        return false;
    };
    OctetString::from_der(&encoded)
        .ok()
        .is_some_and(|octet| octet.as_bytes() == expected)
}

fn ess_ok(attrs: &SetOfVec<Attribute>, leaf: &Certificate) -> bool {
    let Some(value) = one_value(attrs, ESS_CERT_V2) else {
        return !attrs.as_slice().iter().any(|attr| attr.oid == ESS_CERT_V2);
    };
    let Ok(encoded) = value.to_der() else {
        return false;
    };
    let Ok(parsed) = SigningCertificateV2::from_der(&encoded) else {
        return false;
    };
    let Some(first) = parsed.certs.first() else {
        return false;
    };
    if let Some(algorithm) = &first.hash_algorithm {
        if !sha256_id(algorithm) {
            return false;
        }
    }
    let Ok(leaf_der) = leaf.to_der() else {
        return false;
    };
    let hash = sha2::Sha256::digest(&leaf_der);
    first.cert_hash.as_bytes() == hash.as_slice()
}

fn timestamps_ok(signer: &::cms::signed_data::SignerInfo, depth: u8) -> bool {
    let Some(unsigned) = &signer.unsigned_attrs else {
        return true;
    };
    for attr in unsigned.as_slice() {
        if attr.oid != TIME_STAMP_TOKEN {
            continue;
        }
        if depth >= 2 || attr.values.as_slice().len() != 1 {
            return false;
        }
        let Ok(der) = attr.values.as_slice()[0].to_der() else {
            return false;
        };
        if !timestamp::verify_token(&der, signer.signature.as_bytes(), depth + 1) {
            return false;
        }
    }
    true
}

fn has_timestamp(signer: &::cms::signed_data::SignerInfo) -> bool {
    signer.unsigned_attrs.as_ref().is_some_and(|attrs| {
        attrs.as_slice().iter().any(|attr| attr.oid == TIME_STAMP_TOKEN)
    })
}

/// Exactly one value. `None` when the attribute is absent or malformed.
fn one_value<'a>(attrs: &'a SetOfVec<Attribute>, oid: ObjectIdentifier) -> Option<&'a Any> {
    let mut found = None;
    for attr in attrs.as_slice() {
        if attr.oid != oid {
            continue;
        }
        if found.is_some() || attr.values.as_slice().len() != 1 {
            return None;
        }
        found = Some(&attr.values.as_slice()[0]);
    }
    found
}

fn sha256_algorithm() -> AlgorithmIdentifierOwned {
    AlgorithmIdentifierOwned {
        oid: const_oid::db::rfc5912::ID_SHA_256,
        parameters: None,
    }
}

fn sha256_id(algorithm: &AlgorithmIdentifierOwned) -> bool {
    algorithm.oid == const_oid::db::rfc5912::ID_SHA_256 && null_or_absent(&algorithm.parameters)
}

fn null_or_absent(parameters: &Option<Any>) -> bool {
    match parameters {
        None => true,
        Some(any) => any.tag() == der::Tag::Null,
    }
}

fn rsa_public(cert: &Certificate) -> Option<rsa::RsaPublicKey> {
    let der = cert.tbs_certificate.subject_public_key_info.to_der().ok()?;
    rsa::RsaPublicKey::from_public_key_der(&der).ok()
}

fn p256_public(cert: &Certificate) -> Option<p256::ecdsa::VerifyingKey> {
    let der = cert.tbs_certificate.subject_public_key_info.to_der().ok()?;
    p256::ecdsa::VerifyingKey::from_public_key_der(&der).ok()
}

fn failed() -> crate::error::PdfError {
    crypto("Signature could not be verified.")
}
