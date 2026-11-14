//! `sweetpad upload`, end to end against a stub App Store Connect. Each test
//! runs with its own HOME.

mod common;
#[path = "common/stub.rs"]
mod stub;

use std::path::Path;
use std::process::{Command, Output};

use common::TempDir;
use serde_json::json;
use stub::{Received, Stub, data, error};

/// How the stub behaves.
#[derive(Clone, Copy, PartialEq)]
enum Scenario {
    Accepted,
    Duplicate,
    Rejected,
}

/// A stub App Store Connect answering the Build Upload API's calls.
fn serve(scenario: Scenario) -> Stub {
    let mut polls = 0;
    let mut size = 0u64;
    Stub::start(move |request, base| answer(scenario, request, base, &mut polls, &mut size))
}

fn answer(
    scenario: Scenario,
    r: &Received,
    base: &str,
    polls: &mut u32,
    size: &mut u64,
) -> (&'static str, String) {
    let path = r.path.split('?').next().unwrap_or_default();
    match (r.method.as_str(), path) {
        ("GET", "/v1/apps") => (
            "200 OK",
            json!({"data": [
                {"type": "apps", "id": "111", "attributes": {"bundleId": "dev.example.app.other"}},
                {"type": "apps", "id": "1234567890", "attributes": {"bundleId": "dev.example.app"}},
            ]})
            .to_string(),
        ),
        ("POST", "/v1/buildUploads") if scenario == Scenario::Duplicate => (
            "409 Conflict",
            json!({"errors": [{"status": "409", "code": "ENTITY_ERROR.ATTRIBUTE.INVALID.DUPLICATE",
                "title": "The provided entity includes an attribute with a value that has already been used",
                "detail": "The bundle version must be higher than the previously uploaded version."}]})
            .to_string(),
        ),
        ("POST", "/v1/buildUploads") => (
            "201 Created",
            json!({"data": {"type": "buildUploads", "id": "U1"}}).to_string(),
        ),
        ("POST", "/v1/buildUploadFiles") => {
            *size = r.json()["data"]["attributes"]["fileSize"].as_u64().unwrap();
            let half = *size / 2;
            let op = |n: u32, offset: u64, length: u64| {
                json!({"method": "PUT", "url": format!("{base}/part/{n}"), "offset": offset,
                       "length": length, "partNumber": n,
                       "requestHeaders": [{"name": "Content-Type", "value": "application/octet-stream"}]})
            };
            (
                "201 Created",
                json!({"data": {"type": "buildUploadFiles", "id": "F1", "attributes": {
                    "uploadOperations": [op(1, 0, half), op(2, half, *size - half)]}}})
                .to_string(),
            )
        }
        ("PUT", _) => ("200 OK", String::new()),
        ("PATCH", "/v1/buildUploadFiles/F1") => ("200 OK", json!({"data": {"id": "F1"}}).to_string()),
        // Apple answers 400 to any include but build here: an upload names no app.
        ("GET", "/v1/buildUploads/U1") if r.path.contains("include=") && !r.path.ends_with("?include=build") => (
            "400 Bad Request",
            json!({"errors": [{"status": "400", "code": "PARAMETER_ERROR.INVALID",
                "title": "A parameter has an invalid value",
                "detail": "The relationship 'app' cannot be included"}]})
            .to_string(),
        ),
        ("GET", "/v1/buildUploads/U1") => {
            *polls += 1;
            let state = match (scenario, *polls) {
                (_, 1) => json!({"state": "PROCESSING"}),
                (Scenario::Rejected, _) => json!({"state": "FAILED", "errors": [
                    {"code": "90725", "description": "SDK version issue."}]}),
                _ => json!({"state": "COMPLETE", "warnings": [
                    {"code": "90899", "description": "Something to look at."}]}),
            };
            let mut data = json!({"type": "buildUploads", "id": "U1", "attributes": {
                "cfBundleShortVersionString": "1.2", "cfBundleVersion": "42", "platform": "IOS",
                "state": state}});
            if *polls > 1 && scenario == Scenario::Accepted {
                data["relationships"]["build"] = json!({"data": {"type": "builds", "id": "B1"}});
            }
            ("200 OK", json!({"data": data, "included": []}).to_string())
        }
        ("GET", "/v1/builds/B1") => (
            "200 OK",
            json!({"data": {"type": "builds", "id": "B1", "attributes": {"processingState": "VALID"}}})
                .to_string(),
        ),
        _ => ("404 Not Found", json!({"errors": [{"status": "404", "title": "no route"}]}).to_string()),
    }
}

/// A HOME with an API key and an IPA for dev.example.app 1.2 (42).
fn home(tag: &str) -> TempDir {
    let dir = TempDir::new(&format!("sweetpad-upload-{tag}"));
    stub::write_key(&dir);
    let app = dir.join("Payload/Delivery.app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("Info.plist"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
         <key>CFBundleIdentifier</key><string>dev.example.app</string>\
         <key>CFBundleShortVersionString</key><string>1.2</string>\
         <key>CFBundleVersion</key><string>42</string>\
         <key>DTPlatformName</key><string>iphoneos</string></dict></plist>\n",
    )
    .unwrap();
    // Incompressible bytes, so both parts carry something.
    let filler: Vec<u8> = (0..200_000u32)
        .map(|i| i.wrapping_mul(2_654_435_761).to_be_bytes()[1])
        .collect();
    std::fs::write(app.join("Delivery"), filler).unwrap();
    let status = Command::new("zip")
        .args(["-qr", "Delivery.ipa", "Payload"])
        .current_dir(&*dir)
        .status()
        .unwrap();
    assert!(status.success());
    dir
}

fn sweetpad(home: &Path, stub: &Stub, args: &[&str]) -> Output {
    stub.command(home).args(args).output().unwrap()
}

#[test]
fn an_accepted_upload_sends_every_part_and_follows_processing() {
    let home = home("accepted");
    let stub = serve(Scenario::Accepted);
    let out = sweetpad(
        &home,
        &stub,
        &["upload", "Delivery.ipa", "--wait", "-o", "json"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let data = data(&out);
    assert_eq!(data["uploadId"], "U1");
    assert_eq!(
        data["app"],
        json!({"id": "1234567890", "bundleId": "dev.example.app"})
    );
    assert_eq!(
        (data["version"].as_str(), data["build"].as_str()),
        (Some("1.2"), Some("42"))
    );
    assert_eq!(data["platform"], "IOS");
    assert_eq!(data["state"], "COMPLETE");
    assert_eq!(data["buildId"], "B1");
    assert_eq!(data["processingState"], "VALID");
    assert_eq!(data["warnings"][0]["code"], "90899");

    let requests = stub.requests();
    let create = requests
        .iter()
        .find(|r| r.path == "/v1/buildUploads")
        .unwrap();
    assert_eq!(
        create.json()["data"]["attributes"],
        json!({"cfBundleShortVersionString": "1.2", "cfBundleVersion": "42", "platform": "IOS"})
    );
    assert_eq!(
        create.json()["data"]["relationships"]["app"]["data"]["id"],
        "1234567890"
    );
    let bearer = create.header("authorization").unwrap();
    assert!(
        bearer.starts_with("Bearer ") && bearer.matches('.').count() == 2,
        "{bearer}"
    );
    let file = requests
        .iter()
        .find(|r| r.path == "/v1/buildUploadFiles")
        .unwrap();
    assert_eq!(file.json()["data"]["attributes"]["uti"], "com.apple.ipa");
    assert_eq!(
        file.json()["data"]["attributes"]["fileName"],
        "Delivery.ipa"
    );
    let mut parts: Vec<&Received> = requests.iter().filter(|r| r.method == "PUT").collect();
    parts.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(parts.len(), 2);
    assert!(
        parts.iter().all(|p| p.header("authorization").is_none()),
        "part URLs take no token"
    );
    assert_eq!(
        parts[0].header("content-type"),
        Some("application/octet-stream")
    );
    let sent: Vec<u8> = parts.iter().flat_map(|p| p.body.clone()).collect();
    assert_eq!(sent, std::fs::read(home.join("Delivery.ipa")).unwrap());
    let commit = requests.iter().find(|r| r.method == "PATCH").unwrap();
    assert_eq!(commit.json()["data"]["attributes"]["uploaded"], true);
    let commit_at = requests.iter().position(|r| r.method == "PATCH").unwrap();
    assert!(requests.iter().rposition(|r| r.method == "PUT").unwrap() < commit_at);
}

#[test]
fn a_used_build_number_says_to_raise_it() {
    let home = home("duplicate");
    let stub = serve(Scenario::Duplicate);
    let out = sweetpad(
        &home,
        &stub,
        &[
            "upload",
            "Delivery.ipa",
            "--app-id",
            "1234567890",
            "-o",
            "json",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    let error = error(&out);
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("build 42 of version 1.2 is already uploaded"),
        "{message}"
    );
    assert!(message.contains("CURRENT_PROJECT_VERSION"), "{message}");
    assert!(
        !stub.requests().iter().any(|r| r.path == "/v1/apps"),
        "--app-id skips the lookup"
    );
}

#[test]
fn a_rejected_upload_reports_apple_errors_and_exits_1() {
    let home = home("rejected");
    let stub = serve(Scenario::Rejected);
    let out = sweetpad(&home, &stub, &["upload", "Delivery.ipa", "-o", "json"]);
    assert_eq!(out.status.code(), Some(1));
    let data = data(&out);
    assert_eq!(data["state"], "FAILED");
    assert_eq!(
        data["errors"],
        json!([{"code": "90725", "description": "SDK version issue."}])
    );

    let human = sweetpad(&home, &stub, &["upload", "status", "U1"]);
    assert_eq!(human.status.code(), Some(1));
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("Apple rejected the upload"), "{text}");
    assert!(text.contains("ITMS-90725: SDK version issue."), "{text}");
    // An upload doesn't name its app, so status can't show one.
    assert!(!text.contains("  app "), "{text}");
}

#[test]
fn missing_inputs_are_usage_errors_before_any_request() {
    let home = home("usage");
    let stub = serve(Scenario::Accepted);
    std::fs::write(home.join("notes.txt"), "x").unwrap();
    for (args, needle) in [
        (vec!["upload", "notes.txt"], "isn't an .ipa or a .pkg"),
        (vec!["upload", "missing.ipa"], "can't read missing.ipa"),
        (vec!["upload"], "upload needs a file"),
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
        .env_remove("SWEETPAD_ASC_KEY_ID")
        .args(["upload", "Delivery.ipa", "-o", "json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        error(&out)["message"]
            .as_str()
            .unwrap()
            .contains("--api-key")
    );
    assert!(stub.requests().is_empty(), "no request went out");
}
