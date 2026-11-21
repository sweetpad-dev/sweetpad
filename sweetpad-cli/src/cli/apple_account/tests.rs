//! Synthetic accounts and canned responses; nothing here talks to Apple.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{Cursor, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::rc::Rc;

use base64::Engine as _;
use rsa::BigUint;
use serde_json::{Value, json};

use super::cookies::{Cookie, Jar, http_date};
use super::download::{
    self, Options, Release, archive_path, normalize_version, prerelease_links, select_release,
};
use super::http::{Call, Client, Exchange, Sent, Transport, read_body};
use super::login::Login;
use super::session::{Session, private_write};
use super::srp::{Srp, hashcash, n, pad, pbkdf2, sha};
use super::{login_with, logout, url};
use crate::cli::testdir::TempDir;

type Log = Rc<RefCell<Vec<Sent>>>;

struct Fake {
    handler: Box<dyn FnMut(&Sent) -> Exchange>,
    log: Log,
}

impl Transport for Fake {
    fn send(&mut self, request: &Sent, _timeout_secs: u64) -> Result<Exchange, String> {
        self.log.borrow_mut().push(request.clone());
        Ok((self.handler)(request))
    }
}

fn reply(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Exchange {
    Exchange {
        status,
        headers: headers
            .iter()
            .map(|(n, v)| ((*n).to_string(), (*v).to_string()))
            .collect(),
        body: Box::new(Cursor::new(body.to_vec())),
    }
}

fn json_reply(status: u16, headers: &[(&str, &str)], body: &Value) -> Exchange {
    reply(status, headers, body.to_string().as_bytes())
}

fn fake(handler: impl FnMut(&Sent) -> Exchange + 'static) -> (Client, Log) {
    let log = Log::default();
    let client = Client::new(Box::new(Fake {
        handler: Box::new(handler),
        log: log.clone(),
    }));
    (client, log)
}

/// A client that answers with `responses` in order, and the requests it saw.
fn queue(responses: Vec<Exchange>) -> (Client, Log, Rc<RefCell<VecDeque<Exchange>>>) {
    let pending = Rc::new(RefCell::new(VecDeque::from(responses)));
    let left = pending.clone();
    let (client, log) = fake(move |sent| {
        left.borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("no response left for {} {}", sent.method, sent.url))
    });
    (client, log, pending)
}

fn cookie(domain: &str, specified: bool) -> Cookie {
    Cookie {
        name: "synthetic".into(),
        value: "session-secret".into(),
        domain: domain.into(),
        domain_specified: specified,
        path: "/".into(),
        expires: None,
    }
}

fn no_prompt() -> impl FnMut(&str, bool) -> Result<String, String> {
    |question: &str, _| panic!("asked {question:?}")
}

#[test]
fn gzip_bodies_are_decoded_with_a_limit() {
    use std::io::Write as _;
    let gzip = |data: &[u8]| {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    };
    let headers = vec![("Content-Encoding".to_string(), "gzip".to_string())];
    let body = gzip(br#"{"resultCode":0,"downloads":[]}"#);
    let data = read_body(&headers, &mut Cursor::new(body), 1024).unwrap();
    let value: Value = serde_json::from_slice(&data).unwrap();
    assert_eq!(value["downloads"], json!([]));
    let err = read_body(&headers, &mut Cursor::new(gzip(&[b'a'; 1000])), 100).unwrap_err();
    assert!(err.contains("larger"), "{err}");
    let brotli = vec![("Content-Encoding".to_string(), "br".to_string())];
    assert!(read_body(&brotli, &mut Cursor::new(Vec::new()), 100).is_err());
}

#[test]
fn the_service_key_lookup_sends_no_cookies_and_does_not_follow_the_logout() {
    let (client, log, _) = queue(vec![reply(
        302,
        &[
            (
                "Location",
                "https://idmsa.apple.com/appleauth/auth/signin?widgetKey=public-key",
            ),
            ("Set-Cookie", "tracking=1; Domain=apple.com; Path=/"),
        ],
        b"",
    )]);
    let mut session = Session::new("unused".into(), client);
    session.client.cookies.set(cookie(".apple.com", true));
    let mut prompt = no_prompt();
    let mut note = |_: &str| {};
    let key = Login::new(&mut session, &mut prompt, &mut note)
        .service_key()
        .unwrap();
    assert_eq!(key, "public-key");
    let sent = &log.borrow()[0];
    assert_eq!(sent.method, "HEAD");
    assert_eq!(sent.header("Cookie"), None);
    assert_eq!(log.borrow().len(), 1, "the redirect was followed");
    assert_eq!(session.client.cookies.iter().count(), 1);
}

#[test]
fn the_password_is_read_from_the_terminal_without_echo() {
    let (mut master, mut slave) = (0, 0);
    // SAFETY: openpty fills two descriptors this test then owns.
    let opened = unsafe {
        libc::openpty(
            &raw mut master,
            &raw mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(opened, 0);
    // SAFETY: both descriptors were just opened and are owned here.
    let (master, slave) = unsafe {
        (
            std::fs::File::from_raw_fd(master),
            std::fs::File::from_raw_fd(slave),
        )
    };
    let worker = std::thread::spawn(move || super::tty::ask_on(&slave, "Password: ", true));
    let mut transcript = Vec::new();
    let mut sent = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        let mut poll = libc::pollfd {
            fd: master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: polling one descriptor this test owns.
        if unsafe { libc::poll(&raw mut poll, 1, 100) } > 0 {
            let mut buffer = [0u8; 256];
            let read = (&master).read(&mut buffer).unwrap_or(0);
            transcript.extend_from_slice(&buffer[..read]);
        }
        let text = String::from_utf8_lossy(&transcript);
        if text.contains("Password: ") && !sent {
            // SAFETY: reading the pty's settings into a termios owned here.
            let mut term: libc::termios = unsafe { std::mem::zeroed() };
            assert_eq!(
                unsafe { libc::tcgetattr(master.as_raw_fd(), &raw mut term) },
                0
            );
            assert_eq!(term.c_lflag & libc::ECHO, 0, "echo is on");
            std::io::Write::write_all(&mut &master, b"synthetic-tty-password\n").unwrap();
            sent = true;
        }
        if sent && worker.is_finished() {
            break;
        }
    }
    assert_eq!(worker.join().unwrap().unwrap(), "synthetic-tty-password");
    assert!(!String::from_utf8_lossy(&transcript).contains("synthetic-tty-password"));
}

#[test]
fn a_saved_session_round_trips_with_session_cookies_and_private_modes() {
    let dir = TempDir::new("apple-session");
    let path = dir.join("private/sweetpad/state/session.json");
    let (client, _, _) = queue(Vec::new());
    let mut session = Session::new(path.clone(), client);
    session.apple_id = Some("synthetic@example.com".into());
    session.client.cookies.set(cookie(".apple.com", true));
    session.save().unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    for folder in [
        path.parent().unwrap(),
        &dir.join("private/sweetpad"),
        &dir.join("private"),
    ] {
        assert_eq!(mode(folder), 0o700, "{}", folder.display());
    }
    let stored: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let keys: Vec<&String> = stored.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["appleId", "cookies", "version"]);
    let (client, _, _) = queue(Vec::new());
    let mut loaded = Session::new(path, client);
    assert!(loaded.load(true).unwrap());
    assert_eq!(loaded.apple_id.as_deref(), Some("synthetic@example.com"));
    let cookies: Vec<&Cookie> = loaded.client.cookies.iter().collect();
    assert_eq!(cookies, [&cookie(".apple.com", true)]);
}

#[test]
fn a_session_file_is_refused_when_shared_linked_or_holding_other_domains() {
    let dir = TempDir::new("apple-session-unsafe");
    let path = dir.join("session.json");
    let (client, _, _) = queue(Vec::new());
    let mut session = Session::new(path.clone(), client);
    session.apple_id = Some("synthetic@example.com".into());
    session.save().unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let err = session.load(false).unwrap_err();
    assert!(err.contains("mode 600"), "{err}");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    let alias = dir.join("link");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    let (client, _, _) = queue(Vec::new());
    let err = Session::new(alias, client).load(false).unwrap_err();
    assert!(err.contains("symbolic link"), "{err}");

    let mut data: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    data["cookies"] = json!([{"name": "x", "value": "secret", "domain": "apple.com.evil.example",
        "domainSpecified": true, "path": "/", "expires": null}]);
    std::fs::write(&path, data.to_string()).unwrap();
    let err = session.load(false).unwrap_err();
    assert!(err.contains("cookie"), "{err}");
    assert!(!err.contains("secret"), "{err}");

    let err = Session::new(dir.join("missing.json"), queue(Vec::new()).0)
        .load(true)
        .unwrap_err();
    assert!(err.contains("auth login"), "{err}");
}

#[test]
fn host_only_cookies_go_to_their_host_over_https_only() {
    let mut jar = Jar::default();
    jar.set(cookie("idmsa.apple.com", false));
    for (target, expected) in [
        ("https://idmsa.apple.com/", true),
        ("https://idmsa.apple.com/appleauth/auth", true),
        ("https://child.idmsa.apple.com/", false),
        ("http://idmsa.apple.com/", false),
        ("https://example.com/", false),
    ] {
        assert_eq!(jar.header(target, 0).is_some(), expected, "{target}");
    }
    let mut jar = Jar::default();
    jar.set(cookie(".apple.com", true));
    for (target, expected) in [
        ("https://apple.com/", true),
        ("https://developer.apple.com/", true),
        ("https://notapple.com/", false),
    ] {
        assert_eq!(jar.header(target, 0).is_some(), expected, "{target}");
    }
}

#[test]
fn set_cookie_keeps_domain_path_and_expiry_and_refuses_other_domains() {
    let mut jar = Jar::default();
    let headers: Vec<(String, String)> = [
        "myacinfo=abc; Domain=.apple.com; Path=/; Secure; HttpOnly",
        "dslang=US-EN; Path=/appleauth; Max-Age=3600",
        "stolen=1; Domain=example.com; Path=/",
        "gone=1; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
        "broken",
    ]
    .iter()
    .map(|v| ("Set-Cookie".to_string(), (*v).to_string()))
    .collect();
    jar.receive(
        "https://idmsa.apple.com/appleauth/auth/signin",
        &headers,
        1000,
    );
    let names: Vec<(&str, &str, &str, Option<i64>)> = jar
        .iter()
        .map(|c| {
            (
                c.name.as_str(),
                c.domain.as_str(),
                c.path.as_str(),
                c.expires,
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            ("myacinfo", ".apple.com", "/", None),
            ("dslang", "idmsa.apple.com", "/appleauth", Some(4600)),
        ]
    );
    assert_eq!(
        jar.header("https://idmsa.apple.com/appleauth/auth", 1000)
            .as_deref(),
        Some("dslang=US-EN; myacinfo=abc")
    );
    assert_eq!(
        jar.header("https://developer.apple.com/download/", 1000)
            .as_deref(),
        Some("myacinfo=abc")
    );
    // An expiry in the past deletes the cookie.
    let expire = vec![(
        "set-cookie".to_string(),
        "myacinfo=; Domain=apple.com; Path=/; Max-Age=0".to_string(),
    )];
    jar.receive("https://developer.apple.com/", &expire, 2000);
    assert!(jar.iter().all(|c| c.name != "myacinfo"));
}

#[test]
fn http_dates_in_the_formats_servers_send() {
    for text in [
        "Thu, 01 Jan 2026 00:00:10 GMT",
        "Thursday, 01-Jan-26 00:00:10 GMT",
        "Thu Jan  1 00:00:10 2026",
    ] {
        assert_eq!(http_date(text), Some(1_767_225_610), "{text}");
    }
    assert_eq!(http_date("soon"), None);
}

#[test]
fn only_https_apple_urls_and_redirects_are_allowed() {
    for target in [
        "https://apple.com.evil.example/",
        "http://idmsa.apple.com/",
        "https://user@apple.com/",
        "https://apple.com:444/",
        "file:///tmp/cookies",
        "https://evil.example\\.apple.com/",
    ] {
        assert!(!url::apple(target), "{target}");
    }
    for target in [
        "https://apple.com/",
        "https://idmsa.apple.com:443/appleauth",
        "https://DEVELOPER.apple.com/download/",
    ] {
        assert!(url::apple(target), "{target}");
    }
    let (mut client, _, _) = queue(vec![reply(
        302,
        &[("Location", "https://example.com/")],
        b"",
    )]);
    let err = client
        .open(Call::get("https://developer.apple.com/download/"))
        .err()
        .unwrap();
    assert!(err.contains("redirect"), "{err}");
    let (mut client, log, _) = queue(vec![
        reply(302, &[("Location", "/account/")], b""),
        reply(200, &[], b"{}"),
    ]);
    let (code, _, _) = client
        .request(Call::new("POST", "https://developer.apple.com/x").json(json!({"a": 1})))
        .unwrap();
    assert_eq!(code, 200);
    let second = &log.borrow()[1];
    assert_eq!(second.url, "https://developer.apple.com/account/");
    assert_eq!(
        (second.method.as_str(), second.body.is_none()),
        ("GET", true)
    );
}

#[test]
fn urls_join_and_queries_decode() {
    let base = "https://developer.apple.com/services-account/download?path=x";
    assert_eq!(
        url::join(base, "https://download.developer.apple.com/a"),
        "https://download.developer.apple.com/a"
    );
    assert_eq!(url::join(base, "/b"), "https://developer.apple.com/b");
    assert_eq!(
        url::join(base, "c"),
        "https://developer.apple.com/services-account/c"
    );
    assert_eq!(
        url::join(base, "//idmsa.apple.com/d"),
        "https://idmsa.apple.com/d"
    );
    assert_eq!(
        url::query_value("a=1&widgetKey=k%2B1+2", "widgetKey").as_deref(),
        Some("k+1 2")
    );
    assert_eq!(
        url::form(&[("path", "/Developer Tools/a+b")]),
        "path=%2FDeveloper+Tools%2Fa%2Bb"
    );
    assert_eq!(
        url::encode("/Developer_Tools/Xcode 27.aea", "/"),
        "/Developer_Tools/Xcode%2027.aea"
    );
}

#[test]
fn srp_refuses_bad_public_keys_rounds_and_protocols() {
    assert_eq!(n().bits(), 2048);
    let b64 = base64::engine::general_purpose::STANDARD;
    let valid =
        json!({"salt": b64.encode(b"salt"), "b": "Ag==", "iteration": 1000, "protocol": "s2k"});
    let client = Srp::new(BigUint::from(123u32));
    assert!(
        client
            .proofs("synthetic@example.com", "password", &valid)
            .is_ok()
    );
    for (key, value) in [
        ("b", json!("AA==")),
        ("b", json!(b64.encode(n().to_bytes_be()))),
        ("iteration", json!(0)),
        ("iteration", json!(10_000_001)),
        ("protocol", json!("unknown")),
        ("b", json!("!")),
        ("salt", json!(null)),
    ] {
        let mut challenge = valid.clone();
        challenge[key] = value.clone();
        assert!(
            client
                .proofs("synthetic@example.com", "password", &challenge)
                .is_err(),
            "{key}={value}"
        );
    }
}

#[test]
#[allow(clippy::many_single_char_names)] // SRP's own names
fn srp_proofs_match_an_independent_server_for_both_password_modes() {
    // The server's S = (A * v^u)^b checks the client's subtraction and
    // exponent from the other side.
    let (username, password, salt) = (
        "synthetic@example.com",
        "not-a-real-password",
        b"fixture-salt",
    );
    let n = n();
    let g = BigUint::from(2u32);
    let private_b = BigUint::parse_bytes(
        b"E487CB59D31AC550471E81F00F6928E01DDA08E974A004F49E61F5D105284D20",
        16,
    )
    .unwrap();
    let b64 = base64::engine::general_purpose::STANDARD;
    for protocol in ["s2k", "s2k_fo"] {
        let mut digest = sha(&[password.as_bytes()]).to_vec();
        if protocol == "s2k_fo" {
            digest = crate::cli::apple_archive::hex(&digest).into_bytes();
        }
        let derived = pbkdf2(&digest, salt, 1000);
        let x = BigUint::from_bytes_be(&sha(&[salt, &sha(&[b":", &derived])]));
        let v = g.modpow(&x, &n);
        let k = BigUint::from_bytes_be(&sha(&[&pad(&n.to_bytes_be()), &pad(&[2])]));
        let b_public = (&k * &v + g.modpow(&private_b, &n)) % &n;
        let client = Srp::new(
            BigUint::parse_bytes(
                b"60975527035CF2AD1989806F0407210BC81EDC04E2762A56AFD529DDDA2D4393",
                16,
            )
            .unwrap(),
        );
        let challenge = json!({
            "salt": b64.encode(salt),
            "b": b64.encode(b_public.to_bytes_be()),
            "iteration": 1000,
            "protocol": protocol,
        });
        let (m1, m2) = client.proofs(username, password, &challenge).unwrap();
        let public_a = BigUint::from_bytes_be(&client.public);
        let u = BigUint::from_bytes_be(&sha(&[
            &pad(&public_a.to_bytes_be()),
            &pad(&b_public.to_bytes_be()),
        ]));
        let shared = (&public_a * v.modpow(&u, &n)).modpow(&private_b, &n);
        let key = sha(&[&shared.to_bytes_be()]);
        let xor: Vec<u8> = sha(&[&pad(&n.to_bytes_be())])
            .iter()
            .zip(sha(&[&pad(&[2])]))
            .map(|(a, b)| a ^ b)
            .collect();
        let expected_m1 = sha(&[
            &xor,
            &sha(&[username.as_bytes()]),
            salt,
            &client.public,
            &b_public.to_bytes_be(),
            &key,
        ]);
        assert_eq!(b64.decode(m1).unwrap(), expected_m1, "{protocol}");
        assert_eq!(
            b64.decode(m2).unwrap(),
            sha(&[&client.public, &expected_m1, &key]),
            "{protocol}"
        );
    }
}

#[test]
fn pbkdf2_matches_rfc_7914() {
    // RFC 7914 section 11, PBKDF2-HMAC-SHA256 with one iteration.
    let derived = pbkdf2(b"passwd", b"salt", 1);
    assert_eq!(
        crate::cli::apple_archive::hex(&derived),
        "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc"
    );
}

#[test]
fn hashcash_counts_in_decimal_until_the_leading_bits_are_zero() {
    use sha1::Digest as _;
    let proof = hashcash(Some("8"), Some("fixture-resource"), Some("261112230000")).unwrap();
    assert!(
        proof.starts_with("1:8:261112230000:fixture-resource::"),
        "{proof}"
    );
    assert_eq!(sha1::Sha1::digest(proof.as_bytes())[0], 0);
    assert!(hashcash(Some("30"), Some("resource"), None).is_err());
    assert!(hashcash(None, Some("resource"), None).is_err());
    assert!(hashcash(Some("1"), Some(""), None).is_err());
}

#[test]
fn sign_in_completes_srp_the_device_code_and_the_web_session() {
    let dir = TempDir::new("apple-login");
    let (client, log, pending) = queue(vec![
        reply(
            302,
            &[(
                "Location",
                "https://idmsa.apple.com/appleauth/auth/signin?widgetKey=public-key",
            )],
            b"",
        ),
        json_reply(200, &[], &json!({"federated": false})),
        json_reply(200, &[], &json!({"federated": false})),
        json_reply(
            200,
            &[
                ("X-Apple-HC-Bits", "0"),
                ("X-Apple-HC-Challenge", "fixture"),
            ],
            &json!({}),
        ),
        json_reply(
            200,
            &[],
            &json!({"salt": "c2FsdA==", "b": "Ag==", "iteration": 1, "protocol": "s2k", "c": "challenge"}),
        ),
        reply(
            409,
            &[
                ("scnt", "first"),
                ("X-Apple-ID-Session-Id", "session"),
                ("Set-Cookie", "myacinfo=signed-in; Domain=apple.com; Path=/"),
            ],
            b"",
        ),
        json_reply(
            200,
            &[("scnt", "rotated")],
            &json!({"trustedPhoneNumbers": [], "securityCode": {"length": 6}}),
        ),
        reply(204, &[], b""),
        reply(204, &[], b""),
        json_reply(200, &[], &json!({"user": {"fullName": "Synthetic User"}})),
        reply(200, &[("Content-Type", "text/html")], b"<html></html>"),
        json_reply(200, &[], &json!({"resultCode": 0, "downloads": []})),
    ]);
    let mut session = Session::new(dir.join("session.json"), client);
    let mut answers = vec!["123456", "synthetic-password"];
    let mut asked = Vec::new();
    let mut prompt = |question: &str, hidden: bool| {
        asked.push((question.to_string(), hidden));
        Ok(answers.pop().unwrap().to_string())
    };
    let mut note = |_: &str| {};
    Login::new(&mut session, &mut prompt, &mut note)
        .login("synthetic@example.com")
        .unwrap();
    assert!(pending.borrow().is_empty());
    assert_eq!(
        asked.iter().map(|(_, hidden)| *hidden).collect::<Vec<_>>(),
        [true, false]
    );
    let log = log.borrow();
    assert!(
        log[2].url.ends_with("/federate"),
        "the lookup isn't repeated"
    );
    let complete = &log[5];
    assert!(
        complete
            .url
            .ends_with("/signin/complete?isRememberMeEnabled=false")
    );
    assert!(
        complete
            .header("X-Apple-HC")
            .is_some_and(|h| h.starts_with("1:0:"))
    );
    assert_eq!(log[6].header("X-Apple-HC"), None);
    let verification = &log[7];
    assert!(
        verification
            .url
            .ends_with("/verify/trusteddevice/securitycode")
    );
    assert_eq!(verification.header("scnt"), Some("rotated"));
    let body: Value = serde_json::from_slice(verification.body.as_ref().unwrap()).unwrap();
    assert_eq!(body, json!({"securityCode": {"code": "123456"}}));
    assert_eq!(log[9].header("Cookie"), Some("myacinfo=signed-in"));
    let saved = std::fs::read_to_string(&session.path).unwrap();
    assert!(!saved.contains("synthetic-password"));
    assert!(saved.contains("signed-in"));
}

#[test]
fn the_second_factor_uses_the_automatic_text_and_refuses_security_keys() {
    for (options, expected) in [
        (
            json!({"noTrustedDevices": true, "trustedPhoneNumbers": [{"id": 9}]}),
            "/verify/phone/securitycode",
        ),
        (
            json!({"trustedPhoneNumbers": []}),
            "/verify/trusteddevice/securitycode",
        ),
    ] {
        let (client, log, _) = queue(vec![
            json_reply(200, &[], &options),
            reply(204, &[], b""),
            reply(204, &[], b""),
        ]);
        let mut session = Session::new("unused".into(), client);
        let mut prompt = |_: &str, _: bool| Ok("123 456".to_string());
        let mut note = |_: &str| {};
        let mut login = Login::new(&mut session, &mut prompt, &mut note);
        login.headers = vec![
            ("scnt".into(), "s".into()),
            ("X-Apple-ID-Session-Id".into(), "id".into()),
        ];
        login.second_factor().unwrap();
        let sent = &log.borrow()[1];
        assert!(sent.url.ends_with(expected), "{}", sent.url);
        if expected.contains("phone") {
            let body: Value = serde_json::from_slice(sent.body.as_ref().unwrap()).unwrap();
            assert_eq!(body["phoneNumber"], json!({"id": 9}));
        }
    }
    let (client, _, _) = queue(vec![json_reply(
        200,
        &[],
        &json!({"fsaChallenge": {"challenge": "x"}}),
    )]);
    let mut session = Session::new("unused".into(), client);
    let mut prompt = no_prompt();
    let mut note = |_: &str| {};
    let mut login = Login::new(&mut session, &mut prompt, &mut note);
    login.headers = vec![
        ("scnt".into(), "s".into()),
        ("X-Apple-ID-Session-Id".into(), "id".into()),
    ];
    let err = login.second_factor().unwrap_err();
    assert!(err.contains("security keys"), "{err}");
}

#[test]
fn a_failed_sign_in_leaves_the_saved_session_alone() {
    let dir = TempDir::new("apple-login-failed");
    let path = dir.join("session.json");
    std::fs::write(&path, "existing-session").unwrap();
    let (client, _, _) = queue(vec![
        reply(
            302,
            &[("Location", "https://idmsa.apple.com/x?widgetKey=key")],
            b"",
        ),
        json_reply(200, &[], &json!({"federated": true})),
    ]);
    let mut session = Session::new(path.clone(), client);
    let mut prompt = no_prompt();
    let mut note = |_: &str| {};
    let err = Login::new(&mut session, &mut prompt, &mut note)
        .login("synthetic@example.com")
        .unwrap_err();
    assert!(err.contains("federated"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing-session");
}

#[test]
fn an_expired_session_says_so_without_the_response_body() {
    let (client, _, _) = queue(vec![json_reply(
        401,
        &[],
        &json!({"secret": "never-log-this"}),
    )]);
    let err = Session::new("unused".into(), client).catalog().unwrap_err();
    assert!(err.contains("session is unavailable"), "{err}");
    assert!(!err.contains("never-log-this"), "{err}");
}

#[test]
fn non_interactive_sign_in_fails_without_a_session_and_logout_reports() {
    let dir = TempDir::new("apple-noninteractive");
    let path = dir.join("session.json");
    let (client, log, _) = queue(Vec::new());
    let mut session = Session::new(path.clone(), client);
    let mut prompt = no_prompt();
    let err = login_with(
        &mut session,
        "synthetic@example.com",
        false,
        &mut prompt,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(err.contains("terminal"), "{err}");
    assert!(log.borrow().is_empty());
    assert!(!path.exists());
    assert!(login_with(&mut session, "not an email", true, &mut prompt, &mut |_| {}).is_err());
    assert_eq!(logout(&path).unwrap()["loggedOut"], json!(true));
    std::fs::write(&path, "{}").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    logout(&path).unwrap();
    assert!(!path.exists());
}

#[test]
fn a_saved_session_for_the_same_account_is_reused() {
    let dir = TempDir::new("apple-reuse");
    let path = dir.join("session.json");
    let mut saved = Session::new(path.clone(), queue(Vec::new()).0);
    saved.apple_id = Some("Synthetic@Example.com".into());
    saved.save().unwrap();
    let (client, _, _) = queue(vec![json_reply(
        200,
        &[],
        &json!({"resultCode": 0, "downloads": []}),
    )]);
    let mut session = Session::new(path, client);
    let result = login_with(
        &mut session,
        "synthetic@example.com",
        false,
        &mut no_prompt(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(result["reused"], json!(true));
}

const PATH: &str = "/Developer_Tools/Xcode_27.3_beta/Xcode_27.3_beta.aea";

fn payload() -> Vec<u8> {
    let mut data = b"AEA1".to_vec();
    for _ in 0..16 {
        data.extend(0..=255u8);
    }
    data
}

fn release(size: usize) -> Release {
    Release {
        name: "Xcode 27.3 beta".into(),
        path: PATH.into(),
        size: Some(size as u64),
    }
}

#[test]
fn a_beta_never_stands_in_for_a_release() {
    let catalog =
        [json!({"name": "Xcode 27.3 beta", "files": [{"remotePath": PATH, "fileSize": "4100"}]})];
    let chosen = select_release(&catalog, "27.3-beta1").unwrap();
    assert_eq!((chosen.path.as_str(), chosen.size), (PATH, Some(4100)));
    let err = select_release(&catalog, "27.3").unwrap_err();
    assert!(err.contains("isn't in Apple's downloads"), "{err}");
    for (input, expected) in [
        ("27.2.0", "27.2"),
        ("Xcode 27.3 Beta 2", "27.3beta2"),
        ("xcode_27.3-release candidate", "27.3rc"),
        ("27", "27"),
    ] {
        assert_eq!(normalize_version(input).unwrap(), expected, "{input}");
    }
    for input in ["", "latest", "27.3.1.4", "27..3", "27.3 gm"] {
        assert!(normalize_version(input).is_err(), "{input}");
    }
    let both = [json!({"name": "Xcode 27.2", "files": [
        {"remotePath": "Developer_Tools/Xcode_27.2/Xcode_27.2.xip"},
        {"remotePath": "Developer_Tools/Xcode_27.2/Xcode_27.2.aea"},
    ]})];
    assert!(
        select_release(&both, "27.2")
            .unwrap()
            .path
            .ends_with("Xcode_27.2.aea")
    );
}

#[test]
fn prerelease_links_take_apple_xcode_archives_only() {
    let html = r#"<a class="button direct-download" href="/services-account/download?path=/Developer_Tools/Xcode_27.3_beta/Xcode_27.3_beta.aea&amp;x=1">Xcode</a>
        <a class="direct-download" href="https://example.com/Developer_Tools/Xcode_27.3_beta/x.aea">bad</a>
        <a href="/services-account/download?path=/Developer_Tools/Xcode_27.4/Xcode_27.4.aea">no class</a>"#;
    let links = prerelease_links(html).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(select_release(&links, "27.3 beta").unwrap().path, PATH);
}

#[test]
fn catalog_paths_stay_under_developer_tools() {
    for path in [
        "https://example.com/file.aea",
        "/Developer_Tools/../file.aea",
        "/Developer_Tools/%2e%2e/file.aea",
        "/Developer_Tools/file.aea?secret=token",
        "/Developer_Tools/file.zip",
    ] {
        assert!(archive_path(path).is_err(), "{path}");
    }
    assert_eq!(
        archive_path("/Developer_Tools/Xcode%2027/X.xip").unwrap(),
        "/Developer_Tools/Xcode 27/X.xip"
    );
}

#[derive(Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools)] // independent server misbehaviors
struct Flags {
    truncate: bool,
    ignore_range: bool,
    text: bool,
    bad_range: bool,
    redirect: bool,
}

/// A client whose download server serves `payload` the way `flags` say.
fn server(payload: Vec<u8>, flags: Flags) -> (Client, Log) {
    fake(move |sent| {
        if sent.url.contains("/services-account/download") {
            return reply(200, &[], b"");
        }
        if flags.redirect {
            return reply(302, &[("Location", "https://example.com/stolen")], b"");
        }
        let offset = sent
            .header("Range")
            .filter(|_| !flags.ignore_range)
            .map_or(0, |r| {
                r.trim_start_matches("bytes=")
                    .trim_end_matches('-')
                    .parse()
                    .unwrap()
            });
        let len = payload.len();
        let length = (len - offset).to_string();
        let range = format!(
            "bytes {}-{}/{len}",
            if flags.bad_range { 0 } else { offset },
            len - 1
        );
        let mut headers = vec![
            ("Content-Length", length.as_str()),
            (
                "Content-Type",
                if flags.text {
                    "text/html"
                } else {
                    "application/octet-stream"
                },
            ),
            ("ETag", "\"synthetic-version-1\""),
        ];
        if offset > 0 {
            headers.push(("Content-Range", &range));
        }
        let end = if flags.truncate { len / 2 } else { len };
        reply(
            if offset > 0 { 206 } else { 200 },
            &headers,
            &payload[offset..end],
        )
    })
}

fn fetch(
    client: Client,
    dir: &Path,
    release: &Release,
    destination: &Path,
) -> Result<Value, String> {
    let mut session = Session::new(dir.join("session.json"), client);
    download::download(
        &mut session,
        release,
        destination,
        Options {
            persist_session: false,
            free_space: &|_| Ok(u64::MAX),
            note: &mut |_| {},
        },
    )
}

#[test]
fn a_finished_download_has_a_receipt_and_is_reused_only_when_it_matches() {
    let dir = TempDir::new("xcode-download");
    let path = dir.join("Xcode.aea");
    let data = payload();
    let (client, log) = server(data.clone(), Flags::default());
    let mut session = Session::new(dir.join("session.json"), client);
    let run = |session: &mut Session| {
        download::download(
            session,
            &release(data.len()),
            &path,
            Options {
                persist_session: false,
                free_space: &|_| Ok(u64::MAX),
                note: &mut |_| {},
            },
        )
    };
    let first = run(&mut session).unwrap();
    assert_eq!(first["reused"], json!(false));
    assert_eq!(first["xcodeVersion"], json!("27.3 beta"));
    assert_eq!(std::fs::read(&path).unwrap(), data);
    let count = log.borrow().len();
    assert_eq!(run(&mut session).unwrap()["reused"], json!(true));
    assert_eq!(log.borrow().len(), count);
    std::fs::write(&path, b"AEA1corrupt").unwrap();
    let err = run(&mut session).unwrap_err();
    assert!(err.contains("receipt"), "{err}");
}

#[test]
fn an_interrupted_download_resumes_with_a_checked_range() {
    let dir = TempDir::new("xcode-resume");
    let path = dir.join("Xcode.aea");
    let data = payload();
    let (client, _) = server(
        data.clone(),
        Flags {
            truncate: true,
            ..Flags::default()
        },
    );
    let err = fetch(client, &dir, &release(data.len()), &path).unwrap_err();
    assert!(err.contains("incomplete"), "{err}");
    assert!(!path.exists());
    let offset = std::fs::metadata(dir.join("Xcode.aea.part")).unwrap().len();
    assert!(offset > 0);
    let (client, log) = server(data.clone(), Flags::default());
    let result = fetch(client, &dir, &release(data.len()), &path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), data);
    let log = log.borrow();
    let last = log.last().unwrap();
    assert_eq!(
        last.header("Range"),
        Some(format!("bytes={offset}-").as_str())
    );
    assert_eq!(last.header("If-Range"), Some("\"synthetic-version-1\""));
    assert_eq!(result["archiveBytes"], json!(data.len()));
    assert!(!dir.join("Xcode.aea.part.json").exists());
}

fn partial(dir: &Path, data: &[u8], state: &Value) {
    let part = dir.join("Xcode.aea.part");
    std::fs::write(&part, data).unwrap();
    std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o600)).unwrap();
    private_write(&dir.join("Xcode.aea.part.json"), state).unwrap();
}

#[test]
fn a_server_that_ignores_the_range_restarts_the_file() {
    let dir = TempDir::new("xcode-norange");
    let data = payload();
    partial(
        &dir,
        &data[..100],
        &json!({"sourcePath": PATH, "validator": "\"old\""}),
    );
    let (client, _) = server(
        data.clone(),
        Flags {
            ignore_range: true,
            ..Flags::default()
        },
    );
    fetch(client, &dir, &release(data.len()), &dir.join("Xcode.aea")).unwrap();
    assert_eq!(std::fs::read(dir.join("Xcode.aea")).unwrap(), data);
}

#[test]
fn a_complete_partial_file_is_fetched_again_without_a_range_past_the_end() {
    let dir = TempDir::new("xcode-eof");
    let data = payload();
    partial(
        &dir,
        &data,
        &json!({"sourcePath": PATH, "validator": "\"synthetic-version-1\"", "expectedTotal": data.len()}),
    );
    let (client, log) = server(data.clone(), Flags::default());
    fetch(client, &dir, &release(data.len()), &dir.join("Xcode.aea")).unwrap();
    assert_eq!(log.borrow().last().unwrap().header("Range"), None);
    assert_eq!(std::fs::read(dir.join("Xcode.aea")).unwrap(), data);
}

#[test]
fn a_xip_downloads_and_a_named_session_file_is_left_alone() {
    let dir = TempDir::new("xcode-xip");
    let mut data = payload();
    data[..4].copy_from_slice(b"xar!");
    let release = Release {
        name: "Xcode 27.2".into(),
        path: "/Developer_Tools/Xcode_27.2/Xcode_27.2.xip".into(),
        size: Some(data.len() as u64),
    };
    let (client, _) = server(data, Flags::default());
    let result = fetch(client, &dir, &release, &dir.join("Xcode.xip")).unwrap();
    assert_eq!(result["xcodeVersion"], json!("27.2"));
    assert!(!dir.join("session.json").exists());
}

#[test]
fn size_and_space_problems_stop_before_any_archive_data_is_written() {
    let dir = TempDir::new("xcode-space");
    let path = dir.join("Xcode.aea");
    let data = payload();
    let (client, _) = server(data.clone(), Flags::default());
    let err = fetch(client, &dir, &release(data.len() + 1), &path).unwrap_err();
    assert!(err.contains("length differs"), "{err}");
    let (client, _) = server(data.clone(), Flags::default());
    let mut session = Session::new(dir.join("session.json"), client);
    let err = download::download(
        &mut session,
        &release(data.len()),
        &path,
        Options {
            persist_session: false,
            free_space: &|_| Ok(1),
            note: &mut |_| {},
        },
    )
    .unwrap_err();
    assert!(err.contains("free space"), "{err}");
    assert!(!dir.join("Xcode.aea.part").exists());
}

#[test]
fn html_wrong_magic_and_outside_redirects_are_refused() {
    let data = payload();
    let mut html = data.clone();
    html[..4].copy_from_slice(b"HTML");
    for (flags, body, expected) in [
        (
            Flags {
                text: true,
                ..Flags::default()
            },
            data.clone(),
            "sign-in page",
        ),
        (Flags::default(), html, "format"),
        (
            Flags {
                redirect: true,
                ..Flags::default()
            },
            data.clone(),
            "redirect",
        ),
    ] {
        let dir = TempDir::new("xcode-refused");
        let path = dir.join("Xcode.aea");
        let (client, _) = server(body, flags);
        let err = fetch(client, &dir, &release(data.len()), &path).unwrap_err();
        assert!(err.contains(expected), "{err}");
        assert!(!path.exists());
    }
}

#[test]
fn an_invalid_range_keeps_the_partial_file() {
    let dir = TempDir::new("xcode-badrange");
    let data = payload();
    partial(
        &dir,
        &data[..100],
        &json!({"sourcePath": PATH, "validator": "\"old\""}),
    );
    let (client, _) = server(
        data.clone(),
        Flags {
            bad_range: true,
            ..Flags::default()
        },
    );
    let err = fetch(client, &dir, &release(data.len()), &dir.join("Xcode.aea")).unwrap_err();
    assert!(err.contains("range"), "{err}");
    assert_eq!(
        std::fs::read(dir.join("Xcode.aea.part")).unwrap(),
        &data[..100]
    );
}

#[test]
fn a_symlinked_destination_is_never_written_through() {
    let dir = TempDir::new("xcode-symlink");
    let original = dir.join("original");
    std::fs::write(&original, "preserve").unwrap();
    let path = dir.join("Xcode.aea");
    std::os::unix::fs::symlink(&original, &path).unwrap();
    let data = payload();
    let (client, _) = server(data.clone(), Flags::default());
    assert!(fetch(client, &dir, &release(data.len()), &path).is_err());
    assert_eq!(std::fs::read_to_string(&original).unwrap(), "preserve");
}

#[test]
fn gateway_errors_and_a_refused_srp_setup_are_retried_but_a_post_is_not() {
    let key = || {
        reply(
            302,
            &[("Location", "https://idmsa.apple.com/x?widgetKey=key")],
            b"",
        )
    };
    let challenge = || {
        json_reply(
            200,
            &[
                ("X-Apple-HC-Bits", "0"),
                ("X-Apple-HC-Challenge", "fixture"),
            ],
            &json!({}),
        )
    };
    let refused = || {
        json_reply(
            400,
            &[],
            &json!({"iteration": 0, "serviceErrors": [{"code": "-900007"}]}),
        )
    };
    let (client, log, pending) = queue(vec![
        key(),
        json_reply(
            200,
            &[
                ("scnt", "first"),
                ("Set-Cookie", "aasp=first; Domain=apple.com; Path=/"),
            ],
            &json!({"federated": false}),
        ),
        json_reply(200, &[], &json!({"federated": false})),
        reply(504, &[], b""),
        reply(502, &[], b""),
        challenge(),
        refused(),
        json_reply(200, &[], &json!({"federated": false})),
        challenge(),
        refused(),
    ]);
    let mut session = Session::new("unused".into(), client);
    let mut prompt = |_: &str, _: bool| Ok("synthetic-password".to_string());
    let mut note = |_: &str| {};
    let mut login = Login::new(&mut session, &mut prompt, &mut note);
    login.retry_pause = std::time::Duration::ZERO;
    let err = login.login("synthetic@example.com").unwrap_err();
    assert!(
        err.contains("SRP setup failed (HTTP 400, -900007)"),
        "{err}"
    );
    assert!(pending.borrow().is_empty());
    let log = log.borrow();
    let challenges = log
        .iter()
        .filter(|sent| sent.url.contains("/signin?"))
        .count();
    assert_eq!(challenges, 4);
    let restart = &log[7];
    assert!(restart.url.ends_with("/federate"));
    assert_eq!(restart.header("scnt"), None);
    assert_eq!(restart.header("Cookie"), None);
    assert!(restart.header("X-Apple-Widget-Key").is_some());
    drop(log);

    let (client, log, _) = queue(vec![key(), reply(504, &[], b"")]);
    let mut session = Session::new("unused".into(), client);
    let mut prompt = no_prompt();
    let mut note = |_: &str| {};
    let mut login = Login::new(&mut session, &mut prompt, &mut note);
    login.retry_pause = std::time::Duration::ZERO;
    let err = login.login("synthetic@example.com").unwrap_err();
    assert!(err.contains("HTTP 504"), "{err}");
    assert_eq!(log.borrow().len(), 2);
}

#[test]
fn a_refused_password_step_names_apples_code_and_reason() {
    let (client, _, _) = queue(vec![
        reply(
            302,
            &[("Location", "https://idmsa.apple.com/x?widgetKey=key")],
            b"",
        ),
        json_reply(200, &[], &json!({"federated": false})),
        json_reply(200, &[], &json!({"federated": false})),
        json_reply(
            200,
            &[
                ("X-Apple-HC-Bits", "0"),
                ("X-Apple-HC-Challenge", "fixture"),
            ],
            &json!({}),
        ),
        json_reply(
            200,
            &[],
            &json!({"salt": "c2FsdA==", "b": "Ag==", "iteration": 1, "protocol": "s2k", "c": "challenge"}),
        ),
        json_reply(
            403,
            &[],
            &json!({"serviceErrors": [{"code": "-20209", "message": "This Apple Account\nhas been locked."}]}),
        ),
    ]);
    let mut session = Session::new("unused".into(), client);
    let mut prompt = |_: &str, _: bool| Ok("synthetic-password".to_string());
    let mut note = |_: &str| {};
    let err = Login::new(&mut session, &mut prompt, &mut note)
        .login("synthetic@example.com")
        .unwrap_err();
    assert_eq!(
        err,
        "Apple blocked the sign-in (HTTP 403, -20209: This Apple Account has been locked). \
         Review the account at account.apple.com before trying again."
    );
}
