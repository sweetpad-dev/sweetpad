//! The CMS signature over a code directory, laid out as `codesign` writes
//! it: detached, with the certificates above the signing one first, Apple's
//! CDHashes attributes beside the standard signed ones, and a secure
//! timestamp's token, when it has one, as an unsigned one.

use std::time::SystemTime;

use super::crypto::{self, Hash, sha256};
use super::der::{self, oids};
use super::identity::Identity;
use super::plist::{self, Plist};
use super::time::utc_time;

/// The CMS SignedData, in DER, signing `code_directory` as `identity` at
/// `time`, stamped by the timestamp server at `timestamp`, if any.
pub(super) fn sign(
    identity: &Identity,
    code_directory: &[u8],
    time: SystemTime,
    timestamp: Option<&str>,
) -> Result<Vec<u8>, String> {
    let key = identity
        .key
        .as_ref()
        .ok_or("the identity has no private key")?;
    let cdhash = sha256(code_directory);
    let cdhashes = plist::to_xml(&Plist::Dict(vec![(
        "cdhashes".to_string(),
        Plist::Array(vec![Plist::Data(cdhash[..20].to_vec())]),
    )]));
    let attribute =
        |oid: &[u8], value: Vec<u8>| der::sequence(&[&der::oid(oid), &der::set_of(vec![value])]);
    let attributes = der::set_of(vec![
        attribute(oids::CONTENT_TYPE, der::oid(oids::DATA)),
        attribute(
            oids::SIGNING_TIME,
            der::tlv(der::UTC_TIME, utc_time(time).as_bytes()),
        ),
        attribute(oids::MESSAGE_DIGEST, der::tlv(der::OCTET_STRING, &cdhash)),
        attribute(
            oids::APPLE_CDHASHES2,
            der::sequence(&[
                &der::oid(oids::SHA256),
                &der::tlv(der::OCTET_STRING, &cdhash),
            ]),
        ),
        attribute(
            oids::APPLE_CDHASHES,
            der::tlv(der::OCTET_STRING, cdhashes.as_bytes()),
        ),
    ]);
    let signature = crypto::sign_sha256(key, &sha256(&attributes))?;
    let certificate = &identity.certificate;
    let mut signer = vec![
        der::integer(1),
        der::sequence(&[&certificate.issuer, &certificate.serial]),
        der::algorithm(oids::SHA256),
        // The signed attributes, under their implicit tag.
        der::tlv(
            der::context(0),
            &attributes[der_header_length(&attributes)..],
        ),
        der::algorithm(oids::SHA256_WITH_RSA),
        der::tlv(der::OCTET_STRING, &signature),
    ];
    if let Some(url) = timestamp {
        let token = super::timestamp::token(url, Hash::Sha256, &sha256(&signature))?;
        let unsigned = der::set_of(vec![attribute(oids::TIMESTAMP_TOKEN, token)]);
        signer.push(der::tlv(
            der::context(1),
            &unsigned[der_header_length(&unsigned)..],
        ));
    }
    let signer = der::sequence(&signer.iter().map(Vec::as_slice).collect::<Vec<_>>());
    let certificates: Vec<&[u8]> = identity
        .chain
        .iter()
        .map(|c| c.der.as_slice())
        .chain([certificate.der.as_slice()])
        .collect();
    let signed_data = der::sequence(&[
        &der::integer(1),
        &der::constructed(der::SET, &[&der::algorithm(oids::SHA256)]),
        &der::sequence(&[&der::oid(oids::DATA)]),
        &der::constructed(der::context(0), &certificates),
        &der::constructed(der::SET, &[&signer]),
    ]);
    Ok(der::sequence(&[
        &der::oid(oids::SIGNED_DATA),
        &der::constructed(der::context(0), &[&signed_data]),
    ]))
}

/// How many bytes an element's tag and length take.
pub(super) fn der_header_length(element: &[u8]) -> usize {
    match element.get(1) {
        Some(n) if *n >= 0x80 => 2 + usize::from(n & 0x7f),
        _ => 2,
    }
}
