//! `sweetpad feedback`, end to end against a stub server on 127.0.0.1.
//!
//! Every invocation here points the send at a local `TcpListener` through the
//! hidden `SWEETPAD_FEEDBACK_URL`, so no test can reach the real Sentry
//! project, and each runs with its own HOME and XDG directories.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::mpsc;
use std::time::Duration;

use common::TempDir;
use serde_json::Value;

const REPORT: &str = "\
## 2026-09-27T10:00Z · bug · medium
- **Seen:** 3× (first 2026-09-20T08:00Z, last 2026-09-27T10:00Z)
  - 2026-09-27 · sweetpad 0.1.10 · ~/Developer/Other — same failure
- **Context:** an iOS app in a workspace; the user asked to run it
- **Command:** `sweetpad run --on <simulator> --no-logs`
- **Expected:** the app launches and the command returns
- **Actual:** exit 1: `error: couldn't find the built app for <scheme>`
- **Assumption or gap:** the build succeeded, so the app exists
- **Fix idea:** look for the app where the build wrote it
";

/// One request the stub received.
struct Received {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A local envelope endpoint answering every request with `status` and
/// `body`, and handing each request to the test. As a proxy it answers a
/// `CONNECT` the same way.
struct Stub {
    url: String,
    /// `127.0.0.1:<port>`.
    addr: String,
    received: mpsc::Receiver<Received>,
}

impl Stub {
    fn start(status: &'static str, body: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, received) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let request = read_request(&mut stream);
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
                if tx.send(request).is_err() {
                    break;
                }
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}/api/1/envelope/"),
            addr: format!("127.0.0.1:{port}"),
            received,
        }
    }

    fn next(&self) -> Received {
        self.received
            .recv_timeout(Duration::from_secs(10))
            .expect("the stub received no request")
    }

    fn assert_nothing_received(&self) {
        assert!(
            self.received
                .recv_timeout(Duration::from_millis(500))
                .is_err(),
            "the stub received a request"
        );
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Received {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
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
    let request_line = lines.next().unwrap().to_string();
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
        request_line,
        headers,
        body: String::from_utf8(body).unwrap(),
    }
}

/// A scratch HOME holding the report, with the XDG dirs inside it.
fn home(tag: &str) -> TempDir {
    let dir = TempDir::new(&format!("sweetpad-feedback-{tag}"));
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(dir.join("entry.txt"), REPORT).unwrap();
    dir
}

/// Run sweetpad in `home` with its sends going to `endpoint`.
fn sweetpad(home: &Path, endpoint: &str, args: &[&str]) -> Output {
    sweetpad_with(home, endpoint, &[], args)
}

/// [`sweetpad`] with `env` set, and no proxy variable but those in it.
fn sweetpad_with(home: &Path, endpoint: &str, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sweetpad"));
    for var in [
        "https_proxy",
        "HTTPS_PROXY",
        "http_proxy",
        "HTTP_PROXY",
        "all_proxy",
        "ALL_PROXY",
        "no_proxy",
        "NO_PROXY",
    ] {
        command.env_remove(var);
    }
    command.envs(env.iter().copied());
    command
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home)
        .env("TMPDIR", home)
        .env("SWEETPAD_FEEDBACK_URL", endpoint)
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .output()
        .expect("failed to run the sweetpad binary")
}

fn stdout_json(out: &Output) -> Value {
    let text = String::from_utf8_lossy(&out.stdout);
    let value: Value = serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("stdout is not one JSON value ({e}):\n{text}"));
    assert_eq!(value["ok"], true, "{value}");
    value["data"].clone()
}

/// The error envelope on stderr's last line.
fn stderr_error(out: &Output) -> Value {
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

fn dry_run(home: &Path, endpoint: &str) -> Value {
    let out = sweetpad(
        home,
        endpoint,
        &["feedback", "submit", "entry.txt", "--dry-run", "-o", "json"],
    );
    assert!(out.status.success(), "{out:?}");
    stdout_json(&out)
}

#[test]
fn a_dry_run_prints_the_payload_and_sends_nothing() {
    let stub = Stub::start("200 OK", r#"{"id":"x"}"#);
    let home = home("dry");
    let data = dry_run(&home, &stub.url);

    assert_eq!(data["sent"], false);
    assert_eq!(data["endpoint"], stub.url);
    let digest = data["digest"].as_str().unwrap();
    assert_eq!(digest.len(), 16);
    assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()), "{digest}");
    assert_eq!(
        data["sendCommand"],
        format!("sweetpad feedback submit entry.txt --approve {digest}")
    );
    assert_eq!(data["addedAtSend"], serde_json::json!(["timestamp"]));

    let payload = &data["payload"];
    assert_eq!(payload["platform"], "other");
    assert_eq!(payload["level"], "info");
    assert_eq!(payload["sdk"]["name"], "sweetpad-cli");
    assert_eq!(payload["sdk"]["settings"]["infer_ip"], "never");
    assert_eq!(payload["tags"]["source"], "cli");
    assert_eq!(payload["tags"]["kind"], "bug");
    assert_eq!(payload["tags"]["severity"], "medium");
    assert!(
        payload["release"]
            .as_str()
            .unwrap()
            .starts_with("sweetpad-cli@")
    );
    assert_eq!(payload["contexts"]["os"]["name"], "macOS");
    let event_id = payload["event_id"].as_str().unwrap();
    assert_eq!(event_id.len(), 32, "{event_id}");
    assert_eq!(&event_id[12..13], "4", "{event_id}");
    let message = payload["contexts"]["feedback"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("bug · medium\n\nContext: an iOS app"),
        "{message}"
    );
    assert!(message.contains("Fix idea: look for the app"), "{message}");
    // The heading's timestamp and the Seen line stay local.
    assert!(!message.contains("2026-09-2"), "{message}");
    assert!(!message.contains("~/Developer/Other"), "{message}");

    // No address anywhere, and no user object for Relay to put one in.
    assert!(payload.get("user").is_none(), "{payload}");
    assert!(payload.get("timestamp").is_none(), "{payload}");
    let text = payload.to_string();
    assert!(
        !text.contains("ip_address") && !text.contains("{{auto}}"),
        "{text}"
    );

    // The human form shows the same payload and digest.
    let out = sweetpad(
        &home,
        &stub.url,
        &["feedback", "submit", "entry.txt", "--dry-run"],
    );
    let human = String::from_utf8_lossy(&out.stdout);
    assert!(human.starts_with("Nothing was sent."), "{human}");
    assert!(human.contains(&format!("digest: {digest}")), "{human}");
    assert!(human.contains("Added by sweetpad:"), "{human}");
    assert!(human.contains("\"infer_ip\": \"never\""), "{human}");

    stub.assert_nothing_received();
}

#[test]
fn an_approved_report_is_sent_as_one_feedback_envelope() {
    let stub = Stub::start("200 OK", r#"{"id":"0123456789abcdef0123456789abcdef"}"#);
    let home = home("send");
    let dry = dry_run(&home, &stub.url);
    let digest = dry["digest"].as_str().unwrap();

    let out = sweetpad(
        &home,
        &stub.url,
        &[
            "feedback",
            "submit",
            "entry.txt",
            "--approve",
            digest,
            "-o",
            "json",
        ],
    );
    assert!(out.status.success(), "{out:?}");
    let data = stdout_json(&out);
    assert_eq!(data["sent"], true);
    assert_eq!(data["eventId"], "0123456789abcdef0123456789abcdef");
    assert_eq!(data["digest"], digest);

    let request = stub.next();
    assert_eq!(request.request_line, "POST /api/1/envelope/ HTTP/1.1");
    assert_eq!(
        request.header("Content-Type"),
        Some("application/x-sentry-envelope")
    );
    let auth = request.header("X-Sentry-Auth").unwrap();
    assert!(auth.starts_with("Sentry sentry_version=7"), "{auth}");
    assert!(
        auth.contains("sentry_key=5f6c5f5f15492a077f6bac254c6d8461"),
        "{auth}"
    );

    let lines: Vec<&str> = request.body.lines().collect();
    assert_eq!(lines.len(), 3, "{}", request.body);
    let header: Value = serde_json::from_str(lines[0]).unwrap();
    let item: Value = serde_json::from_str(lines[1]).unwrap();
    let event: Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(header["event_id"], dry["payload"]["event_id"]);
    assert_eq!(item["type"], "feedback");
    assert_eq!(item["length"], lines[2].len());

    // What went out is the dry run's payload plus the time of sending.
    assert!(
        event["timestamp"].as_u64().unwrap() > 1_700_000_000,
        "{event}"
    );
    let mut without_time = event.clone();
    without_time.as_object_mut().unwrap().remove("timestamp");
    assert_eq!(without_time, dry["payload"]);
    assert!(
        !request.body.contains("ip_address") && !request.body.contains("{{auto}}"),
        "{}",
        request.body
    );
    stub.assert_nothing_received();
}

#[test]
fn a_digest_that_no_longer_matches_sends_nothing() {
    let stub = Stub::start("200 OK", r#"{"id":"x"}"#);
    let home = home("mismatch");
    let digest = dry_run(&home, &stub.url)["digest"]
        .as_str()
        .unwrap()
        .to_string();

    // The file changes after the user saw the dry run.
    std::fs::write(
        home.join("entry.txt"),
        REPORT.replace("an iOS app", "a macOS app"),
    )
    .unwrap();
    let new_digest = dry_run(&home, &stub.url)["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(new_digest, digest);

    for approve in [digest.as_str(), "0000000000000000"] {
        let out = sweetpad(
            &home,
            &stub.url,
            &[
                "feedback",
                "submit",
                "entry.txt",
                "--approve",
                approve,
                "-o",
                "json",
            ],
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let error = stderr_error(&out);
        let message = error["message"].as_str().unwrap();
        assert!(message.contains("nothing was sent"), "{message}");
        assert!(message.contains("--dry-run' again"), "{message}");
        // The digest that would pass is not handed out here.
        assert!(!message.contains(&new_digest), "{message}");
    }
    stub.assert_nothing_received();
}

#[test]
fn submit_without_a_flag_is_a_usage_error_pointing_at_the_dry_run() {
    let stub = Stub::start("200 OK", r#"{"id":"x"}"#);
    let home = home("noflag");
    let out = sweetpad(
        &home,
        &stub.url,
        &["feedback", "submit", "entry.txt", "-o", "json"],
    );
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let error = stderr_error(&out);
    assert_eq!(error["code"], "usage_error");
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("'sweetpad feedback submit entry.txt --dry-run'"),
        "{message}"
    );

    // Both flags at once is clap's refusal.
    let out = sweetpad(
        &home,
        &stub.url,
        &[
            "feedback",
            "submit",
            "entry.txt",
            "--dry-run",
            "--approve",
            "0",
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    stub.assert_nothing_received();
}

#[test]
fn malformed_reports_are_usage_errors_that_name_the_problem() {
    let stub = Stub::start("200 OK", r#"{"id":"x"}"#);
    let home = home("malformed");
    let submit = |args: &[&str]| {
        let mut argv = vec!["feedback", "submit"];
        argv.extend_from_slice(args);
        argv.extend_from_slice(&["--dry-run", "-o", "json"]);
        let out = sweetpad(&home, &stub.url, &argv);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
        let error = stderr_error(&out);
        assert_eq!(error["code"], "usage_error", "{args:?}");
        error["message"].as_str().unwrap().to_string()
    };

    std::fs::write(
        home.join("partial.txt"),
        "## bug · high\n- **Context:** c\n- **Actual:** a\n",
    )
    .unwrap();
    let message = submit(&["partial.txt"]);
    assert!(
        message.contains("missing 'Command', 'Expected' and 'Assumption or gap'"),
        "{message}"
    );

    std::fs::write(home.join("heading.txt"), "- **Context:** c\n").unwrap();
    assert!(submit(&["heading.txt"]).contains("must start with the heading"));

    std::fs::write(
        home.join("field.txt"),
        REPORT.replace("**Context:**", "**Project:**"),
    )
    .unwrap();
    assert!(submit(&["field.txt"]).contains("unknown field 'Project'"));

    assert!(submit(&["absent.txt"]).contains("can't read absent.txt"));
    stub.assert_nothing_received();
}

#[test]
fn off_on_and_status_round_trip_and_keep_the_rest_of_the_config() {
    let stub = Stub::start("200 OK", r#"{"id":"x"}"#);
    let home = home("toggle");
    let config_dir = home.join("sweetpad");
    std::fs::create_dir_all(&config_dir).unwrap();
    let config = config_dir.join("config.toml");
    let original = "# personal defaults\n[defaults]\nconfiguration = \"Debug\"  # always\n";
    std::fs::write(&config, original).unwrap();
    let json = |args: &[&str]| {
        let mut argv = args.to_vec();
        argv.extend_from_slice(&["-o", "json"]);
        let out = sweetpad(&home, &stub.url, &argv);
        assert!(out.status.success(), "{args:?}: {out:?}");
        stdout_json(&out)
    };

    let status = json(&["feedback", "status"]);
    assert_eq!(status["enabled"], true);
    assert_eq!(status["setInConfig"], false);

    let off = json(&["feedback", "off"]);
    assert_eq!(off["enabled"], false);
    assert_eq!(off["changed"], true);
    let written = std::fs::read_to_string(&config).unwrap();
    assert_eq!(
        written,
        format!("{original}\n[feedback]\nenabled = false\n")
    );
    assert_eq!(json(&["feedback", "off"])["changed"], false);
    assert_eq!(json(&["feedback", "status"])["enabled"], false);

    // Off: submit refuses in both modes, and the help says only that.
    for mode in ["--dry-run", "--approve"] {
        let mut argv = vec!["feedback", "submit", "entry.txt", mode];
        if mode == "--approve" {
            argv.push("0000000000000000");
        }
        argv.extend_from_slice(&["-o", "json"]);
        let out = sweetpad(&home, &stub.url, &argv);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let message = stderr_error(&out)["message"].as_str().unwrap().to_string();
        assert!(message.contains("turned off"), "{message}");
    }
    let help = sweetpad(&home, &stub.url, &["help", "feedback"]);
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(
        text.contains("The user turned off feedback reports"),
        "{text}"
    );
    assert!(text.contains("'sweetpad feedback on'"), "{text}");
    assert!(!text.contains("submit"), "{text}");
    assert_eq!(
        json(&["help", "feedback"])["text"].as_str().unwrap().trim(),
        text.trim()
    );

    let on = json(&["feedback", "on"]);
    assert_eq!(on["enabled"], true);
    assert_eq!(on["changed"], true);
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        format!("{original}\n[feedback]\nenabled = true\n")
    );
    let status = json(&["feedback", "status"]);
    assert_eq!(status["enabled"], true);
    assert_eq!(status["setInConfig"], true);
    let help = sweetpad(&home, &stub.url, &["help", "feedback"]);
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(
        text.contains("sweetpad feedback submit <file> --dry-run"),
        "{text}"
    );
    stub.assert_nothing_received();
}

#[test]
fn feedback_on_leaves_a_missing_config_missing() {
    let stub = Stub::start("200 OK", r#"{"id":"x"}"#);
    let home = home("on-absent");
    let out = sweetpad(&home, &stub.url, &["feedback", "on"]);
    assert!(out.status.success(), "{out:?}");
    assert!(!home.join("sweetpad").join("config.toml").exists());
}

#[test]
fn a_send_that_cant_connect_is_reported_in_words() {
    // A port nothing listens on once this listener is gone.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}/api/1/envelope/");
    let home = home("offline");
    let digest = dry_run(&home, &url)["digest"].as_str().unwrap().to_string();
    let out = sweetpad(
        &home,
        &url,
        &[
            "feedback",
            "submit",
            "entry.txt",
            "--approve",
            &digest,
            "-o",
            "json",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let error = stderr_error(&out);
    assert_eq!(error["code"], "generic");
    let message = error["message"].as_str().unwrap();
    assert!(
        message.starts_with(&format!(
            "couldn't send the report: can't reach 127.0.0.1:{port}"
        )),
        "{message}"
    );
}

#[test]
fn a_report_the_server_refuses_is_an_error() {
    let stub = Stub::start("429 Too Many Requests", r#"{"detail":"rate limited"}"#);
    let home = home("refused");
    let digest = dry_run(&home, &stub.url)["digest"]
        .as_str()
        .unwrap()
        .to_string();
    let out = sweetpad(
        &home,
        &stub.url,
        &[
            "feedback",
            "submit",
            "entry.txt",
            "--approve",
            &digest,
            "-o",
            "json",
        ],
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let message = stderr_error(&out)["message"].as_str().unwrap().to_string();
    assert!(
        message.starts_with("the report was not accepted"),
        "{message}"
    );
    assert!(message.contains("HTTP 429 Too Many Requests"), "{message}");
    assert!(message.contains("rate limited"), "{message}");
    stub.next();
}

/// An endpoint no resolver answers for, so a send that skipped the proxy
/// reaches nothing at all.
const UNRESOLVABLE: &str = "https://sentry.invalid/api/1/envelope/";

fn approve(home: &Path, env: &[(&str, &str)]) -> Output {
    let digest = dry_run(home, UNRESOLVABLE)["digest"]
        .as_str()
        .unwrap()
        .to_string();
    sweetpad_with(
        home,
        UNRESOLVABLE,
        env,
        &[
            "feedback",
            "submit",
            "entry.txt",
            "--approve",
            &digest,
            "-o",
            "json",
        ],
    )
}

#[test]
fn an_https_send_asks_the_proxy_in_https_proxy_for_a_tunnel() {
    let proxy = Stub::start("403 Forbidden", "");
    let home = home("proxy");
    let value = format!("http://me:secret@{}/", proxy.addr);
    let out = approve(&home, &[("HTTPS_PROXY", &value)]);

    let connect = proxy.next();
    assert_eq!(connect.request_line, "CONNECT sentry.invalid:443 HTTP/1.1");
    assert_eq!(
        connect.header("Proxy-Authorization"),
        Some("Basic bWU6c2VjcmV0"),
        "me:secret, as Basic credentials"
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let message = stderr_error(&out)["message"].as_str().unwrap().to_string();
    assert_eq!(
        message,
        format!(
            "couldn't send the report: the proxy at {} (HTTPS_PROXY) wouldn't open a \
             connection to sentry.invalid",
            proxy.addr
        )
    );
}

#[test]
fn the_lower_case_variable_wins_and_a_proxy_asking_for_credentials_says_how() {
    let proxy = Stub::start("407 Proxy Authentication Required", "");
    let home = home("proxy-auth");
    let value = format!("http://{}", proxy.addr);
    let out = approve(
        &home,
        &[
            ("https_proxy", &value),
            ("HTTPS_PROXY", "http://127.0.0.1:9"),
        ],
    );

    let connect = proxy.next();
    assert_eq!(connect.request_line, "CONNECT sentry.invalid:443 HTTP/1.1");
    assert_eq!(connect.header("Proxy-Authorization"), None);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let message = stderr_error(&out)["message"].as_str().unwrap().to_string();
    assert_eq!(
        message,
        format!(
            "couldn't send the report: the proxy at {0} (https_proxy) asks for credentials; \
             put them in https_proxy as 'http://user:password@{0}'",
            proxy.addr
        )
    );
}

/// `no_proxy` is honored even for the lower-case `https_proxy`, which the
/// HTTP client would otherwise apply on its own.
#[test]
fn no_proxy_sends_around_the_proxy() {
    let proxy = Stub::start("403 Forbidden", "");
    let home = home("no-proxy");
    let value = format!("http://{}", proxy.addr);
    let out = approve(
        &home,
        &[("https_proxy", &value), ("no_proxy", "localhost,.invalid")],
    );

    proxy.assert_nothing_received();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let message = stderr_error(&out)["message"].as_str().unwrap().to_string();
    assert!(
        message.starts_with("couldn't send the report: can't reach sentry.invalid"),
        "{message}"
    );
    assert!(!message.contains("proxy"), "{message}");
}

#[test]
fn a_proxy_sweetpad_cant_use_is_refused_without_echoing_it() {
    let home = home("socks");
    let out = approve(
        &home,
        &[("HTTPS_PROXY", "socks5://me:secret@127.0.0.1:1080")],
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let message = stderr_error(&out)["message"].as_str().unwrap().to_string();
    assert_eq!(
        message,
        "HTTPS_PROXY names a proxy sweetpad can't use, so nothing was sent. It takes an HTTP \
         proxy, written 'http://host:port' or 'http://user:password@host:port'"
    );
}
