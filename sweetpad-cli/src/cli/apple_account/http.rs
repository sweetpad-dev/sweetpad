//! Requests to Apple: every URL and redirect checked against
//! [`url::apple`], cookies kept per hop, and JSON bodies read with a limit.

use std::io::Read;

use serde_json::Value;

use super::cookies::Jar;
use super::url;

/// One request as it goes on the wire.
#[derive(Clone, Debug)]
pub(crate) struct Sent {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<Vec<u8>>,
}

#[cfg(test)]
impl Sent {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        header(&self.headers, name)
    }
}

/// A response whose body has not been read yet.
pub(crate) struct Exchange {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Box<dyn Read>,
}

impl Exchange {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        header(&self.headers, name)
    }
}

/// Sends one request without following redirects. The network in use,
/// a canned exchange in the tests.
pub(crate) trait Transport {
    fn send(&mut self, request: &Sent, timeout_secs: u64) -> Result<Exchange, String>;
}

/// The network, through the environment's proxy settings.
pub(crate) struct Network(pub(crate) crate::cli::net::Session);

impl Transport for Network {
    fn send(&mut self, request: &Sent, timeout_secs: u64) -> Result<Exchange, String> {
        let method = match request.method.as_str() {
            "GET" => minreq::Method::Get,
            "HEAD" => minreq::Method::Head,
            "POST" => minreq::Method::Post,
            "PUT" => minreq::Method::Put,
            other => minreq::Method::Custom(other.to_string()),
        };
        let mut call = crate::cli::net::Request::new(method, request.url.as_str())
            .timeout(timeout_secs)
            .without_redirects();
        for (name, value) in &request.headers {
            call = call.header(name.as_str(), value.as_str());
        }
        if let Some(body) = &request.body {
            call = call.body(body.clone());
        }
        let response = self
            .0
            .stream(&call)
            .map_err(|error| format!("Couldn't connect to Apple: {error}."))?;
        Ok(Exchange {
            status: response.status,
            headers: response.headers,
            body: Box::new(response.body),
        })
    }
}

/// What to send.
pub(crate) struct Call {
    pub(crate) method: &'static str,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) json: Option<Value>,
    pub(crate) follow: bool,
    /// Whether the jar's cookies go with it and its cookies are kept.
    pub(crate) cookies: bool,
    pub(crate) timeout_secs: u64,
}

impl Call {
    pub(crate) fn new(method: &'static str, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: Vec::new(),
            json: None,
            follow: true,
            cookies: true,
            timeout_secs: 60,
        }
    }

    pub(crate) fn get(url: impl Into<String>) -> Self {
        Self::new("GET", url)
    }

    pub(crate) fn header(mut self, name: &str, value: &str) -> Self {
        put(&mut self.headers, name, value);
        self
    }

    pub(crate) fn headers(mut self, headers: &[(String, String)]) -> Self {
        for (name, value) in headers {
            put(&mut self.headers, name, value);
        }
        self
    }

    pub(crate) fn json(mut self, body: Value) -> Self {
        self.json = Some(body);
        self
    }

    pub(crate) fn no_follow(mut self) -> Self {
        self.follow = false;
        self
    }

    pub(crate) fn without_cookies(mut self) -> Self {
        self.cookies = false;
        self
    }

    pub(crate) fn timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }
}

/// A response read whole: status, headers, and the body as JSON (`None`
/// when it is not JSON, an empty object when it is empty).
pub(crate) type Reply = (u16, Vec<(String, String)>, Option<Value>);

pub(crate) struct Client {
    transport: Box<dyn Transport>,
    pub(crate) cookies: Jar,
}

impl Client {
    pub(crate) fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            cookies: Jar::default(),
        }
    }

    pub(crate) fn network() -> Self {
        Self::new(Box::new(Network(crate::cli::net::Session::from_env())))
    }

    /// Send `call`, following redirects to Apple hosts when it asks to.
    pub(crate) fn open(&mut self, call: Call) -> Result<Exchange, String> {
        let Call {
            method,
            url: start,
            headers: extra,
            json,
            follow,
            cookies,
            timeout_secs,
        } = call;
        if !url::apple(&start) {
            return Err("Only HTTPS Apple service URLs are supported.".into());
        }
        let mut method = method;
        let mut target = start;
        let mut body = json.map(|json| json.to_string().into_bytes());
        for _ in 0..10 {
            let mut headers = vec![
                (
                    "User-Agent".to_string(),
                    "Sweetpad/1 (Linux; Apple Developer downloads)".to_string(),
                ),
                ("Accept".to_string(), "application/json".to_string()),
                ("Accept-Encoding".to_string(), "gzip".to_string()),
            ];
            for (name, value) in &extra {
                put(&mut headers, name, value);
            }
            if body.is_some() {
                put(&mut headers, "Content-Type", "application/json");
            }
            let now = now();
            if cookies && let Some(cookie) = self.cookies.header(&target, now) {
                put(&mut headers, "Cookie", &cookie);
            }
            let sent = Sent {
                method: method.to_string(),
                url: target.clone(),
                headers,
                body: body.clone(),
            };
            let response = self.transport.send(&sent, timeout_secs)?;
            if cookies {
                self.cookies.receive(&target, &response.headers, now);
            }
            if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                return Ok(response);
            }
            let Some(location) = response.header("location") else {
                return Ok(response);
            };
            let next = url::join(&target, location);
            if !url::apple(&next) {
                return Err("Apple returned an unsupported redirect destination.".into());
            }
            // A 307 or 308 repeats the method, so a POST stops there; the
            // others continue as a GET without the body.
            let keeps_method = matches!(response.status, 307 | 308);
            if !follow || (keeps_method && !matches!(method, "GET" | "HEAD")) {
                return Ok(response);
            }
            method = "GET";
            body = None;
            target = next;
        }
        Err("Apple redirected the request too many times.".into())
    }

    /// Send `call` and read the response as JSON.
    pub(crate) fn request(&mut self, call: Call) -> Result<Reply, String> {
        let head = call.method == "HEAD";
        let mut response = self.open(call)?;
        let raw = if head || matches!(response.status, 100..=199 | 204 | 304) {
            Vec::new()
        } else {
            read_body(&response.headers, &mut response.body, 64 * 1024 * 1024)?
        };
        let data = if raw.is_empty() {
            Some(Value::Object(serde_json::Map::new()))
        } else {
            serde_json::from_slice(&raw).ok()
        };
        Ok((response.status, response.headers, data))
    }
}

/// A body up to `limit` bytes, gzip decoded when the response says so.
pub(crate) fn read_body(
    headers: &[(String, String)],
    body: &mut dyn Read,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let encoding = header(headers, "content-encoding")
        .unwrap_or("identity")
        .trim()
        .to_ascii_lowercase();
    let mut source: Box<dyn Read + '_> = match encoding.as_str() {
        "identity" | "" => Box::new(body),
        "gzip" => Box::new(flate2::read::GzDecoder::new(body)),
        _ => return Err("Apple returned a response encoding sweetpad can't read.".into()),
    };
    let mut data = Vec::new();
    source
        .by_ref()
        .take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|_| "Apple's response was cut off or malformed. Run the command again.")?;
    if data.len() as u64 > limit {
        return Err("Apple's response is larger than sweetpad accepts.".into());
    }
    Ok(data)
}

pub(crate) fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Set `name` to `value`, replacing it whatever its case.
pub(crate) fn put(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
    headers.push((name.to_string(), value.to_string()));
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
