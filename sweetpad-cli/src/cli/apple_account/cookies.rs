//! Apple's sign-in cookies: kept from `Set-Cookie`, sent back on matching
//! HTTPS requests, and written to the session file.
//!
//! A cookie that names its domain is stored as `.apple.com` and matches the
//! hosts under it; one that names none matches only the host that set it.

use super::url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cookie {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) domain: String,
    pub(crate) domain_specified: bool,
    pub(crate) path: String,
    /// Seconds since the epoch; `None` for a cookie that lasts the session.
    pub(crate) expires: Option<i64>,
}

impl Cookie {
    pub(crate) fn expired(&self, now: i64) -> bool {
        self.expires.is_some_and(|expires| expires <= now)
    }

    fn matches_host(&self, host: &str) -> bool {
        if self.domain_specified {
            let domain = self.domain.trim_start_matches('.');
            host == domain || host.ends_with(&format!(".{domain}"))
        } else {
            host == self.domain
        }
    }

    fn matches_path(&self, path: &str) -> bool {
        path == self.path
            || (path.starts_with(&self.path)
                && (self.path.ends_with('/') || path[self.path.len()..].starts_with('/')))
    }
}

#[derive(Default)]
pub(crate) struct Jar {
    cookies: Vec<Cookie>,
}

impl Jar {
    /// Add `cookie`, replacing one with the same domain, path and name.
    pub(crate) fn set(&mut self, cookie: Cookie) {
        self.remove(&cookie);
        self.cookies.push(cookie);
    }

    fn remove(&mut self, cookie: &Cookie) {
        self.cookies.retain(|c| {
            (c.domain.as_str(), c.path.as_str(), c.name.as_str())
                != (
                    cookie.domain.as_str(),
                    cookie.path.as_str(),
                    cookie.name.as_str(),
                )
        });
    }

    pub(crate) fn clear(&mut self) {
        self.cookies.clear();
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Cookie> {
        self.cookies.iter()
    }

    /// The `Cookie` header for a request to `target`, sent over HTTPS only.
    pub(crate) fn header(&self, target: &str, now: i64) -> Option<String> {
        let parsed = url::parse(target)?;
        if parsed.scheme != "https" {
            return None;
        }
        let path = if parsed.path.is_empty() {
            "/"
        } else {
            parsed.path
        };
        let mut matching: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|c| !c.expired(now) && c.matches_host(&parsed.host) && c.matches_path(path))
            .collect();
        // The most specific path first, as browsers send them.
        matching.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
        (!matching.is_empty()).then(|| {
            matching
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    /// Keep the cookies a response from `target` sets.
    pub(crate) fn receive(&mut self, target: &str, headers: &[(String, String)], now: i64) {
        let Some(parsed) = url::parse(target) else {
            return;
        };
        for (_, value) in headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
        {
            if let Some(cookie) = parse_set_cookie(value, &parsed, now) {
                if cookie.expired(now) {
                    self.remove(&cookie);
                } else {
                    self.set(cookie);
                }
            }
        }
    }
}

/// One `Set-Cookie` value from a response to `target`, or `None` when it is
/// malformed or names a domain the host is not under.
fn parse_set_cookie(value: &str, target: &url::Url<'_>, now: i64) -> Option<Cookie> {
    let mut parts = value.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let (name, value) = (name.trim(), value.trim());
    if name.is_empty() || name.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
        return None;
    }
    let (mut domain, mut path, mut expires, mut max_age) = (None, None, None, None);
    for attribute in parts {
        let (key, value) = attribute.split_once('=').unwrap_or((attribute, ""));
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "domain" if !value.trim_start_matches('.').is_empty() => {
                domain = Some(value.trim_start_matches('.').to_ascii_lowercase());
            }
            "path" if value.starts_with('/') => path = Some(value.to_string()),
            "expires" => expires = http_date(value),
            "max-age" => max_age = value.parse::<i64>().ok(),
            _ => {}
        }
    }
    let host = target.host.as_str();
    let (domain, domain_specified) = match domain {
        Some(domain) => {
            // A domain must have an inner dot and cover the host.
            if !domain.contains('.') || !(host == domain || host.ends_with(&format!(".{domain}"))) {
                return None;
            }
            (format!(".{domain}"), true)
        }
        None => (host.to_string(), false),
    };
    let path = path.unwrap_or_else(|| match target.path.rfind('/') {
        Some(end) if end > 0 => target.path[..end].to_string(),
        _ => "/".to_string(),
    });
    let expires = match max_age {
        Some(seconds) => Some(now.saturating_add(seconds)),
        None => expires,
    };
    Some(Cookie {
        name: name.to_string(),
        value: value.to_string(),
        domain,
        domain_specified,
        path,
        expires,
    })
}

/// An HTTP date (`Thu, 01 Jan 2026 00:00:00 GMT`, `Thursday, 01-Jan-26
/// 00:00:00 GMT` or `Thu Jan  1 00:00:00 2026`) as seconds since the epoch.
pub(crate) fn http_date(text: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let (mut day, mut month, mut year, mut clock) = (None, None, None, None);
    for token in text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == ':'))
        .filter(|t| !t.is_empty())
    {
        if token.contains(':') {
            let mut parts = token.split(':').map(str::parse::<i64>);
            let (h, m, s) = (
                parts.next()?.ok()?,
                parts.next()?.ok()?,
                parts.next()?.ok()?,
            );
            clock = Some(h * 3600 + m * 60 + s);
        } else if token.bytes().all(|b| b.is_ascii_digit()) {
            let number: i64 = token.parse().ok()?;
            if day.is_none() && token.len() <= 2 {
                day = Some(number);
            } else {
                year = Some(match (token.len(), number) {
                    (2, 70..) => 1900 + number,
                    (2, _) => 2000 + number,
                    _ => number,
                });
            }
        } else if let Some(index) = MONTHS
            .iter()
            .position(|m| token.len() >= 3 && token[..3].eq_ignore_ascii_case(m))
        {
            month = Some(i64::try_from(index).ok()? + 1);
        }
    }
    let (day, month, year) = (day?, month?, year?);
    if !(1..=31).contains(&day) {
        return None;
    }
    Some(crate::cli::exits::days_from_civil(year, month, day) * 86_400 + clock.unwrap_or(0))
}
