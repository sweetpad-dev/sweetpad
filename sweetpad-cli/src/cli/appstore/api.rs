//! Calls to the App Store Connect API, and its errors in words.

use std::cell::RefCell;

use serde_json::Value;

use super::jwt;
use super::key::ApiKey;
use crate::cli::net::{Request, Session};
use crate::cli::CliError;

const BASE: &str = "https://api.appstoreconnect.apple.com";

/// A refused or failed call: the status and Apple's error codes, so a caller
/// can tell a duplicate from any other refusal, beside the message.
pub(crate) struct ApiFailure {
    pub(crate) status: Option<u16>,
    pub(crate) codes: Vec<String>,
    pub(crate) error: Box<CliError>,
}

impl From<ApiFailure> for CliError {
    fn from(failure: ApiFailure) -> Self {
        *failure.error
    }
}

/// A client signed with one API key. Single-threaded: it caches its token.
pub(crate) struct Client {
    pub(crate) session: Session,
    key: ApiKey,
    base: String,
    token: RefCell<Option<(String, u64)>>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Client {
    /// `SWEETPAD_ASC_BASE_URL` points the client at another server; the
    /// tests' stub is the only one.
    pub(crate) fn new(key: ApiKey, session: Session) -> Self {
        let base = std::env::var("SWEETPAD_ASC_BASE_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| BASE.to_string());
        Self {
            session,
            key,
            base: base.trim_end_matches('/').to_string(),
            token: RefCell::new(None),
        }
    }

    /// A bearer token, reused until a minute before it expires.
    pub(crate) fn bearer(&self) -> String {
        let now = now();
        let mut cached = self.token.borrow_mut();
        if let Some((token, expires)) = cached.as_ref()
            && *expires > now + 60
        {
            return format!("Bearer {token}");
        }
        let token = jwt::token(&self.key, now);
        *cached = Some((token.clone(), now + jwt::LIFETIME_SECS));
        format!("Bearer {token}")
    }

    pub(crate) fn get(&self, path: &str) -> Result<Value, ApiFailure> {
        self.call(minreq::Method::Get, path, None, true)
    }

    pub(crate) fn post(&self, path: &str, body: &Value) -> Result<Value, ApiFailure> {
        self.call(minreq::Method::Post, path, Some(body), false)
    }

    pub(crate) fn patch(&self, path: &str, body: &Value) -> Result<Value, ApiFailure> {
        self.call(minreq::Method::Patch, path, Some(body), true)
    }

    fn call(
        &self,
        method: minreq::Method,
        path: &str,
        body: Option<&Value>,
        repeatable: bool,
    ) -> Result<Value, ApiFailure> {
        let shown = format!("{method} {}", path.split('?').next().unwrap_or(path));
        let mut request = Request::new(method, format!("{}{path}", self.base))
            .header("Authorization", self.bearer())
            .header("Accept", "application/json")
            .header("User-Agent", concat!("sweetpad-cli/", env!("CARGO_PKG_VERSION")));
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body.to_string().into_bytes());
        }
        let sent = if repeatable {
            self.session.send_retrying(&request, 4)
        } else {
            self.session.send(&request)
        };
        let response = sent.map_err(|error| ApiFailure {
            status: None,
            codes: Vec::new(),
            error: Box::new(error.context(format!("App Store Connect {shown}"))),
        })?;
        let value: Value = if response.body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&response.body).unwrap_or(Value::Null)
        };
        if response.ok() {
            return Ok(value);
        }
        Err(refusal(response.status, &value, &response.snippet(), &shown))
    }
}

/// Apple's `{"errors": [...]}` as one message.
fn refusal(status: u16, body: &Value, snippet: &str, shown: &str) -> ApiFailure {
    let errors = body["errors"].as_array().cloned().unwrap_or_default();
    let codes: Vec<String> = errors
        .iter()
        .filter_map(|e| e["code"].as_str().map(str::to_string))
        .collect();
    let details: Vec<String> = errors
        .iter()
        .map(|e| {
            let title = e["title"].as_str().unwrap_or_default();
            let detail = e["detail"].as_str().unwrap_or_default();
            match (title.is_empty(), detail.is_empty()) {
                (_, false) if detail.starts_with(title) || title.is_empty() => detail.to_string(),
                (false, false) => format!("{title}: {detail}"),
                _ => title.to_string(),
            }
        })
        .filter(|d| !d.is_empty())
        .collect();
    let what = if details.is_empty() {
        if snippet.is_empty() {
            format!("HTTP {status}")
        } else {
            format!("HTTP {status}: {snippet}")
        }
    } else {
        format!("HTTP {status}: {}", details.join("; "))
    };
    let tip = match status {
        401 => Some(
            "check the key ID, the issuer ID (none for an individual key) and the .p8 file; a \
             revoked key also answers 401"
                .to_string(),
        ),
        403 => Some("the API key's role doesn't allow this; App Manager or Admin does".to_string()),
        _ => None,
    };
    ApiFailure {
        status: Some(status),
        codes,
        error: Box::new(
            CliError::new(what)
                .context(format!("App Store Connect refused {shown}"))
                .tip(tip),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_errors_become_one_message_with_their_codes() {
        let body = serde_json::json!({"errors": [{
            "status": "409",
            "code": "ENTITY_ERROR.ATTRIBUTE.INVALID.DUPLICATE",
            "title": "The provided entity includes an attribute with a value that has already been used",
            "detail": "The bundle version must be higher than the previously uploaded version."
        }]});
        let failure = refusal(409, &body, "", "POST /v1/buildUploads");
        assert_eq!(failure.status, Some(409));
        assert_eq!(failure.codes, ["ENTITY_ERROR.ATTRIBUTE.INVALID.DUPLICATE"]);
        let message = failure.error.to_string();
        assert!(message.starts_with("App Store Connect refused POST /v1/buildUploads: HTTP 409: The provided entity"), "{message}");
        assert!(message.contains("must be higher"), "{message}");
        let failure = refusal(401, &Value::Null, "Unauthenticated", "GET /v1/apps");
        assert_eq!(failure.error.to_string(), "App Store Connect refused GET /v1/apps: HTTP 401: Unauthenticated");
        assert!(failure.error.tip_text().unwrap().contains("issuer"));
    }
}
