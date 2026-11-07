//! The blobs of an embedded code signature, as `codesign` writes them: the
//! code directory, the requirements, the entitlements in XML and in DER,
//! and the superblob that indexes them.

use super::crypto::sha256;
use super::der;
use super::identity::Identity;
use super::plist::Plist;

pub(super) const REQUIREMENT: u32 = 0xfade_0c00;
pub(super) const REQUIREMENTS: u32 = 0xfade_0c01;
pub(super) const CODE_DIRECTORY: u32 = 0xfade_0c02;
pub(super) const EMBEDDED_SIGNATURE: u32 = 0xfade_0cc0;
pub(super) const ENTITLEMENTS: u32 = 0xfade_7171;
pub(super) const DER_ENTITLEMENTS: u32 = 0xfade_7172;
pub(super) const CMS_WRAPPER: u32 = 0xfade_0b01;

/// The superblob slot of each blob.
pub(super) const SLOT_CODE_DIRECTORY: u32 = 0;
pub(super) const SLOT_REQUIREMENTS: u32 = 2;
pub(super) const SLOT_ENTITLEMENTS: u32 = 5;
pub(super) const SLOT_DER_ENTITLEMENTS: u32 = 7;
pub(super) const SLOT_CMS: u32 = 0x1_0000;

/// The code directory's special slots, by number: what each hashes.
pub(super) const SPECIAL_INFO: usize = 1;
pub(super) const SPECIAL_REQUIREMENTS: usize = 2;
pub(super) const SPECIAL_RESOURCES: usize = 3;
pub(super) const SPECIAL_ENTITLEMENTS: usize = 5;
pub(super) const SPECIAL_DER_ENTITLEMENTS: usize = 7;

/// Code directory flags.
pub(super) const FLAG_ADHOC: u32 = 0x2;
pub(super) const FLAG_LINKER_SIGNED: u32 = 0x2_0000;

/// Executable segment flags.
pub(super) const EXECSEG_MAIN_BINARY: u64 = 0x1;
pub(super) const EXECSEG_ALLOW_UNSIGNED: u64 = 0x10;

/// The page size the code directory hashes the code in: 16 KiB.
const PAGE_SIZE_LOG2: u8 = 14;
const VERSION: u32 = 0x2_0400;
const HEADER_SIZE: usize = 88;

/// A blob: its magic, its length and `payload`.
pub(super) fn blob(magic: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.extend_from_slice(&magic.to_be_bytes());
    out.extend_from_slice(&u32::try_from(payload.len() + 8).unwrap_or(0).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// What a code directory records besides the hashes of the code.
pub(super) struct Directory<'a> {
    pub identifier: &'a str,
    pub team: Option<&'a str>,
    pub flags: u32,
    /// The executable segment's file offset, size and flags.
    pub exec_segment: (u64, u64, u64),
    /// The special slots' hashes, by slot number; a slot left out of the
    /// middle hashes as zeros.
    pub special: Vec<(usize, [u8; 32])>,
}

impl Directory<'_> {
    /// Detached resource signatures hash the complete Info.plist once. They
    /// have neither Mach-O executable segments nor a code-page size.
    pub fn build_unpaged(&self, code: &[u8]) -> Vec<u8> {
        let header_size = if self.team.is_some() { 52 } else { 48 };
        let version = if self.team.is_some() {
            0x2_0200
        } else {
            0x2_0100
        };
        let slots = self.special.iter().map(|(n, _)| *n).max().unwrap_or(0);
        let team_offset = header_size + self.identifier.len() + 1;
        let hash_offset = team_offset + self.team.map_or(0, |t| t.len() + 1) + 32 * slots;
        let mut out = Vec::new();
        for value in [
            CODE_DIRECTORY as usize,
            hash_offset + 32,
            version,
            self.flags as usize,
            hash_offset,
            header_size,
            slots,
            1,
            code.len(),
        ] {
            out.extend_from_slice(&u32::try_from(value).unwrap_or(0).to_be_bytes());
        }
        out.extend_from_slice(&[32, 2, 0, 0]);
        out.extend_from_slice(&[0; 8]); // spare and scatter offset
        if self.team.is_some() {
            out.extend_from_slice(&u32::try_from(team_offset).unwrap_or(0).to_be_bytes());
        }
        out.extend_from_slice(self.identifier.as_bytes());
        out.push(0);
        if let Some(team) = self.team {
            out.extend_from_slice(team.as_bytes());
            out.push(0);
        }
        for slot in (1..=slots).rev() {
            let hash = self
                .special
                .iter()
                .find(|(n, _)| *n == slot)
                .map_or([0; 32], |(_, hash)| *hash);
            out.extend_from_slice(&hash);
        }
        out.extend_from_slice(&sha256(code));
        out
    }

    /// The code directory over `code`, the slice up to its signature.
    pub fn build(&self, code: &[u8]) -> Vec<u8> {
        let slots = self.special.iter().map(|(n, _)| *n).max().unwrap_or(0);
        let pages = code.len().div_ceil(1 << PAGE_SIZE_LOG2);
        let team = self.team.map_or(0, |t| t.len() + 1);
        let identity_offset = HEADER_SIZE;
        let team_offset = identity_offset + self.identifier.len() + 1;
        let hash_offset = team_offset + team + 32 * slots;
        let length = hash_offset + 32 * pages;
        let mut out = Vec::with_capacity(length);
        let mut u32s = |values: &[usize]| {
            for value in values {
                out.extend_from_slice(&u32::try_from(*value).unwrap_or(0).to_be_bytes());
            }
        };
        u32s(&[
            CODE_DIRECTORY as usize,
            length,
            VERSION as usize,
            self.flags as usize,
            hash_offset,
            identity_offset,
            slots,
            pages,
            code.len(),
        ]);
        // Hash size, hash type (SHA-256), platform and page size.
        out.extend_from_slice(&[32, 2, 0, PAGE_SIZE_LOG2]);
        // The spare, the scatter offset, the team offset, another spare and
        // the 64-bit code limit, unused below 4 GiB.
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        let team_field = if self.team.is_some() { team_offset } else { 0 };
        out.extend_from_slice(&u32::try_from(team_field).unwrap_or(0).to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&0u64.to_be_bytes());
        let (base, limit, flags) = self.exec_segment;
        for value in [base, limit, flags] {
            out.extend_from_slice(&value.to_be_bytes());
        }
        out.extend_from_slice(self.identifier.as_bytes());
        out.push(0);
        if let Some(team) = self.team {
            out.extend_from_slice(team.as_bytes());
            out.push(0);
        }
        for slot in (1..=slots).rev() {
            let hash = self
                .special
                .iter()
                .find(|(n, _)| *n == slot)
                .map_or([0; 32], |(_, h)| *h);
            out.extend_from_slice(&hash);
        }
        for page in code.chunks(1 << PAGE_SIZE_LOG2) {
            out.extend_from_slice(&sha256(page));
        }
        out
    }
}

/// The fields of a code directory a re-sign keeps.
pub(super) struct Existing {
    pub identifier: String,
    pub flags: u32,
    pub entitlements: Option<Vec<u8>>,
}

/// The identifier, flags and entitlements of the signature `superblob`.
pub(super) fn read_existing(superblob: &[u8]) -> Option<Existing> {
    let be = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(
            superblob.get(at..at + 4)?.try_into().ok()?,
        ))
    };
    if be(0)? != EMBEDDED_SIGNATURE {
        return None;
    }
    let mut existing = Existing {
        identifier: String::new(),
        flags: 0,
        entitlements: None,
    };
    for i in 0..be(8)? as usize {
        let slot = be(12 + 8 * i)?;
        let at = be(16 + 8 * i)? as usize;
        let length = be(at + 4)? as usize;
        let body = superblob.get(at..at + length)?;
        if slot == SLOT_CODE_DIRECTORY {
            existing.flags = be(at + 12)?;
            let ident = u32::from_be_bytes(body.get(20..24)?.try_into().ok()?) as usize;
            let end = body.get(ident..)?.iter().position(|b| *b == 0)?;
            existing.identifier = String::from_utf8_lossy(&body[ident..ident + end]).into_owned();
        } else if slot == SLOT_ENTITLEMENTS {
            existing.entitlements = Some(body.get(8..)?.to_vec());
        }
    }
    Some(existing)
}

/// The requirements blob: the designated requirement `codesign` writes
/// for `identity`, or none for an ad-hoc signature.
pub(super) fn requirements(
    identifier: &str,
    identity: Option<&Identity>,
) -> Result<Vec<u8>, String> {
    let Some(identity) = identity else {
        return Ok(blob(REQUIREMENTS, &0u32.to_be_bytes()));
    };
    let mut expression = Vec::new();
    op(&mut expression, OP_AND);
    op(&mut expression, OP_IDENT);
    data(&mut expression, identifier.as_bytes());
    let intermediate = identity.chain.first();
    if identity.is_apple_issued()
        && intermediate.is_some_and(|c| c.has_extension(der::oids::APPLE_WWDR_INTERMEDIATE))
    {
        // anchor apple generic and certificate leaf[subject.CN] = "…" and
        // certificate 1[field.1.2.840.113635.100.6.2.1] exists
        op(&mut expression, OP_AND);
        op(&mut expression, OP_APPLE_GENERIC_ANCHOR);
        op(&mut expression, OP_AND);
        op(&mut expression, OP_CERT_FIELD);
        op(&mut expression, 0);
        data(&mut expression, b"subject.CN");
        op(&mut expression, MATCH_EQUAL);
        data(&mut expression, identity.certificate.common_name.as_bytes());
        op(&mut expression, OP_CERT_GENERIC);
        op(&mut expression, 1);
        data(&mut expression, der::oids::APPLE_WWDR_INTERMEDIATE);
        op(&mut expression, MATCH_EXISTS);
    } else if identity.is_apple_issued() {
        return Err(format!(
            "signing with '{}' isn't supported: only Apple Development and Apple \
             Distribution identities are",
            identity.certificate.common_name
        ));
    } else {
        // certificate root = H"…"
        op(&mut expression, OP_ANCHOR_HASH);
        op(&mut expression, u32::MAX);
        data(&mut expression, &super::crypto::sha1(&identity.root().der));
    }
    let mut requirement = 1u32.to_be_bytes().to_vec();
    requirement.extend(expression);
    let requirement = blob(REQUIREMENT, &requirement);
    let mut set = Vec::new();
    // One requirement, the designated one, past the count and its index.
    set.extend_from_slice(&1u32.to_be_bytes());
    set.extend_from_slice(&3u32.to_be_bytes());
    set.extend_from_slice(&20u32.to_be_bytes());
    set.extend(requirement);
    Ok(blob(REQUIREMENTS, &set))
}

const OP_IDENT: u32 = 2;
const OP_ANCHOR_HASH: u32 = 4;
const OP_AND: u32 = 6;
const OP_CERT_FIELD: u32 = 11;
const OP_CERT_GENERIC: u32 = 14;
const OP_APPLE_GENERIC_ANCHOR: u32 = 15;
const MATCH_EXISTS: u32 = 0;
const MATCH_EQUAL: u32 = 1;

fn op(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// Bytes in a requirement: their length, then them, padded to four bytes.
fn data(out: &mut Vec<u8>, bytes: &[u8]) {
    op(out, u32::try_from(bytes.len()).unwrap_or(0));
    out.extend_from_slice(bytes);
    out.resize(out.len().next_multiple_of(4), 0);
}

/// The entitlements in DER, as `--generate-entitlement-der` encodes them.
pub(super) fn der_entitlements(entitlements: &Plist) -> Result<Vec<u8>, String> {
    Ok(blob(
        DER_ENTITLEMENTS,
        &der_entitlements_payload(entitlements)?,
    ))
}

/// The DER entitlements outside their blob, as `derq` writes them.
pub(super) fn der_entitlements_payload(entitlements: &Plist) -> Result<Vec<u8>, String> {
    Ok(der::constructed(
        0x70,
        &[&der::integer(1), &der_value(entitlements)?],
    ))
}

fn der_value(value: &Plist) -> Result<Vec<u8>, String> {
    Ok(match value {
        Plist::Dict(entries) => {
            let mut sorted: Vec<&(String, Plist)> = entries.iter().collect();
            sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            let mut content = Vec::new();
            for (key, value) in sorted {
                content.extend(der::sequence(&[
                    &der::tlv(der::UTF8_STRING, key.as_bytes()),
                    &der_value(value)?,
                ]));
            }
            der::tlv(0xb0, &content)
        }
        Plist::Array(items) => {
            let parts = items.iter().map(der_value).collect::<Result<Vec<_>, _>>()?;
            der::tlv(der::SEQUENCE, &parts.concat())
        }
        Plist::String(text) => der::tlv(der::UTF8_STRING, text.as_bytes()),
        Plist::Bool(value) => der::tlv(der::BOOLEAN, &[if *value { 0xff } else { 0 }]),
        Plist::Int(n) => {
            let bytes = n.to_be_bytes();
            let mut start = 0;
            while start < 7
                && ((bytes[start] == 0 && bytes[start + 1] & 0x80 == 0)
                    || (bytes[start] == 0xff && bytes[start + 1] & 0x80 != 0))
            {
                start += 1;
            }
            der::tlv(der::INTEGER, &bytes[start..])
        }
        _ => return Err("entitlements hold a value DER entitlements can't encode".to_string()),
    })
}

/// The superblob of `blobs`, each with its slot, in slot order.
pub(super) fn superblob(mut blobs: Vec<(u32, Vec<u8>)>) -> Vec<u8> {
    blobs.sort_by_key(|(slot, _)| *slot);
    let mut index = Vec::new();
    let mut body = Vec::new();
    let header = 12 + 8 * blobs.len();
    for (slot, bytes) in &blobs {
        index.extend_from_slice(&slot.to_be_bytes());
        index.extend_from_slice(
            &u32::try_from(header + body.len())
                .unwrap_or(0)
                .to_be_bytes(),
        );
        body.extend_from_slice(bytes);
    }
    let mut payload = u32::try_from(blobs.len())
        .unwrap_or(0)
        .to_be_bytes()
        .to_vec();
    payload.extend(index);
    payload.extend(body);
    blob(EMBEDDED_SIGNATURE, &payload)
}
