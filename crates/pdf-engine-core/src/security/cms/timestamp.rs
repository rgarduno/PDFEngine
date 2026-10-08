//! RFC 3161 time-stamp request and token checks.
//!
//! The request covers the CMS signature value. A token is accepted when its
//! signed data verifies against the certificate inside the token and the
//! imprint equals that signature value. This does not contact a time-stamping
//! authority and does not decide trust.

#[cfg(test)]
use std::time::SystemTime;

use der::asn1::{GeneralizedTime, ObjectIdentifier, OctetString};
use der::{Any, Decode, Encode, Reader, Sequence, SliceReader, Tagged};
use sha2::Digest;
use spki::AlgorithmIdentifierOwned;
#[cfg(test)]
use x509_cert::Certificate;

use super::rejected;
use super::signed;
use crate::error::PdfResult;

const TST_INFO: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");
#[cfg(test)]
const POLICY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.3.4.1");

#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct MessageImprint {
    hash_algorithm: AlgorithmIdentifierOwned,
    hashed_message: OctetString,
}

#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct TimeStampReq {
    version: u8,
    message_imprint: MessageImprint,
    cert_req: bool,
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
struct TstInfo {
    version: u8,
    policy: ObjectIdentifier,
    message_imprint: MessageImprint,
    serial_number: u8,
    gen_time: GeneralizedTime,
}

/// DER `TimeStampReq` for `signature_value`.
pub(crate) fn request_for(signature_value: &[u8]) -> PdfResult<Vec<u8>> {
    let digest = sha2::Sha256::digest(signature_value);
    let request = TimeStampReq {
        version: 1,
        message_imprint: MessageImprint {
            hash_algorithm: sha256_alg(),
            hashed_message: OctetString::new(digest.as_slice()).map_err(|_| rejected())?,
        },
        cert_req: true,
    };
    request.to_der().map_err(|_| rejected())
}

/// Builds a signed timestamp token over `signature_value` for tests.
#[cfg(test)]
pub(crate) fn mint_token(
    signer: &p256::ecdsa::SigningKey,
    cert: &Certificate,
    signature_value: &[u8],
) -> PdfResult<Vec<u8>> {
    let imprint_digest = sha2::Sha256::digest(signature_value);
    let info = TstInfo {
        version: 1,
        policy: POLICY,
        message_imprint: MessageImprint {
            hash_algorithm: sha256_alg(),
            hashed_message: OctetString::new(imprint_digest.as_slice()).map_err(|_| rejected())?,
        },
        serial_number: 1,
        gen_time: GeneralizedTime::from_system_time(SystemTime::now()).map_err(|_| rejected())?,
    };
    let tst_der = info.to_der().map_err(|_| rejected())?;
    let octet = OctetString::new(tst_der)
        .map_err(|_| rejected())?
        .to_der()
        .map_err(|_| rejected())?;
    let eci = ::cms::signed_data::EncapsulatedContentInfo {
        econtent_type: TST_INFO,
        econtent: Some(Any::from_der(&octet).map_err(|_| rejected())?),
    };
    signed::sign_eci::<_, p256::ecdsa::DerSignature>(signer, cert, &[], &eci, None)
}

/// The token's imprint is SHA-256 of `signature_value` and its signature verifies.
pub(crate) fn covers(token: &[u8], signature_value: &[u8]) -> bool {
    verify_token(token, signature_value, 1)
}

pub(crate) fn verify_token(token: &[u8], signature_value: &[u8], depth: u8) -> bool {
    if token.is_empty() || token.len() > 64 * 1024 || depth > 2 {
        return false;
    }
    let Some(token_der) = content_info_der(token) else {
        return false;
    };
    let Ok(info) = ::cms::content_info::ContentInfo::from_der(&token_der) else {
        return false;
    };
    if info.content_type != const_oid::db::rfc5911::ID_SIGNED_DATA {
        return false;
    }
    let Ok(signed) = info.content.decode_as::<::cms::signed_data::SignedData>() else {
        return false;
    };
    let Some(content) = signed.encap_content_info.econtent.as_ref() else {
        return false;
    };
    if signed.encap_content_info.econtent_type != TST_INFO {
        return false;
    }
    let Some(tst_bytes) = tst_bytes(content) else {
        return false;
    };
    let tst_hash = sha2::Sha256::digest(&tst_bytes);
    if !signed::message_digest_matches(&signed, tst_hash.as_slice(), TST_INFO, depth) {
        return false;
    }
    let Some(imprint) = message_imprint(&tst_bytes) else {
        return false;
    };
    if !sha256_id(&imprint.hash_algorithm) {
        return false;
    }
    let expected = sha2::Sha256::digest(signature_value);
    imprint.hashed_message.as_bytes() == expected.as_slice()
}

pub(crate) fn content_info_der(token: &[u8]) -> Option<Vec<u8>> {
    if let Ok(info) = ::cms::content_info::ContentInfo::from_der(token) {
        if info.content_type == const_oid::db::rfc5911::ID_SIGNED_DATA {
            return Some(token.to_vec());
        }
    }
    let mut reader = SliceReader::new(token).ok()?;
    let info = reader
        .sequence(|reader| {
            reader.sequence(|status| {
                let code: u8 = status.decode()?;
                while !status.is_finished() {
                    let _: Any = status.decode()?;
                }
                if code > 1 {
                    return Err(der::Error::new(der::ErrorKind::Failed, der::Length::ZERO));
                }
                Ok(())
            })?;
            if reader.is_finished() {
                return Err(der::Error::new(der::ErrorKind::Failed, der::Length::ZERO));
            }
            let info: ::cms::content_info::ContentInfo = reader.decode()?;
            if !reader.is_finished() {
                return Err(der::Error::new(der::ErrorKind::Failed, der::Length::ZERO));
            }
            Ok(info)
        })
        .ok()?;
    if !reader.is_finished() {
        return None;
    }
    if info.content_type != const_oid::db::rfc5911::ID_SIGNED_DATA {
        return None;
    }
    info.to_der().ok()
}

fn tst_bytes(content: &Any) -> Option<Vec<u8>> {
    let encoded = content.to_der().ok()?;
    if let Ok(octet) = OctetString::from_der(&encoded) {
        return Some(octet.as_bytes().to_vec());
    }
    Some(content.value().to_vec())
}

fn message_imprint(tst_bytes: &[u8]) -> Option<MessageImprint> {
    let mut reader = SliceReader::new(tst_bytes).ok()?;
    let imprint = reader
        .sequence(|reader| {
            let _version: u8 = reader.decode()?;
            let _policy: ObjectIdentifier = reader.decode()?;
            let imprint: MessageImprint = reader.decode()?;
            let _: Any = reader.decode()?;
            let _: GeneralizedTime = reader.decode()?;
            while !reader.is_finished() {
                let _: Any = reader.decode()?;
            }
            Ok(imprint)
        })
        .ok()?;
    if reader.is_finished() {
        Some(imprint)
    } else {
        None
    }
}

fn sha256_alg() -> AlgorithmIdentifierOwned {
    AlgorithmIdentifierOwned {
        oid: const_oid::db::rfc5912::ID_SHA_256,
        parameters: None,
    }
}

fn sha256_id(algorithm: &AlgorithmIdentifierOwned) -> bool {
    if algorithm.oid != const_oid::db::rfc5912::ID_SHA_256 {
        return false;
    }
    match &algorithm.parameters {
        None => true,
        Some(any) => any.tag() == der::Tag::Null,
    }
}
