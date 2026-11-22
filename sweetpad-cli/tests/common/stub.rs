//! A stub HTTP server on 127.0.0.1 standing in for Apple's services, which
//! the CLI reaches through the hidden `SWEETPAD_ASC_BASE_URL`, so no test can
//! reach Apple. It answers each request with what the test's closure returns
//! and keeps every request for the test to check.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use p256::ecdsa::SigningKey;
use p256::pkcs8::{EncodePrivateKey, LineEnding};
use serde_json::Value;

/// The key ID of the key [`write_key`] writes.
pub const KEY_ID: &str = "TESTKEY123";

/// One request the stub received.
#[derive(Clone, Debug)]
pub struct Received {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Received {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

pub struct Stub {
    pub base: String,
    received: Arc<Mutex<Vec<Received>>>,
}

impl Stub {
    /// Serve `answer(request, base)`, a status line and a JSON body.
    pub fn start(
        mut answer: impl FnMut(&Received, &str) -> (&'static str, String) + Send + 'static,
    ) -> Self {
        Self::start_with_headers(move |request, base| {
            let (status, body) = answer(request, base);
            (status, Vec::new(), body)
        })
    }

    /// Serve `answer(request, base)`, a status line, more headers and a
    /// body.
    #[allow(dead_code)]
    pub fn start_with_headers(
        mut answer: impl FnMut(&Received, &str) -> (&'static str, Vec<(String, String)>, String)
        + Send
        + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&received);
        let url = base.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let request = read_request(&mut stream);
                log.lock().unwrap().push(request.clone());
                let (status, headers, body) = answer(&request, &url);
                let headers = headers
                    .iter()
                    .fold(String::new(), |mut out, (name, value)| {
                        let _ = write!(out, "{name}: {value}\r\n");
                        out
                    });
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
            }
        });
        Self { base, received }
    }

    pub fn requests(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }

    /// The CLI in `home`, which is also its HOME, signed with the team key
    /// [`write_key`] wrote and pointed at this stub.
    pub fn command(&self, home: &Path) -> Command {
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
            .current_dir(home)
            .env("HOME", home)
            .env("XDG_STATE_HOME", home)
            .env("XDG_CONFIG_HOME", home)
            .env("XDG_CACHE_HOME", home)
            .env("TMPDIR", home)
            .env("SWEETPAD_ASC_BASE_URL", &self.base)
            .env("SWEETPAD_ASC_POLL_SECS", "0")
            .env("SWEETPAD_ASC_KEY_ID", KEY_ID)
            .env(
                "SWEETPAD_ASC_ISSUER_ID",
                "57246542-96fe-1a63-e053-0824d011072a",
            );
        command
    }
}

/// Write an API key where altool keeps them under `home`.
pub fn write_key(home: &Path) {
    let key = SigningKey::from_slice(&[9u8; 32]).unwrap();
    let keys = home.join(".appstoreconnect/private_keys");
    std::fs::create_dir_all(&keys).unwrap();
    std::fs::write(
        keys.join(format!("AuthKey_{KEY_ID}.p8")),
        key.to_pkcs8_pem(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
}

/// The data of a successful JSON envelope on stdout.
pub fn data(out: &Output) -> Value {
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

/// The error from the one-shot JSON result.
pub fn error(out: &Output) -> Value {
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["ok"], false, "{value}");
    value["error"].clone()
}

fn read_request(stream: &mut TcpStream) -> Received {
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
