//! The little of URL handling Apple's sign-in needs: splitting, joining a
//! redirect onto the URL it came from, and query encoding.

use std::fmt::Write as _;

/// The parts of an absolute URL.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Url<'a> {
    /// Lower case.
    pub(crate) scheme: String,
    pub(crate) has_userinfo: bool,
    /// Lower case; empty when the URL has no authority.
    pub(crate) host: String,
    /// `None` when absent, `Some(None)` when it is not a port number.
    #[allow(clippy::option_option)] // an absent port and a malformed one differ
    pub(crate) port: Option<Option<u16>>,
    pub(crate) path: &'a str,
    pub(crate) query: Option<&'a str>,
}

pub(crate) fn parse(url: &str) -> Option<Url<'_>> {
    let (scheme, rest) = url.split_once(':')?;
    let mut chars = scheme.chars();
    if !chars.next()?.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    let rest = rest.split('#').next().unwrap_or_default();
    let (authority, rest) = match rest.strip_prefix("//") {
        Some(rest) => {
            let end = rest.find(['/', '?']).unwrap_or(rest.len());
            (&rest[..end], &rest[end..])
        }
        None => ("", rest),
    };
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };
    let (has_userinfo, address) = match authority.rsplit_once('@') {
        Some((_, address)) => (true, address),
        None => (false, authority),
    };
    let (host, port) = match address.rsplit_once(':') {
        Some((host, "")) => (host, None),
        Some((host, port)) => (
            host,
            Some(
                port.bytes()
                    .all(|b| b.is_ascii_digit())
                    .then(|| port.parse().ok())
                    .flatten(),
            ),
        ),
        None => (address, None),
    };
    Some(Url {
        scheme: scheme.to_ascii_lowercase(),
        has_userinfo,
        host: host.to_ascii_lowercase(),
        port,
        path,
        query,
    })
}

/// Whether `url` is an HTTPS URL on apple.com or a host under it, on the
/// default port and without credentials. Every request and redirect is
/// checked against it, so cookies and passwords go to Apple only.
pub(crate) fn apple(url: &str) -> bool {
    let Some(url) = parse(url) else {
        return false;
    };
    url.scheme == "https"
        && !url.has_userinfo
        && matches!(url.port, None | Some(Some(443)))
        && url
            .host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
        && (url.host == "apple.com" || url.host.ends_with(".apple.com"))
}

/// `reference`, such as a `Location` header, resolved against `base`.
pub(crate) fn join(base: &str, reference: &str) -> String {
    if reference.is_empty() {
        return base.to_string();
    }
    if parse(reference).is_some_and(|url| !url.scheme.is_empty()) && !reference.starts_with('/') {
        return reference.to_string();
    }
    let Some((scheme, rest)) = base.split_once("://") else {
        return reference.to_string();
    };
    if let Some(network) = reference.strip_prefix("//") {
        return format!("{scheme}://{network}");
    }
    let rest = rest.split('#').next().unwrap_or_default();
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let origin = format!("{scheme}://{}", &rest[..authority_end]);
    let full_path = &rest[authority_end..];
    if reference.starts_with('/') {
        return format!("{origin}{reference}");
    }
    let path = full_path.split('?').next().unwrap_or_default();
    if reference.starts_with('?') {
        return format!("{origin}{path}{reference}");
    }
    let directory = path.rfind('/').map_or("/", |end| &path[..=end]);
    format!("{origin}{directory}{reference}")
}

/// The first non-empty value of `key` in a query string, decoded the way a
/// form is (`+` is a space).
pub(crate) fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        let value = decode(&value.replace('+', " "));
        (decode(&name.replace('+', " ")) == key && !value.is_empty()).then_some(value)
    })
}

/// `%XX` escapes decoded, with malformed ones kept as they are.
pub(crate) fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            #[allow(clippy::cast_possible_truncation)] // two hex digits make a byte
            out.push((high * 16 + low) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `text` with every byte other than letters, digits, `_.-~` and those in
/// `safe` written as `%XX`.
pub(crate) fn encode(text: &str, safe: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'_' | b'.' | b'-' | b'~')
            || safe.as_bytes().contains(&byte)
        {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// `pairs` as a form-encoded query string.
pub(crate) fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                encode(name, " ").replace(' ', "+"),
                encode(value, " ").replace(' ', "+")
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}
