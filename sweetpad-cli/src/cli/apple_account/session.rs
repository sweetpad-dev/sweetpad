//! The session file, which holds the account name and Apple's cookies but
//! never a password, and the private files and folders it lives in.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::cookies::Cookie;
use super::http::{Call, Client, now};
use super::{CATALOG, LOGIN_HINT, url};

/// The largest session or receipt file read.
const PRIVATE_LIMIT: u64 = 1024 * 1024;

/// `explicit`, or `$XDG_STATE_HOME/sweetpad/apple-session.json`.
pub(crate) fn session_path(explicit: Option<&Path>) -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if let Some(path) = explicit {
        let path = match path.strip_prefix("~") {
            Ok(rest) => home.join(rest),
            Err(_) => path.to_path_buf(),
        };
        return std::path::absolute(&path).unwrap_or(path);
    }
    let root = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        .unwrap_or_else(|| home.join(".local/state"));
    root.join("sweetpad/apple-session.json")
}

/// Create `path` and its missing parents with mode 700, and check that it
/// is a folder the user owns and nobody else can write to.
pub(crate) fn private_directory(path: &Path) -> Result<(), String> {
    let mut missing = Vec::new();
    let mut candidate = path;
    while fs::symlink_metadata(candidate).is_err() {
        missing.push(candidate);
        match candidate.parent() {
            Some(parent) => candidate = parent,
            None => break,
        }
    }
    for directory in missing.iter().rev().chain(std::iter::once(&path)) {
        match fs::DirBuilder::new().mode(0o700).create(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("Couldn't create {}: {error}.", directory.display())),
        }
        let info = fs::symlink_metadata(directory)
            .map_err(|error| format!("Couldn't read {}: {error}.", directory.display()))?;
        if !info.is_dir() || info.uid() != euid() || info.mode() & 0o022 != 0 {
            return Err(format!(
                "{} must be a folder you own that other users can't write to.",
                directory.display()
            ));
        }
    }
    Ok(())
}

/// A private file's contents, or `None` when it does not exist. It must be
/// a regular file the user owns with no group or other permissions; a
/// symbolic link is refused.
pub(crate) fn private_read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path);
    let mut file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
            return Err(format!("{} must not be a symbolic link.", path.display()));
        }
        Err(error) => return Err(format!("Couldn't read {}: {error}.", path.display())),
    };
    let info = file
        .metadata()
        .map_err(|error| format!("Couldn't read {}: {error}.", path.display()))?;
    if !info.is_file()
        || info.uid() != euid()
        || info.mode() & 0o077 != 0
        || info.len() > PRIVATE_LIMIT
    {
        return Err(format!(
            "{} must be a regular file you own, with mode 600.",
            path.display()
        ));
    }
    let mut data = Vec::new();
    Read::by_ref(&mut file)
        .take(PRIVATE_LIMIT + 1)
        .read_to_end(&mut data)
        .map_err(|error| format!("Couldn't read {}: {error}.", path.display()))?;
    Ok(Some(data))
}

/// Replace `path` with `value` as JSON, mode 600, through a temporary file
/// so a reader never sees half of it.
pub(crate) fn private_write(path: &Path, value: &Value) -> Result<(), String> {
    let parent = path.parent().unwrap_or(Path::new("."));
    private_directory(parent)?;
    if fs::symlink_metadata(path).is_ok_and(|info| info.file_type().is_symlink()) {
        return Err(format!("{} must not be a symbolic link.", path.display()));
    }
    let failed = |error: std::io::Error| format!("Couldn't write {}: {error}.", path.display());
    let mut random = [0u8; 8];
    super::srp::random_bytes(&mut random)?;
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let temporary = parent.join(format!(
        ".{name}.{}",
        crate::cli::apple_archive::hex(&random)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(value.to_string().as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(failed)
}

pub(crate) fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

pub(crate) struct Session {
    pub(crate) path: PathBuf,
    pub(crate) apple_id: Option<String>,
    pub(crate) client: Client,
}

impl Session {
    pub(crate) fn new(path: PathBuf, client: Client) -> Self {
        Self {
            path,
            apple_id: None,
            client,
        }
    }

    /// Read the session file into the cookie jar. `false` when there is
    /// none and `required` is not set.
    pub(crate) fn load(&mut self, required: bool) -> Result<bool, String> {
        let Some(raw) = private_read(&self.path)? else {
            if required {
                return Err(format!("There is no Apple session file. {LOGIN_HINT}"));
            }
            return Ok(false);
        };
        let data: Value = serde_json::from_slice(&raw)
            .map_err(|_| "The Apple session file is not a sweetpad session.".to_string())?;
        let (Some(1), Some(apple_id), Some(items)) = (
            data["version"].as_i64(),
            data["appleId"].as_str(),
            data["cookies"].as_array(),
        ) else {
            return Err(
                "sweetpad can't read this Apple session file. Create it with 'sweetpad auth login'."
                    .into(),
            );
        };
        let cookies = items
            .iter()
            .map(stored_cookie)
            .collect::<Option<Vec<_>>>()
            .ok_or("The Apple session file holds a cookie sweetpad can't use.")?;
        self.apple_id = Some(apple_id.to_string());
        for cookie in cookies {
            self.client.cookies.set(cookie);
        }
        Ok(true)
    }

    /// Write the account name and the unexpired Apple cookies.
    pub(crate) fn save(&self) -> Result<(), String> {
        let now = now();
        let cookies: Vec<Value> = self
            .client
            .cookies
            .iter()
            .filter(|c| apple_domain(&c.domain) && !c.expired(now))
            .map(|c| {
                json!({
                    "name": c.name,
                    "value": c.value,
                    "domain": c.domain,
                    "domainSpecified": c.domain_specified,
                    "path": c.path,
                    "expires": c.expires,
                })
            })
            .collect();
        private_write(
            &self.path,
            &json!({"version": 1, "appleId": self.apple_id, "cookies": cookies}),
        )
    }

    /// The downloads the account can see. Fails when the session has expired.
    pub(crate) fn catalog(&mut self) -> Result<Vec<Value>, String> {
        let (code, _, data) = self.client.request(Call::new("POST", CATALOG))?;
        let object = data.as_ref().and_then(Value::as_object);
        if matches!(code, 401 | 403) || (code == 200 && object.is_none()) {
            let detail = if code == 200 { ", not JSON" } else { "" };
            return Err(format!(
                "The Apple Developer session is unavailable (HTTP {code}{detail}). {LOGIN_HINT}"
            ));
        }
        let Some(object) = object.filter(|_| code == 200) else {
            return Err(format!("Apple's download catalog failed (HTTP {code})."));
        };
        let result = object
            .get("resultCode")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        match object.get("downloads").and_then(Value::as_array) {
            Some(downloads) if result == 0 => Ok(downloads.clone()),
            _ => Err(
                "Apple refused download access. Sign in again, and accept the Apple \
                      Developer agreements in a browser."
                    .into(),
            ),
        }
    }
}

fn apple_domain(domain: &str) -> bool {
    url::apple(&format!("https://{}/", domain.trim_start_matches('.')))
}

/// A cookie as the session file stores it, or `None` when it is malformed
/// or not for Apple.
fn stored_cookie(item: &Value) -> Option<Cookie> {
    let domain = item["domain"].as_str()?;
    let name = item["name"].as_str()?;
    let value = item["value"].as_str()?;
    let path = item["path"].as_str()?;
    let expires = match &item["expires"] {
        Value::Null => None,
        other => Some(other.as_i64()?),
    };
    let unsafe_text = |text: &str| text.contains(['\r', '\n']);
    if !path.starts_with('/') || unsafe_text(name) || unsafe_text(value) || !apple_domain(domain) {
        return None;
    }
    Some(Cookie {
        name: name.to_string(),
        value: value.to_string(),
        domain: domain.to_string(),
        domain_specified: item["domainSpecified"].as_bool()?,
        path: path.to_string(),
        expires,
    })
}
