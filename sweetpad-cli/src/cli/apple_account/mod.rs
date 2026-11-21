//! Apple Developer sign-in and Xcode downloads from a terminal, without
//! Apple's frameworks.
//!
//! The protocol follows XcodesOrg/XcodesLoginKit (`Client`,
//! `URLRequest+Apple`, `AppleServiceKeyResolver`) and XcodesOrg/swift-srp.
//! The session file holds the account name and Apple's cookies, never a
//! password. Errors are sentences that carry no credential or response body.

mod cookies;
mod download;
mod http;
mod login;
mod session;
mod srp;
#[cfg(test)]
mod tests;
mod tty;
mod url;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

pub(crate) use download::normalize_version;
use download::{Options, prerelease_links, select_release};
use http::{Call, Client, read_body};
pub(crate) use session::session_path;
use session::{Session, private_read};

const AUTH: &str = "https://idmsa.apple.com/appleauth/auth";
const CATALOG: &str =
    "https://developer.apple.com/services-account/QH65B2/downloadws/listDownloads.action";
const LOGIN_HINT: &str =
    "Run 'sweetpad auth login --apple-id EMAIL', or pass --session-file in CI.";

/// Sign in as `apple_id`, or keep the saved session when it is still good.
pub(crate) fn login(
    path: PathBuf,
    apple_id: &str,
    interactive: bool,
    note: &mut dyn FnMut(&str),
) -> Result<Value, String> {
    let mut session = Session::new(path, Client::network());
    login_with(&mut session, apple_id, interactive, &mut tty::ask, note)
}

fn login_with(
    session: &mut Session,
    apple_id: &str,
    interactive: bool,
    prompt: login::Prompt<'_>,
    note: &mut dyn FnMut(&str),
) -> Result<Value, String> {
    if !apple_id.contains('@') || apple_id.chars().any(char::is_whitespace) {
        return Err("Give the account's email address with --apple-id.".into());
    }
    let same_account = session.load(false)?
        && session
            .apple_id
            .as_deref()
            .is_some_and(|saved| saved.to_lowercase() == apple_id.to_lowercase());
    let reused = same_account && session.catalog().is_ok();
    if !reused {
        if !interactive {
            return Err(format!(
                "There is no valid Apple session, and signing in needs a terminal. {LOGIN_HINT}"
            ));
        }
        // The saved file stays as it is until the new sign-in succeeds.
        session.client.cookies.clear();
        login::Login::new(session, prompt, note).login(apple_id)?;
    }
    Ok(json!({
        "authenticated": true,
        "reused": reused,
        "appleId": session.apple_id,
        "sessionFile": session.path.display().to_string(),
    }))
}

/// Delete the session file.
pub(crate) fn logout(path: &Path) -> Result<Value, String> {
    if std::fs::symlink_metadata(path).is_ok() {
        // Refuse to delete something that is not a session file.
        private_read(path)?;
        std::fs::remove_file(path)
            .map_err(|error| format!("Couldn't delete {}: {error}.", path.display()))?;
    }
    Ok(json!({"loggedOut": true, "sessionFile": path.display().to_string()}))
}

/// Download the archive of Xcode `version` with the saved session.
/// `explicit_session` marks a session file the caller named, which is left
/// as it is.
pub(crate) fn download_xcode(
    session_file: PathBuf,
    explicit_session: bool,
    version: &str,
    output: Option<&Path>,
    note: &mut dyn FnMut(&str),
) -> Result<Value, String> {
    let mut session = Session::new(session_file, Client::network());
    download_with(
        &mut session,
        explicit_session,
        version,
        output,
        &crate::cli::apple_archive::free_space,
        note,
    )
}

fn download_with(
    session: &mut Session,
    explicit_session: bool,
    version: &str,
    output: Option<&Path>,
    free_space: &dyn Fn(&Path) -> Result<u64, String>,
    note: &mut dyn FnMut(&str),
) -> Result<Value, String> {
    normalize_version(version)?;
    session.load(true)?;
    let releases = session.catalog()?;
    let release = if let Ok(release) = select_release(&releases, version) {
        release
    } else {
        // Apple can list a new beta on /download before the catalog has it.
        let mut page = session.client.open(
            Call::get("https://developer.apple.com/download/").header("Accept", "text/html"),
        )?;
        if page.status != 200 {
            return Err("Couldn't read Apple's prerelease downloads.".into());
        }
        let raw = read_body(&page.headers, &mut page.body, 8 * 1024 * 1024)?;
        let mut all = releases;
        all.extend(prerelease_links(&String::from_utf8_lossy(&raw))?);
        select_release(&all, version)?
    };
    let destination = match output {
        Some(path) => path.to_path_buf(),
        None => download::default_destination(release.path.rsplit('/').next().unwrap_or_default()),
    };
    let destination = std::path::absolute(&destination).unwrap_or(destination);
    download::download(
        session,
        &release,
        &destination,
        Options {
            persist_session: !explicit_session,
            free_space,
            note,
        },
    )
}
