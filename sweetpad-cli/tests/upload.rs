//! `sweetpad upload`, end to end against a stub App Store Connect on
//! 127.0.0.1, reached through the hidden `SWEETPAD_ASC_BASE_URL`, so no test
//! can reach Apple. Each test runs with its own HOME.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use common::TempDir;
use p256::ecdsa::SigningKey;
use p256::pkcs8::{EncodePrivateKey, LineEnding};
use serde_json::{Value, json};

/// One request the stub received.
#[derive(Clone, Debug)]
struct Received {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

/// How the stub behaves.
#[derive(Clone, Copy, PartialEq)]
enum Scenario {
    Accepted,
    Duplicate,
    Rejected,
}

/// A stub App Store Connect answering the Build Upload API's calls.
struct Stub {
    base: String,
    received: Arc<Mutex<Vec<Received>>>,
}

impl Stub {
    fn start(scenario: Scenario) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&received);
        let url = base.clone();
        std::thread::spawn(move || {
            let mut polls = 0;
            let mut size = 0u64;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let request = read_request(&mut stream);
                log.lock().unwrap().push(request.clone());
                let (status, body) = answer(scenario, &request, &url, &mut polls, &mut size);
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
            }
        });
        Self { base, received }
    }

    fn requests(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }
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

fn read_request(stream: &mut std::net::TcpStream) -> Received {
    let mut data = Vec::new();
    let mut chunk = vec![0u8; 65536];
    let header_end = loop {
        if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        let n = stream.read(&mut chunk).unwrap();
        assert!(n > 0, "the connection closed inside the headers");
        data.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8(data[..header_end].to_vec()).unwrap();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap().split(' ');
    let method = request_line.next().unwrap().to_string();
    let path = request_line.next().unwrap().to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .map_or(0, |(_, v)| v.parse().unwrap());
    let mut body = data[header_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).unwrap();
        assert!(n > 0, "the connection closed inside the body");
        body.extend_from_slice(&chunk[..n]);
    }
    Received {
        method,
        path,
        headers,
        body,
    }
}

/// A HOME with an API key and an IPA for dev.example.app 1.2 (42).
fn home(tag: &str) -> TempDir {
    let dir = TempDir::new(&format!("sweetpad-upload-{tag}"));
    let key = SigningKey::from_slice(&[9u8; 32]).unwrap();
    let keys = dir.join(".appstoreconnect/private_keys");
    std::fs::create_dir_all(&keys).unwrap();
    std::fs::write(
        keys.join("AuthKey_TESTKEY123.p8"),
        key.to_pkcs8_pem(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
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
    let mut command = Command::new(env!("CARGO_BIN_EXE_sweetpad"));
    for var in [
        "https_proxy",
        "HTTPS_PROXY",
        "http_proxy",
        "all_proxy",
        "ALL_PROXY",
        "no_proxy",
        "NO_PROXY",
        "SWEETPAD_ASC_KEY",
        "SWEETPAD_ASC_KEY_PATH",
        "API_PRIVATE_KEYS_DIR",
    ] {
        command.env_remove(var);
    }
    command
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home)
        .env("TMPDIR", home)
        .env("SWEETPAD_ASC_BASE_URL", &stub.base)
        .env("SWEETPAD_ASC_POLL_SECS", "0")
        .env("SWEETPAD_ASC_KEY_ID", "TESTKEY123")
        .env(
            "SWEETPAD_ASC_ISSUER_ID",
            "57246542-96fe-1a63-e053-0824d011072a",
        )
        .output()
        .expect("failed to run the sweetpad binary")
}

fn data(out: &Output) -> Value {
    let text = String::from_utf8_lossy(&out.stdout);
    let value: Value = serde_json::from_str(text.trim()).unwrap_or_else(|e| {
        panic!(
            "stdout is not one JSON value ({e}):\n{text}\n{}",
            String::from_utf8_lossy(&out.stderr)
        )
    });
    assert_eq!(value["ok"], true, "{value}");
    value["data"].clone()
}

fn error(out: &Output) -> Value {
    let text = String::from_utf8_lossy(&out.stderr);
    let last = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    let value: Value = serde_json::from_str(last)
        .unwrap_or_else(|e| panic!("stderr's last line is no error envelope ({e}):\n{text}"));
    value["error"].clone()
}

#[test]
fn an_accepted_upload_sends_every_part_and_follows_processing() {
    let home = home("accepted");
    let stub = Stub::start(Scenario::Accepted);
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
    let stub = Stub::start(Scenario::Duplicate);
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
    let stub = Stub::start(Scenario::Rejected);
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
    let stub = Stub::start(Scenario::Accepted);
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
    let command_args = vec!["upload", "Delivery.ipa", "-o", "json"];
    let out = {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sweetpad"));
        command
            .args(&command_args)
            .current_dir(&*home)
            .env("HOME", &*home)
            .env("SWEETPAD_ASC_BASE_URL", &stub.base)
            .env_remove("SWEETPAD_ASC_KEY_ID")
            .env_remove("SWEETPAD_ASC_KEY")
            .output()
            .unwrap()
    };
    assert_eq!(out.status.code(), Some(2));
    assert!(
        error(&out)["message"]
            .as_str()
            .unwrap()
            .contains("--api-key")
    );
    assert!(stub.requests().is_empty(), "no request went out");
}
