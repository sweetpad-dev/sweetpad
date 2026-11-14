//! `sweetpad notarize`, end to end against a stub notary service, S3 and
//! ticket lookup. Each test runs with its own HOME.

mod common;
#[path = "common/stub.rs"]
mod stub;

use std::path::Path;

use base64::Engine as _;
use common::TempDir;
use serde_json::json;
use stub::{Received, Stub, data, error};

const TICKET: &[u8] = b"a notarization ticket";

/// The record the synthetic package's ticket is filed under.
const RECORD: &str = "2/1/000b16212c37424d58636e79848f9aa5b0bbc6d1";

#[derive(Clone, Copy, PartialEq)]
enum Verdict {
    Accepted,
    Invalid,
}

/// A stub notary service that decides on the second poll, and a ticket
/// service that files the ticket a moment after that.
fn serve(verdict: Verdict) -> Stub {
    let mut polls = 0;
    let mut lookups = 0;
    Stub::start(move |request, base| answer(verdict, request, base, &mut polls, &mut lookups))
}

fn answer(
    verdict: Verdict,
    r: &Received,
    base: &str,
    polls: &mut u32,
    lookups: &mut u32,
) -> (&'static str, String) {
    match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/notary/v2/submissions") => (
            "200 OK",
            json!({"data": {"type": "newSubmissions", "id": "S1", "attributes": {
                "awsAccessKeyId": "AKIDEXAMPLE", "awsSecretAccessKey": "SECRET",
                "awsSessionToken": "SESSION-TOKEN", "bucket": "notary-bucket",
                "object": "prod/AKIDEXAMPLE/S1"}}})
            .to_string(),
        ),
        ("PUT", "/s3/notary-bucket/prod/AKIDEXAMPLE/S1") => ("200 OK", String::new()),
        ("GET", "/notary/v2/submissions/S1") => {
            *polls += 1;
            let status = match (verdict, *polls) {
                (_, 1) => "In Progress",
                (Verdict::Accepted, _) => "Accepted",
                (Verdict::Invalid, _) => "Invalid",
            };
            (
                "200 OK",
                json!({"data": {"type": "submissions", "id": "S1", "attributes": {
                    "name": "Delivery", "status": status,
                    "createdDate": "2026-11-14T10:00:00.000Z"}}})
                .to_string(),
            )
        }
        ("GET", "/notary/v2/submissions/S1/logs") => (
            "200 OK",
            json!({"data": {"type": "submissionsLog", "id": "S1", "attributes": {
                "developerLogUrl": format!("{base}/log/S1")}}})
            .to_string(),
        ),
        ("GET", "/log/S1") if verdict == Verdict::Accepted => (
            "200 OK",
            json!({"status": "Accepted", "statusSummary": "Ready for distribution", "issues": null})
                .to_string(),
        ),
        ("GET", "/log/S1") => (
            "200 OK",
            json!({"status": "Invalid", "statusSummary": "Archive contains critical validation errors",
                "issues": [{"severity": "error", "architecture": "arm64",
                    "path": "Delivery.zip/Delivery.app/Contents/MacOS/Delivery",
                    "message": "The binary is not signed with a valid Developer ID certificate."}]})
            .to_string(),
        ),
        ("POST", "/cloudkit/lookup") => {
            *lookups += 1;
            let record = r.json()["records"][0]["recordName"].clone();
            let found = if *lookups == 1 {
                json!({"recordName": record, "reason": "Did not find record",
                    "serverErrorCode": "NOT_FOUND"})
            } else {
                json!({"recordName": record, "recordType": "DeveloperIDTicket", "fields": {
                    "signedTicket": {"type": "BYTES",
                        "value": base64::engine::general_purpose::STANDARD.encode(TICKET)}}})
            };
            ("200 OK", json!({"records": [found]}).to_string())
        }
        _ => (
            "404 Not Found",
            json!({"errors": [{"status": "404", "title": "no route"}]}).to_string(),
        ),
    }
}

/// A flat package's first bytes: the header, a table of contents and the
/// heap's checksum, then a file.
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

/// A HOME with an API key, a package and a zip.
fn home(tag: &str) -> TempDir {
    let dir = TempDir::new(&format!("sweetpad-notarize-{tag}"));
    stub::write_key(&dir);
    std::fs::write(dir.join("Delivery.pkg"), package()).unwrap();
    std::fs::write(dir.join("Delivery.zip"), b"PK\x05\x06 an empty zip").unwrap();
    dir
}

fn sweetpad(home: &Path, stub: &Stub, args: &[&str]) -> std::process::Output {
    stub.command(home).args(args).output().unwrap()
}

#[test]
fn an_accepted_package_goes_to_s3_and_gets_its_ticket_stapled() {
    let home = home("accepted");
    let stub = serve(Verdict::Accepted);
    let out = sweetpad(&home, &stub, &["notarize", "Delivery.pkg", "-o", "json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let data = data(&out);
    assert_eq!(data["submissionId"], "S1");
    assert_eq!(data["status"], "Accepted");
    assert_eq!(data["issues"], json!([]));
    assert_eq!(data["stapled"].as_array().unwrap().len(), 1);

    let requests = stub.requests();
    let create = &requests[0];
    assert_eq!(create.path, "/notary/v2/submissions");
    assert_eq!(create.json()["submissionName"], "Delivery.pkg");
    let digest = create.json()["sha256"].as_str().unwrap().to_string();
    assert_eq!(digest.len(), 64);
    assert!(
        create
            .header("authorization")
            .unwrap()
            .starts_with("Bearer ")
    );

    let put = requests.iter().find(|r| r.method == "PUT").unwrap();
    assert_eq!(put.body, package());
    assert_eq!(put.header("x-amz-content-sha256"), Some(digest.as_str()));
    assert_eq!(put.header("x-amz-security-token"), Some("SESSION-TOKEN"));
    let authorization = put.header("authorization").unwrap();
    assert!(
        authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/")
            && authorization.contains("/us-west-2/s3/aws4_request")
            && authorization.contains(
                "SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token"
            ),
        "{authorization}"
    );

    let lookups: Vec<&Received> = requests
        .iter()
        .filter(|r| r.path == "/cloudkit/lookup")
        .collect();
    assert_eq!(
        lookups.len(),
        2,
        "the first lookup comes before Apple files the ticket"
    );
    assert_eq!(lookups[1].json()["records"][0]["recordName"], RECORD);
    assert!(lookups[1].header("authorization").is_none());

    let mut expected = package();
    expected.extend_from_slice(b"t8lr\x01\x00\x01\x00\0\0\0\0\0\0\0\0");
    expected.extend_from_slice(TICKET);
    expected.extend_from_slice(&[b't', b'8', b'l', b'r', 1, 0, 2, 0]);
    expected.extend_from_slice(&(TICKET.len() as u64).to_le_bytes());
    assert_eq!(std::fs::read(home.join("Delivery.pkg")).unwrap(), expected);

    // Stapling again replaces the ticket rather than adding a second one.
    let again = sweetpad(
        &home,
        &stub,
        &["notarize", "staple", "Delivery.pkg", "-o", "json"],
    );
    assert!(again.status.success());
    assert_eq!(stub::data(&again)["record"], RECORD);
    assert_eq!(std::fs::read(home.join("Delivery.pkg")).unwrap(), expected);
}

#[test]
fn an_invalid_submission_shows_apple_issues_and_exits_1() {
    let home = home("invalid");
    let stub = serve(Verdict::Invalid);
    let out = sweetpad(&home, &stub, &["notarize", "Delivery.zip", "-o", "json"]);
    assert_eq!(out.status.code(), Some(1));
    let data = data(&out);
    assert_eq!(data["status"], "Invalid");
    assert_eq!(
        data["issues"],
        json!([
            "Delivery.zip/Delivery.app/Contents/MacOS/Delivery (arm64): \
                The binary is not signed with a valid Developer ID certificate."
        ])
    );
    assert!(
        !stub.requests().iter().any(|r| r.path == "/cloudkit/lookup"),
        "nothing is stapled"
    );

    let log = sweetpad(&home, &stub, &["notarize", "log", "S1"]);
    assert!(log.status.success());
    let text = String::from_utf8_lossy(&log.stdout);
    assert!(
        text.contains("Archive contains critical validation errors (Invalid)"),
        "{text}"
    );
    assert!(
        text.contains("not signed with a valid Developer ID"),
        "{text}"
    );

    let status = sweetpad(&home, &stub, &["notarize", "status", "S1"]);
    assert_eq!(status.status.code(), Some(1));
    let text = String::from_utf8_lossy(&status.stdout);
    assert!(
        text.contains("Apple refused to notarize the file"),
        "{text}"
    );
    assert!(text.contains("sweetpad notarize log S1"), "{text}");
}

#[test]
fn no_wait_returns_the_submission_to_follow_up() {
    let home = home("no-wait");
    let stub = serve(Verdict::Accepted);
    let out = sweetpad(
        &home,
        &stub,
        &["notarize", "Delivery.zip", "--no-wait", "-o", "json"],
    );
    assert!(out.status.success());
    let data = data(&out);
    assert_eq!(data["submissionId"], "S1");
    assert_eq!(data["status"], "In Progress");
    assert!(
        !stub.requests().iter().any(|r| r.method == "GET"),
        "--no-wait asks nothing after sending"
    );
}

#[test]
fn wrong_inputs_are_refused_before_any_request() {
    let home = home("usage");
    let stub = serve(Verdict::Accepted);
    std::fs::write(home.join("notes.txt"), "x").unwrap();
    for (args, needle) in [
        (
            vec!["notarize", "notes.txt"],
            "isn't an app, zip, package or disk image",
        ),
        (vec!["notarize", "missing.pkg"], "missing.pkg doesn't exist"),
        (vec!["notarize"], "notarize needs a file"),
        (
            vec!["notarize", "staple", "Delivery.zip"],
            "takes no ticket",
        ),
    ] {
        let out = sweetpad(&home, &stub, &[args.as_slice(), &["-o", "json"]].concat());
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let error = error(&out);
        assert_eq!(error["code"], "usage_error");
        assert!(
            error["message"].as_str().unwrap().contains(needle),
            "{error}"
        );
    }
    let out = stub
        .command(&home)
        .env_remove("SWEETPAD_ASC_ISSUER_ID")
        .args(["notarize", "Delivery.pkg", "-o", "json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let message = error(&out)["message"].as_str().unwrap().to_string();
    assert!(message.contains("needs a team API key"), "{message}");
    assert!(stub.requests().is_empty(), "no request went out");
}
