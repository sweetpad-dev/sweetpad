//! The CMS signature over a code directory, laid out as `codesign` writes
//! it: detached, with the certificates above the signing one first, Apple's
//! CDHashes attributes beside the standard signed ones, and a secure
//! timestamp's token, when it has one, as an unsigned one.

use std::time::SystemTime;

use super::blobs::CodeHash;
use super::crypto::{self, Hash, sha256};
use super::der::{self, oids};
use super::identity::Identity;
use super::plist::{self, Plist};
use super::time::utc_time;

/// The CMS SignedData, in DER, signing `directories` as `identity` at
/// `time`, stamped by the timestamp server at `timestamp`, if any. The
/// message digest is the first directory's, the one in the code directory
/// slot, and the CDHashes attributes list every directory's hash.
pub(super) fn sign(
    identity: &Identity,
    directories: &[(CodeHash, Vec<u8>)],
    time: SystemTime,
    timestamp: Option<&str>,
) -> Result<Vec<u8>, String> {
    let key = identity
        .key
        .as_ref()
        .ok_or("the identity has no private key")?;
    identity.require_chain()?;
    let (_, primary) = directories.first().ok_or("no code directory to sign")?;
    let digests: Vec<(CodeHash, Vec<u8>)> = directories
        .iter()
        .map(|(hash, directory)| (*hash, hash.digest(directory)))
        .collect();
    let cdhashes = plist::to_xml(&Plist::Dict(vec![(
        "cdhashes".to_string(),
        Plist::Array(
            digests
                .iter()
                .map(|(_, digest)| Plist::Data(digest[..20].to_vec()))
                .collect(),
        ),
    )]));
    let attribute_of =
        |oid: &[u8], values: Vec<Vec<u8>>| der::sequence(&[&der::oid(oid), &der::set_of(values)]);
    let attribute = |oid: &[u8], value: Vec<u8>| attribute_of(oid, vec![value]);
    let attributes = der::set_of(vec![
        attribute(oids::CONTENT_TYPE, der::oid(oids::DATA)),
        attribute(
            oids::SIGNING_TIME,
            der::tlv(der::UTC_TIME, utc_time(time).as_bytes()),
        ),
        attribute(
            oids::MESSAGE_DIGEST,
            der::tlv(der::OCTET_STRING, &sha256(primary)),
        ),
        attribute_of(
            oids::APPLE_CDHASHES2,
            digests
                .iter()
                .map(|(hash, digest)| {
                    let algorithm = match hash {
                        CodeHash::Sha1 => oids::SHA1,
                        CodeHash::Sha256 => oids::SHA256,
                    };
                    der::sequence(&[&der::oid(algorithm), &der::tlv(der::OCTET_STRING, digest)])
                })
                .collect(),
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
