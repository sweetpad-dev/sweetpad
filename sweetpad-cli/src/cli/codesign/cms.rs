//! The CMS signature over a code directory, laid out as `codesign` writes
//! it: detached, with the certificates above the signing one first, and
//! Apple's CDHashes attributes beside the standard signed ones.

use std::time::SystemTime;

use super::crypto::{self, sha256};
use super::der::{self, oids};
use super::identity::Identity;
use super::plist::{self, Plist};

/// The CMS SignedData, in DER, signing `code_directory` as `identity` at
/// `time`.
pub(super) fn sign(
    identity: &Identity,
    code_directory: &[u8],
    time: SystemTime,
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
    let signer = der::sequence(&[
        &der::integer(1),
        &der::sequence(&[&certificate.issuer, &certificate.serial]),
        &der::algorithm(oids::SHA256),
        // The signed attributes, under their implicit tag.
        &der::tlv(
            der::context(0),
            &attributes[der_header_length(&attributes)..],
        ),
        &der::algorithm(oids::SHA256_WITH_RSA),
        &der::tlv(der::OCTET_STRING, &signature),
    ]);
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
fn der_header_length(element: &[u8]) -> usize {
    match element.get(1) {
        Some(n) if *n >= 0x80 => 2 + usize::from(n & 0x7f),
        _ => 2,
    }
}

/// `time` as an ASN.1 UTCTime: `YYMMDDhhmmssZ`.
fn utc_time(time: SystemTime) -> String {
    let seconds = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    // Howard Hinnant's days-to-civil conversion.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{:02}{month:02}{day:02}{:02}{:02}{:02}Z",
        year % 100,
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn utc_time_from_unix_time() {
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_793_735_619);
        assert_eq!(utc_time(time), "261103195339Z");
        assert_eq!(utc_time(SystemTime::UNIX_EPOCH), "700101000000Z");
    }
}
