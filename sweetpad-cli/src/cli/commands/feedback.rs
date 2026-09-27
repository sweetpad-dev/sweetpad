//! `sweetpad feedback …`: a problem report about sweetpad itself, sent to the
//! maintainer as Sentry user feedback once the user has approved it twice
//! (CLI_DESIGN §9t).
//!
//! The agent driving sweetpad writes the report and cleans it; sweetpad sends
//! the text as written. `submit --dry-run` prints the exact payload and a
//! digest of it, and `submit --approve <digest>` sends that payload only while
//! the digest still matches. The CLI never prompts: both approvals are the
//! agent asking the user.

use std::path::{Path, PathBuf};

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::{CliError, CommandResult, Context, ErrorKind, Render, Rendered, config};

/// The VS Code extension's Sentry project: the DSN in
/// `sweetpad-vscode/.env.example`. A DSN's key is a public client key, which
/// can submit events and read nothing.
const SENTRY_DSN: &str =
    "https://5f6c5f5f15492a077f6bac254c6d8461@o325723.ingest.us.sentry.io/4507950563328000";

/// A full envelope URL to send to instead of Sentry's, for the tests' local
/// stub server. Not documented.
pub(crate) const ENDPOINT_ENV: &str = "SWEETPAD_FEEDBACK_URL";

/// The most a send may take, connecting included.
const SEND_TIMEOUT_SECS: u64 = 10;

/// Sentry keeps at most this many characters of a feedback message.
const MAX_MESSAGE_CHARS: usize = 4096;

/// How many hex digits of the payload's SHA-256 make the digest.
const DIGEST_HEX_DIGITS: usize = 16;

/// Report kinds: the sweetpad-feedback log's categories for an issue.
const KINDS: [&str; 6] = [
    "bug",
    "gap",
    "unclear",
    "skill-wrong",
    "docs-wrong",
    "friction",
];

const SEVERITIES: [&str; 3] = ["low", "medium", "high"];

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Check a report file and print exactly what would be sent
    /// ('--dry-run'), or send it once the user approved that ('--approve').
    Submit {
        /// The report, in the format 'sweetpad help feedback' shows.
        file: PathBuf,
        /// Print the exact payload and its digest, and send nothing.
        #[arg(long, conflicts_with = "approve")]
        dry_run: bool,
        /// Send the report, if its payload still has this digest (printed by
        /// '--dry-run').
        #[arg(long, value_name = "DIGEST")]
        approve: Option<String>,
    },
    /// Turn feedback reports off: 'submit' refuses, and 'sweetpad help
    /// feedback' tells agents not to offer one.
    Off,
    /// Turn feedback reports back on.
    On,
    /// Say whether feedback reports are on.
    Status,
}

pub fn run(_ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Submit {
            file,
            dry_run,
            approve,
        } => submit(file, *dry_run, approve.as_deref()),
        Action::Off => toggle(false),
        Action::On => toggle(true),
        Action::Status => status(),
    }
}

/// Whether feedback reports are on: `[feedback] enabled` in config.toml, on
/// when unset. A config that doesn't parse is an error rather than the
/// default, because it may be the file that turned them off.
pub(crate) fn enabled() -> Result<bool, String> {
    config::Config::load().map(|c| c.feedback.enabled.unwrap_or(true))
}

fn config_path_text() -> String {
    config::Config::path().map_or_else(|| "config.toml".to_string(), |p| p.display().to_string())
}

/// `submit`: a dry run when `approve` is `None` (clap keeps the two flags
/// apart), a send when it is set.
fn submit(file: &Path, dry_run: bool, approve: Option<&str>) -> CommandResult {
    let shown_file = crate::cli::xcodebuild::shell_quote(&file.display().to_string());
    if !dry_run && approve.is_none() {
        return Err(CliError::new(format!(
            "'submit' sends nothing without an approved digest: run 'sweetpad feedback submit \
             {shown_file} --dry-run', show the user what it prints, and once they approve, run it \
             with '--approve <digest>'"
        ))
        .kind(ErrorKind::Usage));
    }
    match enabled() {
        Ok(true) => {}
        Ok(false) => {
            return Err(CliError::new(format!(
                "feedback reports are turned off ('[feedback] enabled = false' in {}), so nothing \
                 was sent",
                config_path_text()
            )));
        }
        Err(e) => {
            return Err(CliError::new(format!(
                "can't tell whether feedback reports are turned off, so nothing was sent: {e}"
            )));
        }
    }
    let text = std::fs::read_to_string(file).map_err(|e| {
        CliError::new(format!("can't read {}: {e}", file.display())).kind(ErrorKind::Usage)
    })?;
    let report = Report::parse(&text).map_err(|e| CliError::new(e).kind(ErrorKind::Usage))?;
    let endpoint = Endpoint::current();
    let payload = Payload::build(&report, &Host::detect(), &endpoint)
        .map_err(|e| CliError::new(e).kind(ErrorKind::Usage))?;

    let Some(approve) = approve else {
        return Ok(Rendered::data(DryRun {
            send_command: format!(
                "sweetpad feedback submit {shown_file} --approve {}",
                payload.digest
            ),
            payload,
            endpoint: endpoint.url,
        }));
    };
    if !approve.trim().eq_ignore_ascii_case(&payload.digest) {
        // The new digest stays out of this message: an agent that could copy
        // it from here could send a payload the user never saw.
        return Err(CliError::new(format!(
            "the report's payload no longer has digest '{}', so nothing was sent. The file, \
             sweetpad, Xcode or macOS changed since the dry run, or the digest was mistyped. Run \
             'sweetpad feedback submit {shown_file} --dry-run' again and show the user the new \
             payload before sending it",
            approve.trim()
        )));
    }
    let event_id = send(&payload, &endpoint)?;
    Ok(Rendered::data(Sent {
        event_id,
        digest: payload.digest,
    }))
}

fn toggle(enabled: bool) -> CommandResult {
    let edit = config::set_feedback_enabled(enabled).map_err(|e| {
        CliError::new(e).context(if enabled {
            "turning feedback reports on"
        } else {
            "turning feedback reports off"
        })
    })?;
    Ok(Rendered::data(Toggled {
        enabled,
        path: edit.path,
        changed: edit.changed,
    }))
}

fn status() -> CommandResult {
    let path = config::Config::path();
    let config = config::Config::load()
        .map_err(|e| CliError::new(e).context("reading whether feedback reports are on"))?;
    Ok(Rendered::data(Status {
        enabled: config.feedback.enabled.unwrap_or(true),
        set_in_config: config.feedback.enabled.is_some(),
        path,
    }))
}

/// One report field, in the order the log entry writes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Context,
    Command,
    Expected,
    Actual,
    AssumptionOrGap,
    FixIdea,
}

impl Field {
    const ALL: [Field; 6] = [
        Field::Context,
        Field::Command,
        Field::Expected,
        Field::Actual,
        Field::AssumptionOrGap,
        Field::FixIdea,
    ];

    /// The name the log entry gives it: `- **<label>:** …`.
    fn label(self) -> &'static str {
        match self {
            Field::Context => "Context",
            Field::Command => "Command",
            Field::Expected => "Expected",
            Field::Actual => "Actual",
            Field::AssumptionOrGap => "Assumption or gap",
            Field::FixIdea => "Fix idea",
        }
    }

    fn required(self) -> bool {
        self != Field::FixIdea
    }
}

/// A parsed report file: one sweetpad-feedback log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Report {
    kind: String,
    severity: String,
    /// Every field the file gave a value, in [`Field::ALL`] order.
    fields: Vec<(Field, String)>,
}

/// A field line's name, when it names something the log entry has.
enum Named {
    Sent(Field),
    /// The log's `Seen:` counter, with any sub-bullets under it: read, and
    /// not sent, since its lines carry dates and the other projects an issue
    /// turned up in.
    LocalOnly,
}

fn named(name: &str) -> Option<Named> {
    if name.eq_ignore_ascii_case("seen") {
        return Some(Named::LocalOnly);
    }
    Field::ALL
        .into_iter()
        .find(|f| f.label().eq_ignore_ascii_case(name))
        .map(Named::Sent)
}

impl Report {
    /// Parse a log entry: the `## <timestamp> · <kind> · <severity>` heading,
    /// then `- **<Field>:** <value>` lines. A value runs to the next field
    /// line, so it may wrap or hold a code block.
    fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines().enumerate();
        let Some((_, heading)) = lines.by_ref().find(|(_, l)| !l.trim().is_empty()) else {
            return Err(
                "the report file is empty; 'sweetpad help feedback' shows the format".to_string(),
            );
        };
        let (kind, severity) = parse_heading(heading.trim_end())?;

        // (field, value); `None` marks a field that is read and not sent.
        let mut values: Vec<(Option<Field>, String)> = Vec::new();
        for (index, line) in lines {
            let number = index + 1;
            if line.starts_with("## ") || line.trim_end() == "##" {
                return Err(format!(
                    "line {number} starts a second report; send one report per file"
                ));
            }
            if let Some((name, value)) = field_line(line) {
                let field = match named(name) {
                    Some(Named::Sent(field)) => {
                        if values.iter().any(|(f, _)| *f == Some(field)) {
                            return Err(format!(
                                "line {number} gives '{}' a second time",
                                field.label()
                            ));
                        }
                        Some(field)
                    }
                    Some(Named::LocalOnly) => None,
                    None => {
                        return Err(format!(
                            "line {number} has an unknown field '{name}' (the fields are {})",
                            quoted_labels(Field::ALL.iter().copied())
                        ));
                    }
                };
                values.push((field, value.to_string()));
            } else if let Some((_, value)) = values.last_mut() {
                value.push('\n');
                value.push_str(line);
            } else if !line.trim().is_empty() {
                return Err(format!(
                    "line {number} comes before the first field; after the heading, every line \
                     belongs to a field such as '- **Context:** …'"
                ));
            }
        }

        let fields: Vec<(Field, String)> = Field::ALL
            .into_iter()
            .filter_map(|field| {
                values
                    .iter()
                    .find(|(f, _)| *f == Some(field))
                    .map(|(_, v)| (field, v.trim().to_string()))
                    .filter(|(_, v)| !v.is_empty())
            })
            .collect();
        let missing: Vec<Field> = Field::ALL
            .into_iter()
            .filter(|f| f.required() && !fields.iter().any(|(g, _)| g == f))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "the report is missing {}; every field but 'Fix idea' needs a value \
                 ('sweetpad help feedback' shows the format)",
                quoted_labels(missing.into_iter())
            ));
        }
        Ok(Self {
            kind,
            severity,
            fields,
        })
    }

    /// The feedback message the maintainer reads.
    fn message(&self) -> String {
        let mut message = format!("{} · {}\n", self.kind, self.severity);
        for (field, value) in &self.fields {
            message.push('\n');
            message.push_str(field.label());
            message.push_str(": ");
            message.push_str(value);
        }
        message
    }
}

/// `'Context', 'Command' and 'Actual'`.
fn quoted_labels(fields: impl Iterator<Item = Field>) -> String {
    let labels: Vec<String> = fields.map(|f| format!("'{}'", f.label())).collect();
    match labels.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// The kind and severity from `## <timestamp> · <kind> · <severity>`. The
/// timestamp may be left out, and is not sent.
fn parse_heading(line: &str) -> Result<(String, String), String> {
    let expected = "the file must start with the heading '## <timestamp> · <kind> · <severity>'";
    let Some(rest) = line.strip_prefix("## ") else {
        return Err(format!("{expected}, and its first line is '{line}'"));
    };
    let parts: Vec<&str> = rest.split('·').map(str::trim).collect();
    let (kind, severity) = match parts.as_slice() {
        [_, kind, severity] | [kind, severity] => (kind.to_lowercase(), severity.to_lowercase()),
        _ => return Err(format!("{expected}, and it is '{line}'")),
    };
    if !KINDS.contains(&kind.as_str()) {
        let why = if matches!(kind.as_str(), "correction" | "resolved") {
            format!("'{kind}' entries are about the local log; a report describes the issue itself")
        } else {
            format!("the heading's kind is '{kind}'")
        };
        return Err(format!("{why} (kinds: {})", KINDS.join(", ")));
    }
    if !SEVERITIES.contains(&severity.as_str()) {
        return Err(format!(
            "the heading's severity is '{severity}' (severities: {})",
            SEVERITIES.join(", ")
        ));
    }
    Ok((kind, severity))
}

/// `- **Name:** value` (or `- **Name**: value`) as `(name, value)`.
fn field_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("- **")?;
    let end = rest.find("**")?;
    let (label, after) = (&rest[..end], &rest[end + 2..]);
    let (name, value) = if let Some(name) = label.strip_suffix(':') {
        (name, after)
    } else {
        (label, after.strip_prefix(':')?)
    };
    Some((name.trim(), value.trim_start()))
}

/// What sweetpad adds about the machine: versions, and nothing that names
/// the user, the Mac or a path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Host {
    sweetpad: String,
    /// `26.0.1 (17A400)`, from the active Xcode's version.plist.
    xcode: Option<String>,
    macos_version: Option<String>,
    macos_build: Option<String>,
    arch: String,
}

impl Host {
    fn detect() -> Self {
        let xcode = sweetpad_lib::xcode::active_install();
        let xcode = (!xcode.short_version.is_empty()).then(|| {
            if xcode.build_version.is_empty() {
                xcode.short_version.clone()
            } else {
                format!("{} ({})", xcode.short_version, xcode.build_version)
            }
        });
        Self {
            sweetpad: env!("SWEETPAD_VERSION").to_string(),
            xcode,
            macos_version: sysctl_string("kern.osproductversion"),
            macos_build: sysctl_string("kern.osversion"),
            arch: match std::env::consts::ARCH {
                "aarch64" => "arm64".to_string(),
                other => other.to_string(),
            },
        }
    }

    fn macos(&self) -> String {
        match (&self.macos_version, &self.macos_build) {
            (Some(version), Some(build)) => format!("{version} ({build})"),
            (Some(version), None) => version.clone(),
            _ => "unknown".to_string(),
        }
    }
}

/// A string-valued sysctl, such as `kern.osproductversion`.
fn sysctl_string(name: &str) -> Option<String> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut len: libc::size_t = 0;
    // Safety: a null buffer asks for the value's length only.
    let asked = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &raw mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if asked != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    // Safety: `buf` holds the `len` bytes the first call asked for.
    let read = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            &raw mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if read != 0 {
        return None;
    }
    buf.truncate(len);
    let text = String::from_utf8(buf).ok()?;
    let text = text.trim_end_matches('\0').trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Where a report goes: Sentry's envelope endpoint for [`SENTRY_DSN`], or the
/// test override.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Endpoint {
    url: String,
    host: String,
    key: String,
}

impl Endpoint {
    fn current() -> Self {
        let dsn = Self::from_dsn(SENTRY_DSN).expect("the embedded DSN parses");
        match std::env::var(ENDPOINT_ENV) {
            Ok(url) if !url.is_empty() => Self {
                host: host_of(&url),
                url,
                key: dsn.key,
            },
            _ => dsn,
        }
    }

    /// `https://<key>@<host>/<project>` to its envelope endpoint.
    fn from_dsn(dsn: &str) -> Option<Self> {
        let rest = dsn.strip_prefix("https://")?;
        let (key, rest) = rest.split_once('@')?;
        let (host, project) = rest.split_once('/')?;
        Some(Self {
            url: format!("https://{host}/api/{project}/envelope/"),
            host: host.to_string(),
            key: key.to_string(),
        })
    }
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest).to_string()
}

/// The event a report becomes, and its digest.
#[derive(Debug, Clone)]
struct Payload {
    /// The Sentry event as it is sent, without the `timestamp` added then.
    event: serde_json::Value,
    message: String,
    host: Host,
    /// The first [`DIGEST_HEX_DIGITS`] hex digits of the SHA-256.
    digest: String,
    event_id: String,
}

impl Payload {
    /// Build the feedback event. Everything in it except `event_id` goes into
    /// the SHA-256, along with the endpoint, so an approval covers both what
    /// is sent and where. The digest is the hash's first half, and `event_id`
    /// its second half shaped as a UUID v4, so a dry run and its send agree on
    /// both. `timestamp` is added at send time and is not hashed.
    fn build(report: &Report, host: &Host, endpoint: &Endpoint) -> Result<Self, String> {
        let message = report.message();
        let chars = message.chars().count();
        if chars > MAX_MESSAGE_CHARS {
            return Err(format!(
                "the report is {chars} characters and Sentry keeps {MAX_MESSAGE_CHARS}; shorten it"
            ));
        }
        let version = host.sweetpad.as_str();
        let mut os = serde_json::json!({ "name": "macOS" });
        if let Some(v) = &host.macos_version {
            os["version"] = v.clone().into();
        }
        if let Some(b) = &host.macos_build {
            os["build"] = b.clone().into();
        }
        // `platform` is neither javascript nor cocoa, the platforms Relay
        // gives the connection's IP when an event names none, and
        // `infer_ip: never` turns inference off for the rest; there is no
        // `user` object to hold one.
        let mut event = serde_json::json!({
            "platform": "other",
            "level": "info",
            "release": format!("sweetpad-cli@{version}"),
            "environment": if version.contains("-dev") { "development" } else { "production" },
            "sdk": {
                "name": "sweetpad-cli",
                "version": version,
                "settings": { "infer_ip": "never" },
            },
            "tags": {
                "source": "cli",
                "kind": report.kind,
                "severity": report.severity,
                "xcode": host.xcode.as_deref().unwrap_or("unknown"),
                "arch": host.arch,
            },
            "contexts": {
                "feedback": { "message": message },
                "os": os,
            },
        });
        let hashed = serde_json::json!({ "endpoint": endpoint.url, "event": event });
        let hash = hex(&sha256(hashed.to_string().as_bytes()));
        let event_id = uuid_v4_shaped(&hash[32..]);
        event["event_id"] = event_id.clone().into();
        Ok(Self {
            event,
            message,
            host: host.clone(),
            digest: hash[..DIGEST_HEX_DIGITS].to_string(),
            event_id,
        })
    }

    /// The Sentry envelope: its header, the `feedback` item header, and the
    /// event with the time of sending.
    fn envelope(&self, timestamp: u64) -> String {
        let mut event = self.event.clone();
        event["timestamp"] = timestamp.into();
        let body = event.to_string();
        let header = serde_json::json!({
            "event_id": self.event_id,
            "sdk": { "name": "sweetpad-cli", "version": self.host.sweetpad },
        });
        let item = serde_json::json!({ "type": "feedback", "length": body.len() });
        format!("{header}\n{item}\n{body}\n")
    }
}

/// SHA-256 from CommonCrypto, which is part of libSystem, so every macOS
/// process already has it.
fn sha256(data: &[u8]) -> [u8; 32] {
    unsafe extern "C" {
        fn CC_SHA256(data: *const std::ffi::c_void, len: u32, md: *mut u8) -> *mut u8;
    }
    let len = u32::try_from(data.len()).expect("a report payload is far below 4 GiB");
    let mut digest = [0u8; 32];
    // Safety: `data` is `len` readable bytes and `digest` the 32 bytes
    // CC_SHA256 writes.
    unsafe { CC_SHA256(data.as_ptr().cast(), len, digest.as_mut_ptr()) };
    digest
}

/// 32 hex digits with a UUID v4's version digit (`4`) and variant digit
/// (`8`–`b`) set, the form Sentry's feedback spec asks of `event_id`.
fn uuid_v4_shaped(hex32: &str) -> String {
    hex32
        .chars()
        .enumerate()
        .map(|(i, c)| match i {
            12 => '4',
            16 => {
                let variant = 8 | (c.to_digit(16).unwrap_or(0) & 3);
                char::from_digit(variant, 16).unwrap_or('8')
            }
            _ => c,
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// POST the envelope, returning the event id Sentry answers with.
fn send(payload: &Payload, endpoint: &Endpoint) -> Result<String, CliError> {
    let proxy = SendProxy::from_env(endpoint)?;
    // With no proxy named, minreq applies `https_proxy` by itself (and
    // `http_proxy` or `all_proxy` to plain HTTP), and it reads no `no_proxy`.
    // The choice above is the one that holds, so those go first.
    for key in ["https_proxy", "http_proxy", "all_proxy"] {
        // Safety: `feedback submit` runs on the main thread alone, and the
        // process exits once this send returns.
        unsafe { std::env::remove_var(key) };
    }
    let version = payload.host.sweetpad.as_str();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut request = minreq::post(&endpoint.url)
        .with_header("Content-Type", "application/x-sentry-envelope")
        .with_header(
            "X-Sentry-Auth",
            format!(
                "Sentry sentry_version=7, sentry_client=sweetpad-cli/{version}, sentry_key={}",
                endpoint.key
            ),
        )
        .with_header("User-Agent", format!("sweetpad-cli/{version}"))
        .with_body(payload.envelope(timestamp))
        .with_timeout(SEND_TIMEOUT_SECS);
    if let Some(proxy) = &proxy {
        request = request.with_proxy(proxy.proxy.clone());
    }
    let response = request.send().map_err(|e| {
        send_error(&e, &endpoint.host, proxy.as_ref()).context("couldn't send the report")
    })?;
    let body = response.as_str().unwrap_or_default();
    if !(200..300).contains(&response.status_code) {
        let detail: String = body.trim().chars().take(200).collect();
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        return Err(CliError::new(format!(
            "{} answered HTTP {} {}{detail}",
            endpoint.host, response.status_code, response.reason_phrase
        ))
        .context("the report was not accepted"));
    }
    Ok(serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["id"].as_str().map(str::to_string))
        .unwrap_or_else(|| payload.event_id.clone()))
}

/// The HTTP proxy a send goes through.
struct SendProxy {
    proxy: minreq::Proxy,
    /// The variable it came from, for the messages.
    var: &'static str,
    /// `host[:port]`, without the credentials.
    shown: String,
    has_credentials: bool,
}

impl SendProxy {
    /// `https_proxy` or `HTTPS_PROXY`, read in that order as curl reads them,
    /// unless `no_proxy` or `NO_PROXY` exempts the endpoint's host. An
    /// endpoint over plain HTTP, which only the tests' stub is, goes direct.
    fn from_env(endpoint: &Endpoint) -> Result<Option<Self>, CliError> {
        if !endpoint.url.starts_with("https://") {
            return Ok(None);
        }
        let Some((var, value)) = env_either("https_proxy", "HTTPS_PROXY") else {
            return Ok(None);
        };
        if let Some((_, no_proxy)) = env_either("no_proxy", "NO_PROXY")
            && exempts(&no_proxy, &endpoint.host)
        {
            return Ok(None);
        }
        Self::parse(var, &value).map(Some)
    }

    fn parse(var: &'static str, value: &str) -> Result<Self, CliError> {
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
                proxy,
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

/// The first of two environment variables that is set and not empty.
fn env_either(first: &'static str, second: &'static str) -> Option<(&'static str, String)> {
    [first, second].into_iter().find_map(|var| {
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
fn exempts(no_proxy: &str, host: &str) -> bool {
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

/// A failed send, in words.
fn send_error(error: &minreq::Error, host: &str, proxy: Option<&SendProxy>) -> CliError {
    let via = proxy.map_or_else(String::new, |p| {
        format!(" through the proxy at {} ({})", p.shown, p.var)
    });
    let message = match (error, proxy) {
        (minreq::Error::IoError(e), _)
            if matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            format!("no answer from {host}{via} within {SEND_TIMEOUT_SECS} seconds")
        }
        (minreq::Error::IoError(e), _) => format!("can't reach {host}{via}: {e}"),
        (minreq::Error::AddressNotFound, _) => {
            format!("can't reach {host}{via}: its address didn't resolve")
        }
        (minreq::Error::NativeTlsCreateConnection(e), _) => {
            format!("the secure connection to {host}{via} failed: {e}")
        }
        (minreq::Error::InvalidProxyCreds, Some(p)) if p.has_credentials => format!(
            "the proxy at {} ({}) refused its credentials",
            p.shown, p.var
        ),
        (minreq::Error::InvalidProxyCreds, Some(p)) => format!(
            "the proxy at {} ({}) asks for credentials; put them in {} as \
             'http://user:password@{}'",
            p.shown, p.var, p.var, p.shown
        ),
        (minreq::Error::BadProxy, Some(p)) => format!(
            "the proxy at {} ({}) wouldn't open a connection to {host}",
            p.shown, p.var
        ),
        (minreq::Error::ProxyConnect, Some(p)) => format!(
            "the proxy at {} ({}) closed the connection without answering",
            p.shown, p.var
        ),
        (other, _) => format!("sending to {host}{via} failed: {other}"),
    };
    CliError::new(message)
}

/// `submit --dry-run`: what would be sent, and how to send it.
struct DryRun {
    payload: Payload,
    endpoint: String,
    send_command: String,
}

impl Render for DryRun {
    fn human(&self, out: &Output) {
        let host = &self.payload.host;
        out.line("Nothing was sent. This is the report sweetpad would send to the SweetPad");
        out.line("maintainer, as user feedback in Sentry:");
        out.line("");
        for line in self.payload.message.lines() {
            out.line(format!("  {line}").trim_end());
        }
        out.line("");
        out.line("Added by sweetpad:");
        out.line("");
        out.line(&format!("  sweetpad  {}", host.sweetpad));
        out.line(&format!(
            "  Xcode     {}",
            host.xcode.as_deref().unwrap_or("unknown")
        ));
        out.line(&format!("  macOS     {}", host.macos()));
        out.line(&format!("  arch      {}", host.arch));
        out.line("");
        out.line(&format!("The exact payload. It goes to {},", self.endpoint));
        out.line("with the time of sending added as 'timestamp':");
        out.line("");
        out.line(&serde_json::to_string_pretty(&self.payload.event).unwrap_or_default());
        out.line("");
        out.line(&format!("digest: {}", self.payload.digest));
        out.line("");
        out.line("Once the user approves this payload, send it with:");
        out.line(&format!("  {}", self.send_command));
    }

    fn json(&self) -> serde_json::Value {
        let host = &self.payload.host;
        serde_json::json!({
            "sent": false,
            "digest": self.payload.digest,
            "endpoint": self.endpoint,
            "payload": self.payload.event,
            "addedAtSend": ["timestamp"],
            "added": {
                "sweetpad": host.sweetpad,
                "xcode": host.xcode,
                "macos": host.macos(),
                "arch": host.arch,
            },
            "sendCommand": self.send_command,
        })
    }
}

/// `submit --approve`: the report went out.
struct Sent {
    event_id: String,
    digest: String,
}

impl Render for Sent {
    fn human(&self, out: &Output) {
        out.line(&format!(
            "sent the report to the SweetPad maintainer (Sentry event {})",
            self.event_id
        ));
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({ "sent": true, "eventId": self.event_id, "digest": self.digest })
    }
}

/// `feedback off` / `feedback on`.
struct Toggled {
    enabled: bool,
    path: PathBuf,
    changed: bool,
}

impl Render for Toggled {
    fn human(&self, out: &Output) {
        let path = self.path.display();
        out.line(&match (self.enabled, self.changed) {
            (false, true) => {
                format!("feedback reports are off: wrote '[feedback] enabled = false' to {path}")
            }
            (false, false) => format!("feedback reports were already off ({path})"),
            (true, true) => {
                format!("feedback reports are on: set '[feedback] enabled = true' in {path}")
            }
            (true, false) => "feedback reports are on".to_string(),
        });
        if !self.enabled {
            out.note("'sweetpad feedback on' turns them back on");
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.enabled,
            "changed": self.changed,
            "configPath": self.path,
        })
    }
}

/// `feedback status`.
struct Status {
    enabled: bool,
    set_in_config: bool,
    path: Option<PathBuf>,
}

impl Render for Status {
    fn human(&self, out: &Output) {
        let path = self
            .path
            .as_ref()
            .map_or_else(|| "config.toml".to_string(), |p| p.display().to_string());
        out.line(&match (self.enabled, self.set_in_config) {
            (true, false) => "feedback reports are on (the default)".to_string(),
            (true, true) => {
                format!("feedback reports are on ('[feedback] enabled = true' in {path})")
            }
            (false, _) => {
                format!("feedback reports are off ('[feedback] enabled = false' in {path})")
            }
        });
        out.note(if self.enabled {
            "'sweetpad feedback off' turns them off"
        } else {
            "'sweetpad feedback on' turns them back on"
        });
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.enabled,
            "setInConfig": self.set_in_config,
            "configPath": self.path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = "\
## 2026-09-27T10:00Z · bug · medium
- **Seen:** 2× (first 2026-09-20T08:00Z, last 2026-09-27T10:00Z)
  - 2026-09-27 · sweetpad 0.1.10 · ~/Developer/Other — same failure
- **Context:** iOS app in a workspace; the user asked to run it on a simulator
- **Command:** `sweetpad run --on <simulator> --no-logs`
- **Expected:** the app launches and the command returns
- **Actual:** exit 1, `error: couldn't find the built app`
  and a second line of output
- **Assumption or gap:** the build succeeded, so the app should be where it wrote it
- **Fix idea:** look under the build's -derivedDataPath
";

    fn host() -> Host {
        Host {
            sweetpad: "0.1.10".to_string(),
            xcode: Some("26.0.1 (17A400)".to_string()),
            macos_version: Some("27.0".to_string()),
            macos_build: Some("26A428".to_string()),
            arch: "arm64".to_string(),
        }
    }

    fn endpoint() -> Endpoint {
        Endpoint::from_dsn(SENTRY_DSN).unwrap()
    }

    fn is_uuid_v4_hex(id: &str) -> bool {
        id.len() == 32
            && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            && id.as_bytes()[12] == b'4'
            && matches!(id.as_bytes()[16], b'8' | b'9' | b'a' | b'b')
    }

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
        let proxy = SendProxy::parse("HTTPS_PROXY", "http://me:secret@proxy.corp:3128/").unwrap();
        assert_eq!(proxy.shown, "proxy.corp:3128");
        assert!(proxy.has_credentials);
        let proxy = SendProxy::parse("https_proxy", "proxy.corp:3128").unwrap();
        assert_eq!(proxy.shown, "proxy.corp:3128");
        assert!(!proxy.has_credentials);

        for value in [
            "socks5://me:secret@proxy.corp:1080",
            "https://proxy.corp",
            "proxy:port",
        ] {
            let Err(err) = SendProxy::parse("HTTPS_PROXY", value) else {
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
    fn an_event_id_is_shaped_as_a_uuid_v4() {
        assert_eq!(
            uuid_v4_shaped("0123456789abcdef0123456789abcdef"),
            "0123456789ab4def8123456789abcdef"
        );
        assert_eq!(
            uuid_v4_shaped("ffffffffffffffffffffffffffffffff"),
            "ffffffffffff4fffbfffffffffffffff"
        );
    }

    #[test]
    fn sha256_matches_the_published_test_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn the_dsn_names_the_extensions_envelope_endpoint() {
        let e = endpoint();
        assert_eq!(
            e.url,
            "https://o325723.ingest.us.sentry.io/api/4507950563328000/envelope/"
        );
        assert_eq!(e.host, "o325723.ingest.us.sentry.io");
        assert_eq!(e.key, "5f6c5f5f15492a077f6bac254c6d8461");
        assert_eq!(
            host_of("http://127.0.0.1:4000/api/1/envelope/"),
            "127.0.0.1:4000"
        );
    }

    #[test]
    fn a_log_entry_parses_into_the_report_fields() {
        let report = Report::parse(ENTRY).unwrap();
        assert_eq!(report.kind, "bug");
        assert_eq!(report.severity, "medium");
        let labels: Vec<&str> = report.fields.iter().map(|(f, _)| f.label()).collect();
        assert_eq!(
            labels,
            [
                "Context",
                "Command",
                "Expected",
                "Actual",
                "Assumption or gap",
                "Fix idea"
            ]
        );
        // A value runs to the next field, wrapped lines included.
        assert_eq!(
            report.fields[3].1,
            "exit 1, `error: couldn't find the built app`\n  and a second line of output"
        );

        // The heading's timestamp and the Seen line, sub-bullet and all, stay
        // out of what is sent.
        let message = report.message();
        assert!(
            message.starts_with("bug · medium\n\nContext: iOS app"),
            "{message}"
        );
        for local in ["2026-09-27T10:00Z", "Seen", "2×", "~/Developer/Other"] {
            assert!(!message.contains(local), "{local} leaked into:\n{message}");
        }
    }

    #[test]
    fn the_heading_timestamp_is_optional_and_either_bold_form_is_a_field() {
        let report = Report::parse(
            "## gap · low\n- **Context**: c\n- **Command:** n/a\n- **Expected:** e\n\
             - **Actual:** a\n- **Assumption or gap:** g\n",
        )
        .unwrap();
        assert_eq!(
            (report.kind.as_str(), report.severity.as_str()),
            ("gap", "low")
        );
        assert_eq!(report.fields[0], (Field::Context, "c".to_string()));
        // 'Fix idea' is optional.
        assert_eq!(report.fields.len(), 5);
    }

    #[test]
    fn a_report_missing_fields_names_every_one() {
        let err = Report::parse("## bug · high\n- **Context:** c\n- **Expected:**\n").unwrap_err();
        assert!(
            err.contains("missing 'Command', 'Expected', 'Actual' and 'Assumption or gap'"),
            "{err}"
        );
    }

    #[test]
    fn malformed_reports_are_refused_in_words() {
        let fields = "- **Context:** c\n- **Command:** n/a\n- **Expected:** e\n\
                      - **Actual:** a\n- **Assumption or gap:** g\n";
        for (text, expected) in [
            (String::new(), "empty"),
            ("Context: c\n".to_string(), "must start with the heading"),
            ("## bug\n".to_string(), "must start with the heading"),
            (format!("## t · crash · high\n{fields}"), "kind is 'crash'"),
            (
                format!("## t · resolved · low\n{fields}"),
                "about the local log",
            ),
            (
                format!("## t · bug · urgent\n{fields}"),
                "severity is 'urgent'",
            ),
            (
                format!("## t · bug · low\nstray text\n{fields}"),
                "line 2 comes before",
            ),
            (
                format!("## t · bug · low\n{fields}- **Project:** App\n"),
                "unknown field 'Project'",
            ),
            (
                format!("## t · bug · low\n{fields}- **Actual:** again\n"),
                "'Actual' a second time",
            ),
            (
                format!("## t · bug · low\n{fields}## t · gap · low\n"),
                "second report",
            ),
        ] {
            let err = Report::parse(&text).expect_err(&text);
            assert!(err.contains(expected), "{text:?}: {err}");
        }
    }

    #[test]
    fn a_message_over_sentrys_limit_is_refused() {
        let long = "x".repeat(MAX_MESSAGE_CHARS);
        let text = format!(
            "## bug · low\n- **Context:** {long}\n- **Command:** n/a\n- **Expected:** e\n\
             - **Actual:** a\n- **Assumption or gap:** g\n"
        );
        let report = Report::parse(&text).unwrap();
        let err = Payload::build(&report, &host(), &endpoint()).unwrap_err();
        assert!(err.contains("Sentry keeps 4096"), "{err}");
    }

    #[test]
    fn the_payload_carries_the_report_and_versions_and_no_address() {
        let payload = Payload::build(&Report::parse(ENTRY).unwrap(), &host(), &endpoint()).unwrap();
        let event = &payload.event;
        assert_eq!(event["platform"], "other");
        assert_eq!(event["release"], "sweetpad-cli@0.1.10");
        assert_eq!(event["environment"], "production");
        assert_eq!(event["sdk"]["settings"]["infer_ip"], "never");
        assert_eq!(event["tags"]["source"], "cli");
        assert_eq!(event["tags"]["kind"], "bug");
        assert_eq!(event["tags"]["xcode"], "26.0.1 (17A400)");
        assert_eq!(event["contexts"]["os"]["version"], "27.0");
        assert_eq!(event["contexts"]["feedback"]["message"], payload.message);
        assert_eq!(event["event_id"], payload.event_id);
        assert!(event.get("user").is_none());
        assert!(event.get("timestamp").is_none());
        let text = event.to_string();
        assert!(
            !text.contains("ip_address") && !text.contains("{{auto}}"),
            "{text}"
        );
    }

    #[test]
    fn the_digest_is_stable_and_covers_the_payload_and_endpoint() {
        let report = Report::parse(ENTRY).unwrap();
        let a = Payload::build(&report, &host(), &endpoint()).unwrap();
        let b = Payload::build(&report, &host(), &endpoint()).unwrap();
        assert_eq!(a.digest, b.digest);
        assert_eq!(a.digest.len(), DIGEST_HEX_DIGITS);
        assert_eq!(a.event_id, b.event_id);
        assert!(is_uuid_v4_hex(&a.event_id), "{}", a.event_id);

        let mut edited = report.clone();
        edited.fields[0].1.push('.');
        let newer_xcode = Host {
            xcode: Some("26.1 (17B100)".to_string()),
            ..host()
        };
        let elsewhere = Endpoint {
            url: "http://127.0.0.1:1/api/1/envelope/".to_string(),
            ..endpoint()
        };
        for other in [
            Payload::build(&edited, &host(), &endpoint()).unwrap(),
            Payload::build(&report, &newer_xcode, &endpoint()).unwrap(),
            Payload::build(&report, &host(), &elsewhere).unwrap(),
        ] {
            assert_ne!(other.digest, a.digest);
        }
    }

    #[test]
    fn the_envelope_is_a_header_a_feedback_item_and_the_event() {
        let payload = Payload::build(&Report::parse(ENTRY).unwrap(), &host(), &endpoint()).unwrap();
        let envelope = payload.envelope(1_790_000_000);
        let lines: Vec<&str> = envelope.lines().collect();
        assert_eq!(lines.len(), 3, "{envelope}");
        let header: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        let item: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        let event: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(header["event_id"], payload.event_id);
        assert_eq!(item["type"], "feedback");
        assert_eq!(item["length"], lines[2].len());
        assert_eq!(event["timestamp"], 1_790_000_000);
        let mut without_time = event.clone();
        without_time.as_object_mut().unwrap().remove("timestamp");
        assert_eq!(without_time, payload.event);
    }

    #[test]
    fn a_dev_build_reports_as_development() {
        let dev = Host {
            sweetpad: "0.1.10-dev+c6bdf0a0".to_string(),
            ..host()
        };
        let payload = Payload::build(&Report::parse(ENTRY).unwrap(), &dev, &endpoint()).unwrap();
        assert_eq!(payload.event["environment"], "development");
        assert_eq!(payload.event["release"], "sweetpad-cli@0.1.10-dev+c6bdf0a0");
    }

    #[test]
    fn this_macs_versions_read() {
        let host = Host::detect();
        assert!(host.macos_version.is_some(), "{host:?}");
        assert!(
            ["arm64", "x86_64"].contains(&host.arch.as_str()),
            "{host:?}"
        );
    }
}
