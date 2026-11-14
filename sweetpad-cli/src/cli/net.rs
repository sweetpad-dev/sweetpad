//! HTTP for the commands that talk to a server: feedback reports, App Store
//! Connect, the notary service and timestamp authorities.
//!
//! A [`Session`] reads the proxy variables once, the way curl reads them, and
//! hands each request the proxy it should use. minreq applies `https_proxy`
//! (or `http_proxy` for plain HTTP) on its own when a request names none and
//! reads no `no_proxy`, so a session that has to go direct past a set proxy
//! variable removes the variables first.

use std::time::Duration;

use crate::cli::CliError;

/// The HTTP proxy a request goes through.
#[derive(Clone)]
pub(crate) struct Proxy {
    inner: minreq::Proxy,
    /// The variable it came from, for the messages.
    var: &'static str,
    /// `host[:port]`, without the credentials.
    pub(crate) shown: String,
    pub(crate) has_credentials: bool,
}

impl Proxy {
    pub(crate) fn parse(var: &'static str, value: &str) -> Result<Self, CliError> {
        let value = value.trim().trim_end_matches('/');
        let authority = match value.split_once("://") {
            Some(("http", rest)) => Some(rest),
            Some(_) => None,
            None => Some(value),
        };
        let parsed = authority.and_then(|authority| {
            let proxy = minreq::Proxy::new(value).ok()?;
            let (credentials, shown) = match authority.rsplit_once('@') {
                Some((_, shown)) => (true, shown),
                None => (false, authority),
            };
            Some(Self {
                inner: proxy,
                var,
                shown: shown.to_string(),
                has_credentials: credentials,
            })
        });
        // The value stays out of the message: it can hold a password.
        parsed.ok_or_else(|| {
            CliError::new(format!(
                "{var} names a proxy sweetpad can't use, so nothing was sent. It takes an HTTP \
                 proxy, written 'http://host:port' or 'http://user:password@host:port'"
            ))
        })
    }
}

/// The proxy settings of the environment, read once.
#[derive(Clone, Default)]
pub(crate) struct Session {
    https: Option<(&'static str, String)>,
    http: Option<(&'static str, String)>,
    no_proxy: Option<String>,
}

impl Session {
    /// `https_proxy` or `HTTPS_PROXY` for HTTPS and `http_proxy` for plain
    /// HTTP (curl reads no upper-case `HTTP_PROXY`), with `no_proxy` or
    /// `NO_PROXY` exempting hosts.
    ///
    /// When a `no_proxy` list is set beside a proxy, the proxy variables are
    /// removed from this process so minreq can't apply them to an exempt
    /// host. Call it on the main thread before any thread that reads the
    /// environment starts, and after the last child process that needs them.
    pub(crate) fn from_env() -> Self {
        let session = Self {
            https: env_first(&["https_proxy", "HTTPS_PROXY"]),
            http: env_first(&["http_proxy"]),
            no_proxy: env_first(&["no_proxy", "NO_PROXY"]).map(|(_, value)| value),
        };
        if session.no_proxy.is_some() && (session.https.is_some() || session.http.is_some()) {
            for key in ["https_proxy", "HTTPS_PROXY", "http_proxy", "all_proxy", "ALL_PROXY"] {
                // Safety: see the doc comment; callers construct the session
                // before starting threads.
                unsafe { std::env::remove_var(key) };
            }
        }
        session
    }

    /// The proxy for `url`, or `None` to go direct.
    pub(crate) fn proxy(&self, url: &str) -> Result<Option<Proxy>, CliError> {
        let chosen = if url.starts_with("https://") {
            &self.https
        } else {
            &self.http
        };
        let Some((var, value)) = chosen else {
            return Ok(None);
        };
        if let Some(no_proxy) = &self.no_proxy
            && exempts(no_proxy, host(url))
        {
            return Ok(None);
        }
        Proxy::parse(var, value).map(Some)
    }

    /// Send `request` once.
    pub(crate) fn send(&self, request: &Request) -> Result<Response, CliError> {
        self.attempt(request).map_err(|failure| failure.error)
    }

    fn attempt(&self, request: &Request) -> Result<Response, SendFailure> {
        let proxy = self.proxy(&request.url).map_err(|error| SendFailure { error, transport: false })?;
        let mut call = minreq::Request::new(request.method.clone(), request.url.as_str())
            .with_timeout(request.timeout_secs);
        for (name, value) in &request.headers {
            call = call.with_header(name.as_str(), value.as_str());
        }
        if let Some(body) = &request.body {
            call = call.with_body(body.clone());
        }
        if let Some(proxy) = &proxy {
            call = call.with_proxy(proxy.inner.clone());
        }
        let response = call
            .send()
            .map_err(|e| send_error(&e, host(&request.url), proxy.as_ref(), request.timeout_secs))?;
        Ok(Response {
            status: response.status_code,
            reason: response.reason_phrase.clone(),
            headers: response.headers.clone(),
            body: response.into_bytes(),
        })
    }

    /// Send `request`, retrying a dropped connection, a timeout, a 5xx and a
    /// 429 up to `attempts` times with a growing pause (a 429's
    /// `Retry-After`, when it gives one). Only for requests that are safe to
    /// repeat.
    pub(crate) fn send_retrying(&self, request: &Request, attempts: u32) -> Result<Response, CliError> {
        let mut pause = Duration::from_secs(1);
        for attempt in 1..=attempts {
            let last = attempt == attempts;
            match self.attempt(request) {
                Ok(response) if !last && (response.status == 429 || response.status >= 500) => {
                    let wait = response
                        .header("retry-after")
                        .and_then(|v| v.trim().parse::<u64>().ok())
                        .map_or(pause, |s| Duration::from_secs(s.min(60)));
                    std::thread::sleep(wait);
                }
                Err(failure) if !last && failure.transport => std::thread::sleep(pause),
                Ok(response) => return Ok(response),
                Err(failure) => return Err(failure.error),
            }
            pause = (pause * 2).min(Duration::from_secs(30));
        }
        unreachable!("the last attempt returns")
    }
}

/// A failed attempt, and whether it failed in transit (worth repeating)
/// rather than being refused.
struct SendFailure {
    error: CliError,
    transport: bool,
}

/// One HTTP request.
#[derive(Clone)]
pub(crate) struct Request {
    pub(crate) method: minreq::Method,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<Vec<u8>>,
    pub(crate) timeout_secs: u64,
}

impl Request {
    pub(crate) fn new(method: minreq::Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout_secs: 60,
        }
    }

    pub(crate) fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub(crate) fn body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }

    pub(crate) fn timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }
}

/// A response, read whole.
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) reason: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Response {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub(crate) fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The first 200 characters of the body, for an error message.
    pub(crate) fn snippet(&self) -> String {
        String::from_utf8_lossy(&self.body)
            .trim()
            .chars()
            .take(200)
            .collect()
    }
}

/// `host[:port]` of a URL.
pub(crate) fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?']).next().unwrap_or(rest)
}

/// The first of `vars` that is set and not empty.
fn env_first(vars: &[&'static str]) -> Option<(&'static str, String)> {
    vars.iter().copied().find_map(|var| {
        std::env::var(var)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| (var, value))
    })
}

/// Whether a `no_proxy` list exempts `host` (`host[:port]`): `*` exempts
/// every host, and an entry exempts the host it names and the hosts under
/// it, with or without a leading dot (`example.com` and `.example.com` both
/// cover `api.example.com`).
pub(crate) fn exempts(no_proxy: &str, host: &str) -> bool {
    let host = host
        .rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
        .map_or(host, |(host, _)| host)
        .to_ascii_lowercase();
    no_proxy.split(',').map(str::trim).any(|entry| {
        let entry = entry
            .trim_start_matches("*.")
            .trim_start_matches('.')
            .to_ascii_lowercase();
        entry == "*"
            || (!entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}"))))
    })
}

/// A failed request, in words. Transport failures are marked so
/// [`Session::send_retrying`] can tell them from a refusal.
fn send_error(error: &minreq::Error, host: &str, proxy: Option<&Proxy>, timeout: u64) -> SendFailure {
    let via = proxy.map_or_else(String::new, |p| {
        format!(" through the proxy at {} ({})", p.shown, p.var)
    });
    let (message, transport) = match (error, proxy) {
        (minreq::Error::IoError(e), _)
            if matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            (format!("no answer from {host}{via} within {timeout} seconds"), true)
        }
        (minreq::Error::IoError(e), _) => (format!("can't reach {host}{via}: {e}"), true),
        (minreq::Error::AddressNotFound, _) => (
            format!("can't reach {host}{via}: its address didn't resolve"),
            true,
        ),
        #[cfg(not(target_os = "linux"))]
        (minreq::Error::NativeTlsCreateConnection(e), _) => (
            format!("the secure connection to {host}{via} failed: {e}"),
            true,
        ),
        #[cfg(target_os = "linux")]
        (minreq::Error::RustlsCreateConnection(e), _) => (
            format!("the secure connection to {host}{via} failed: {e}"),
            true,
        ),
        (minreq::Error::InvalidProxyCreds, Some(p)) if p.has_credentials => (
            format!(
                "the proxy at {} ({}) refused its credentials",
                p.shown, p.var
            ),
            false,
        ),
        (minreq::Error::InvalidProxyCreds, Some(p)) => (
            format!(
                "the proxy at {} ({}) asks for credentials; put them in {} as \
                 'http://user:password@{}'",
                p.shown, p.var, p.var, p.shown
            ),
            false,
        ),
        (minreq::Error::BadProxy, Some(p)) => (
            format!(
                "the proxy at {} ({}) wouldn't open a connection to {host}",
                p.shown, p.var
            ),
            false,
        ),
        (minreq::Error::ProxyConnect, Some(p)) => (
            format!(
                "the proxy at {} ({}) closed the connection without answering",
                p.shown, p.var
            ),
            true,
        ),
        (other, _) => (format!("sending to {host}{via} failed: {other}"), true),
    };
    SendFailure {
        error: CliError::new(message),
        transport,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_proxy_exempts_the_host_and_the_hosts_under_an_entry() {
        let host = "o325723.ingest.us.sentry.io";
        for list in [
            "*",
            "sentry.io",
            ".sentry.io",
            "*.sentry.io",
            "localhost, ingest.us.sentry.io",
            "SENTRY.IO",
            host,
        ] {
            assert!(exempts(list, host), "{list}");
        }
        for list in ["", "localhost,127.0.0.1", "ry.io", "sentry.io.example", ","] {
            assert!(!exempts(list, host), "{list}");
        }
        assert!(
            exempts("127.0.0.1", "127.0.0.1:8080"),
            "the port is not the host"
        );
    }

    #[test]
    fn a_proxy_is_shown_without_its_credentials() {
        let proxy = Proxy::parse("HTTPS_PROXY", "http://me:secret@proxy.corp:3128/").unwrap();
        assert_eq!(proxy.shown, "proxy.corp:3128");
        assert!(proxy.has_credentials);
        let proxy = Proxy::parse("https_proxy", "proxy.corp:3128").unwrap();
        assert_eq!(proxy.shown, "proxy.corp:3128");
        assert!(!proxy.has_credentials);

        for value in [
            "socks5://me:secret@proxy.corp:1080",
            "https://proxy.corp",
            "proxy:port",
        ] {
            let Err(err) = Proxy::parse("HTTPS_PROXY", value) else {
                panic!("{value} parsed");
            };
            let message = err.to_string();
            assert!(
                message.starts_with("HTTPS_PROXY names a proxy"),
                "{message}"
            );
            assert!(!message.contains("secret"), "{message}");
        }
    }

    #[test]
    fn a_session_picks_the_proxy_by_scheme_and_honors_no_proxy() {
        let session = Session {
            https: Some(("https_proxy", "http://proxy.corp:3128".into())),
            http: None,
            no_proxy: Some("apple.com".into()),
        };
        assert!(session.proxy("https://example.com/x").unwrap().is_some());
        assert!(session.proxy("https://api.appstoreconnect.apple.com/v1").unwrap().is_none());
        assert!(session.proxy("http://timestamp.example.com/ts").unwrap().is_none());
        assert_eq!(host("https://a.example.com:8443/v1?x=y"), "a.example.com:8443");
        assert_eq!(host("http://127.0.0.1:9/"), "127.0.0.1:9");
    }
}
