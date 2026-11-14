//! Stapling a notarization ticket, as `stapler staple` does: the ticket
//! Apple files in CloudKit under the code directory hash of an app's main
//! executable, or under a package's table of contents checksum, written
//! into the app's `Contents/CodeResources`, or after the package's heap
//! between a header and a trailer that gives its length. Gatekeeper then
//! finds it offline.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde_json::{Value, json};

use super::bundle::Layout;
use super::crypto::hex;
use super::inspect;
use crate::cli::appstore::api::stub_base;
use crate::cli::net::{Request, Session};

const LOOKUP: &str = "https://api.apple-cloudkit.com/database/1/com.apple.gk.ticket-delivery/production/public/records/lookup";

/// A package's ticket sits between two records of this size: the magic,
/// then little-endian the version (1), the kind (1 before the ticket, 2
/// after it) and the ticket's length (0 before it).
const MAGIC: &[u8; 4] = b"t8lr";
const RECORD_SIZE: usize = 16;
const HEADER: u16 = 1;
const TRAILER: u16 = 2;

/// What [`staple`] did.
pub(crate) struct Stapled {
    pub path: PathBuf,
    /// The CloudKit record the ticket came from.
    pub record: String,
}

/// Writes a ticket into an app or package.
type WriteTicket = fn(&Path, &[u8]) -> Result<(), String>;

/// Staple its ticket to the app or package at `path`.
pub(crate) fn staple(path: &Path) -> Result<Stapled, String> {
    let session = Session::from_env();
    let (records, write): (Vec<String>, WriteTicket) = if path.is_dir() {
        (app_records(path)?, staple_app)
    } else if path.extension().is_some_and(|e| e == "pkg") {
        (vec![package_record(path)?], staple_package)
    } else {
        return Err(format!(
            "{} is neither an app nor a package, the two sweetpad staples a ticket to",
            path.display()
        ));
    };
    for record in &records {
        if let Some(ticket) = lookup(&session, record)? {
            write(path, &ticket)?;
            return Ok(Stapled {
                path: path.to_path_buf(),
                record: record.clone(),
            });
        }
    }
    Err(format!(
        "Apple has no notarization ticket for {}: notarize it first, or wait a minute after \
         Apple accepts it (looked up {})",
        path.display(),
        records.join(", ")
    ))
}

/// The records an app's ticket can be filed under: each slice's code
/// directory hash, the preferred slice's first.
fn app_records(app: &Path) -> Result<Vec<String>, String> {
    let layout = Layout::of(app)?;
    let executable = layout
        .executable
        .map(|e| layout.root.join(e))
        .ok_or_else(|| format!("{} names no executable", layout.info_path.display()))?;
    Ok(inspect::cdhashes(&executable)?
        .into_iter()
        .map(|(kind, hash)| format!("2/{kind}/{}", hex(&hash)))
        .collect())
}

/// The record a package's ticket is filed under: its table of contents'
/// SHA-1 checksum.
fn package_record(package: &Path) -> Result<String, String> {
    let data =
        std::fs::read(package).map_err(|e| format!("couldn't read {}: {e}", package.display()))?;
    let (heap, _) = xar_layout(&data, package)?;
    let checksum = data
        .get(heap..heap + 20)
        .ok_or_else(|| format!("{} is truncated", package.display()))?;
    Ok(format!("2/1/{}", hex(checksum)))
}

/// Where a package's heap starts and where its unstapled bytes end.
fn xar_layout(data: &[u8], package: &Path) -> Result<(usize, usize), String> {
    if data.get(..4) != Some(b"xar!") {
        return Err(format!("{} isn't a flat package", package.display()));
    }
    let header = usize::from(u16::from_be_bytes([data[4], data[5]]));
    let compressed = data
        .get(8..16)
        .and_then(|b| b.try_into().ok())
        .map(u64::from_be_bytes)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| format!("{} is truncated", package.display()))?;
    let end = match data.len().checked_sub(RECORD_SIZE) {
        Some(at) if data[at..at + 4] == *MAGIC => {
            let damaged = || format!("{}'s stapled ticket is damaged", package.display());
            let length = data[at + 8..at + 16]
                .try_into()
                .map(u64::from_le_bytes)
                .ok()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(damaged)?;
            let start = at
                .checked_sub(length)
                .and_then(|n| n.checked_sub(RECORD_SIZE))
                .ok_or_else(damaged)?;
            if data[start..start + 4] != *MAGIC {
                return Err(damaged());
            }
            start
        }
        _ => data.len(),
    };
    Ok((header + compressed, end))
}

/// The ticket filed under `record`, if Apple has one.
fn lookup(session: &Session, record: &str) -> Result<Option<Vec<u8>>, String> {
    let url = stub_base().map_or_else(
        || LOOKUP.to_string(),
        |base| format!("{base}/cloudkit/lookup"),
    );
    let request = Request::new(minreq::Method::Post, url)
        .header("Content-Type", "application/json")
        .body(
            json!({"records": [{"recordName": record}]})
                .to_string()
                .into_bytes(),
        )
        .timeout(60);
    let response = session
        .send_retrying(&request, 3)
        .map_err(|e| format!("couldn't look up the notarization ticket: {e}"))?;
    if !response.ok() {
        return Err(format!(
            "the ticket service answered {} {}: {}",
            response.status,
            response.reason,
            response.snippet()
        ));
    }
    let value: Value = serde_json::from_slice(&response.body)
        .map_err(|e| format!("the ticket service's answer isn't JSON: {e}"))?;
    let found = &value["records"][0];
    if found["serverErrorCode"].as_str() == Some("NOT_FOUND") {
        return Ok(None);
    }
    let encoded = found["fields"]["signedTicket"]["value"]
        .as_str()
        .ok_or_else(|| format!("the ticket service's answer holds no ticket: {found}"))?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map(Some)
        .map_err(|e| format!("the ticket isn't base64: {e}"))
}

/// An app keeps its ticket in `Contents/CodeResources`, beside the sealed
/// `_CodeSignature`.
fn staple_app(app: &Path, ticket: &[u8]) -> Result<(), String> {
    let layout = Layout::of(app)?;
    let path = layout.root.join("CodeResources");
    std::fs::write(&path, ticket).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

/// A package keeps its ticket after the heap, in place of any ticket
/// stapled before.
fn staple_package(package: &Path, ticket: &[u8]) -> Result<(), String> {
    let mut data =
        std::fs::read(package).map_err(|e| format!("couldn't read {}: {e}", package.display()))?;
    let (_, end) = xar_layout(&data, package)?;
    data.truncate(end);
    data.extend_from_slice(&record(HEADER, 0));
    data.extend_from_slice(ticket);
    data.extend_from_slice(&record(TRAILER, ticket.len()));
    std::fs::write(package, data).map_err(|e| format!("couldn't write {}: {e}", package.display()))
}

fn record(kind: u16, length: usize) -> [u8; RECORD_SIZE] {
    let mut out = [0; RECORD_SIZE];
    out[..4].copy_from_slice(MAGIC);
    out[4..6].copy_from_slice(&1u16.to_le_bytes());
    out[6..8].copy_from_slice(&kind.to_le_bytes());
    out[8..].copy_from_slice(&(length as u64).to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    /// A flat package's first bytes: the header, a table of contents and
    /// the heap's checksum, then a file.
    fn package() -> Vec<u8> {
        let toc = b"compressed table of contents";
        let mut data = b"xar!".to_vec();
        data.extend_from_slice(&28u16.to_be_bytes());
        data.extend_from_slice(&1u16.to_be_bytes());
        data.extend_from_slice(&(toc.len() as u64).to_be_bytes());
        data.extend_from_slice(&1000u64.to_be_bytes());
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend_from_slice(toc);
        data.extend((0..20u8).map(|n| n * 11));
        data.extend_from_slice(b"the heap's files");
        data
    }

    #[test]
    fn a_package_ticket_is_filed_under_its_table_of_contents_checksum() {
        let dir = TempDir::new("sweetpad-staple");
        let path = dir.join("App.pkg");
        std::fs::write(&path, package()).unwrap();
        assert_eq!(
            package_record(&path).unwrap(),
            "2/1/000b16212c37424d58636e79848f9aa5b0bbc6d1"
        );
    }

    #[test]
    fn stapling_a_package_brackets_the_ticket_as_stapler_does() {
        let dir = TempDir::new("sweetpad-staple");
        let path = dir.join("App.pkg");
        let original = package();
        std::fs::write(&path, &original).unwrap();
        staple_package(&path, b"first ticket").unwrap();
        let stapled = std::fs::read(&path).unwrap();
        // As `stapler staple` wrote it for a Developer ID package.
        let mut expected = original.clone();
        expected.extend_from_slice(b"t8lr\x01\x00\x01\x00\0\0\0\0\0\0\0\0");
        expected.extend_from_slice(b"first ticket");
        expected.extend_from_slice(b"t8lr\x01\x00\x02\x00\x0c\0\0\0\0\0\0\0");
        assert_eq!(stapled, expected);
        assert_eq!(package_record(&path).unwrap(), package_record_of(&original));

        staple_package(&path, b"a later, longer ticket").unwrap();
        let restapled = std::fs::read(&path).unwrap();
        assert_eq!(&restapled[..original.len()], &original[..]);
        assert_eq!(
            restapled.len(),
            original.len() + 2 * RECORD_SIZE + b"a later, longer ticket".len()
        );
    }

    fn package_record_of(data: &[u8]) -> String {
        let dir = TempDir::new("sweetpad-staple");
        let path = dir.join("Copy.pkg");
        std::fs::write(&path, data).unwrap();
        package_record(&path).unwrap()
    }

    #[test]
    fn a_damaged_ticket_is_refused_rather_than_cut_wrong() {
        let mut data = package();
        data.extend_from_slice(&record(TRAILER, 5000));
        let error = xar_layout(&data, Path::new("App.pkg")).unwrap_err();
        assert!(error.contains("damaged"), "{error}");
    }
}
