//! Picking an Xcode archive from Apple's catalog and downloading it, with
//! resume, a receipt that vouches for a finished file, and a lock against a
//! second download of the same file.

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{Value, json};

use super::http::Call;
use super::session::{Session, euid, private_directory, private_read, private_write};
use super::{LOGIN_HINT, url};

/// A version as Apple names releases: `27.2`, `27.3beta`, `27.3beta2`,
/// `27.3rc`. Trailing `.0`s go, and `beta1` and `rc1` are `beta` and `rc`.
pub(crate) fn normalize_version(value: &str) -> Result<String, String> {
    let invalid = || "Give an exact Xcode version, such as 27.2 or '27.3 beta'.".to_string();
    let separator = |c: char| c.is_whitespace() || matches!(c, '_' | '-');
    let mut value = value.trim();
    if value.len() > 5
        && value[..5].eq_ignore_ascii_case("xcode")
        && value[5..].starts_with(separator)
    {
        value = value[5..].trim_start_matches(separator);
    }
    let value = value
        .to_lowercase()
        .replace(separator, "")
        .replace("releasecandidate", "rc");
    let split = value
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(value.len());
    let (numbers, suffix) = value.split_at(split);
    let parts: Vec<&str> = numbers.split('.').collect();
    if parts.len() > 3 || parts.iter().any(|p| p.is_empty()) {
        return Err(invalid());
    }
    let mut numbers = parts
        .iter()
        .map(|p| p.parse::<u64>().map_err(|_| invalid()))
        .collect::<Result<Vec<_>, _>>()?;
    let label = suffix
        .strip_prefix("beta")
        .map(|n| ("beta", n))
        .or_else(|| suffix.strip_prefix("rc").map(|n| ("rc", n)));
    let suffix = match label {
        _ if suffix.is_empty() => String::new(),
        Some((label, n)) if n.bytes().all(|b| b.is_ascii_digit()) => {
            if n == "1" {
                label.to_string()
            } else {
                format!("{label}{n}")
            }
        }
        _ => return Err(invalid()),
    };
    while numbers.len() > 1 && numbers.last() == Some(&0) {
        numbers.pop();
    }
    let numbers: Vec<String> = numbers.iter().map(u64::to_string).collect();
    Ok(numbers.join(".") + &suffix)
}

/// A catalog path, checked to be an Xcode archive under `/Developer_Tools/`
/// and never a URL or a path that climbs out of it.
pub(crate) fn archive_path(value: &str) -> Result<String, String> {
    let path = url::decode(value);
    let extension = extension(&path);
    if !path.starts_with("/Developer_Tools/")
        || path.contains(['\\', '?', '#'])
        || path.split('/').any(|piece| matches!(piece, "." | ".."))
        || path.chars().any(|c| u32::from(c) < 32)
        || !matches!(extension.as_str(), "xip" | "aea")
    {
        return Err("Apple returned an Xcode archive path sweetpad can't use.".into());
    }
    Ok(path)
}

/// The lower-case extension of the last path component.
fn extension(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or_default();
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[dot + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// The Xcode archives linked from Apple's `/download/` page, in the
/// catalog's shape. Apple lists a new beta there before the catalog has it.
pub(crate) fn prerelease_links(html: &str) -> Result<Vec<Value>, String> {
    let mut downloads = Vec::new();
    for attributes in anchors(html) {
        let class = attributes
            .iter()
            .find(|(n, _)| n == "class")
            .map(|(_, v)| v.as_str());
        if !class.is_some_and(|c| c.split_whitespace().any(|c| c == "direct-download")) {
            continue;
        }
        let href = attributes
            .iter()
            .find(|(n, _)| n == "href")
            .map_or("", |(_, v)| v);
        let target = url::join("https://developer.apple.com/download/", href);
        if !url::apple(&target) {
            continue;
        }
        let Some(parsed) = url::parse(&target) else {
            continue;
        };
        let path = parsed
            .query
            .and_then(|q| url::query_value(q, "path"))
            .unwrap_or_else(|| parsed.path.to_string());
        let filename = path.rsplit('/').next().unwrap_or_default();
        if !xcode_file_name(filename) {
            continue;
        }
        let path = archive_path(&path)?;
        let stem = &filename[..filename.rfind('.').unwrap_or(filename.len())];
        downloads.push(json!({
            "name": stem.replace('_', " "),
            "files": [{"remotePath": path}],
        }));
    }
    Ok(downloads)
}

/// Whether `name` is `Xcode_<digit>...` with an `.aea` or `.xip` extension.
fn xcode_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("xcode_") else {
        return false;
    };
    let Some(middle) = rest
        .strip_suffix(".aea")
        .or_else(|| rest.strip_suffix(".xip"))
    else {
        return false;
    };
    middle.starts_with(|c: char| c.is_ascii_digit())
        && middle
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ' ' | '-'))
}

/// The attributes of each `<a>` tag, names in lower case and values with
/// character references decoded.
fn anchors(html: &str) -> Vec<Vec<(String, String)>> {
    let mut found = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while let Some(start) = lower[at..].find("<a") {
        let begin = at + start + 2;
        at = begin;
        if !lower[begin..].starts_with(|c: char| c.is_ascii_whitespace()) {
            continue;
        }
        let mut attributes = Vec::new();
        let bytes = html.as_bytes();
        let mut i = begin;
        loop {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] == b'>' {
                break;
            }
            let name_start = i;
            while i < bytes.len()
                && !matches!(bytes[i], b'=' | b'>' | b'/')
                && !bytes[i].is_ascii_whitespace()
            {
                i += 1;
            }
            let name = html[name_start..i].to_ascii_lowercase();
            if i < bytes.len() && bytes[i] == b'/' {
                i += 1;
            }
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let mut value = String::new();
            if i < bytes.len() && bytes[i] == b'=' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i < bytes.len() && matches!(bytes[i], b'"' | b'\'') {
                    let quote = bytes[i];
                    let value_start = i + 1;
                    i = value_start;
                    while i < bytes.len() && bytes[i] != quote {
                        i += 1;
                    }
                    value = unescape(&html[value_start..i.min(bytes.len())]);
                    i += 1;
                } else {
                    let value_start = i;
                    while i < bytes.len() && bytes[i] != b'>' && !bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    value = unescape(&html[value_start..i]);
                }
            }
            if !name.is_empty() {
                attributes.push((name, value));
            }
        }
        at = i.min(html.len());
        found.push(attributes);
    }
    found
}

/// HTML character references in an attribute value.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let entity = &rest[1..end];
            let c = match entity {
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "lt" => Some('<'),
                "gt" => Some('>'),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .map(|hex| u32::from_str_radix(hex, 16))
                    .or_else(|| entity.strip_prefix('#').map(str::parse))
                    .and_then(Result::ok)
                    .and_then(char::from_u32),
            }?;
            Some((c, end))
        });
        if let Some((c, end)) = decoded {
            out.push(c);
            rest = &rest[end + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// The archive picked for a version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Release {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) size: Option<u64>,
}

/// The one archive in `downloads` for `version`. A beta never stands in for
/// a release, and `.aea` wins over `.xip` for the same release.
pub(crate) fn select_release(downloads: &[Value], version: &str) -> Result<Release, String> {
    let selected = normalize_version(version)?;
    let mut choices: Vec<Release> = Vec::new();
    for item in downloads {
        let Some(name) = item["name"].as_str() else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let Some(after) = lower.strip_prefix("xcode") else {
            continue;
        };
        let trimmed = after.trim_start();
        if trimmed.len() == after.len() || !trimmed.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        if normalize_version(name).ok().as_deref() != Some(selected.as_str()) {
            continue;
        }
        for entry in item["files"].as_array().into_iter().flatten() {
            let Some(raw) = entry["remotePath"].as_str() else {
                continue;
            };
            if !matches!(extension(raw).as_str(), "xip" | "aea") {
                continue;
            }
            let path = archive_path(&format!("/{}", raw.trim_start_matches('/')))?;
            let size = match &entry["fileSize"] {
                Value::Null => None,
                value => Some(
                    value
                        .as_u64()
                        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
                        .filter(|size| *size > 0)
                        .ok_or("Apple returned an invalid archive size.")?,
                ),
            };
            let release = Release {
                name: name.to_string(),
                path: path.clone(),
                size,
            };
            match choices.iter_mut().find(|c| c.path == path) {
                Some(existing) => *existing = release,
                None => choices.push(release),
            }
        }
    }
    if choices.is_empty() {
        return Err(format!(
            "Xcode {version} isn't in Apple's downloads. Use its exact release or beta version."
        ));
    }
    let modern: Vec<Release> = choices
        .iter()
        .filter(|c| c.path.to_ascii_lowercase().ends_with(".aea"))
        .cloned()
        .collect();
    let mut choices = if modern.is_empty() { choices } else { modern };
    if choices.len() != 1 {
        return Err(
            "Several Apple downloads match this version. Name a more specific beta or \
                    release candidate."
                .into(),
        );
    }
    Ok(choices.remove(0))
}

/// `$XDG_CACHE_HOME/sweetpad/downloads/<filename>`.
pub(crate) fn default_destination(filename: &str) -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        .unwrap_or_else(|| home.join(".cache"));
    cache.join("sweetpad/downloads").join(filename)
}

fn digest_file(path: &Path) -> Result<String, String> {
    crate::cli::sdk_import::sha256_file(path)
}

/// `path`'s size, when it is a regular file the user owns with mode 600.
fn regular_owned(path: &Path) -> Result<u64, String> {
    let info = fs::symlink_metadata(path)
        .map_err(|error| format!("Couldn't read {}: {error}.", path.display()))?;
    if !info.is_file() || info.uid() != euid() || info.mode() & 0o077 != 0 {
        return Err(format!(
            "{} must be a regular file you own, with mode 600.",
            path.display()
        ));
    }
    Ok(info.len())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Hold an exclusive lock on `path` while the returned file is open.
fn lock(path: &Path) -> Result<fs::File, String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| format!("Couldn't open {}: {error}.", path.display()))?;
    regular_owned(path)?;
    // SAFETY: flock on a descriptor this function owns.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("Another process is downloading this Xcode archive.".into());
    }
    Ok(file)
}

pub(crate) struct Options<'a> {
    /// Write the session back once Apple authorizes the download, which a
    /// read-only CI session file must not be.
    pub(crate) persist_session: bool,
    pub(crate) free_space: &'a dyn Fn(&Path) -> Result<u64, String>,
    pub(crate) note: &'a mut dyn FnMut(&str),
}

/// Download `release` to `destination`, or reuse it there when its receipt
/// vouches for it. A `.part` file resumes where an earlier run stopped.
#[allow(clippy::too_many_lines)] // one transfer, start to finish
pub(crate) fn download(
    session: &mut Session,
    release: &Release,
    destination: &Path,
    options: Options<'_>,
) -> Result<Value, String> {
    let Options {
        persist_session,
        free_space,
        note,
    } = options;
    let parent = destination.parent().unwrap_or(Path::new("."));
    private_directory(parent)?;
    let path = &release.path;
    let mut target = format!(
        "https://download.developer.apple.com{}",
        url::encode(path, "/")
    );
    let receipt = sibling(destination, ".sweetpad-download.json");
    let partial = sibling(destination, ".part");
    let state_path = sibling(&partial, ".json");
    let _lock = lock(&sibling(destination, ".lock"))?;

    if fs::symlink_metadata(destination).is_ok() {
        let size = regular_owned(destination)?;
        let unverified = || {
            format!(
                "{} isn't a download sweetpad can vouch for. Choose another --output-file.",
                destination.display()
            )
        };
        let previous: Value = private_read(&receipt)?
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .ok_or_else(unverified)?;
        let digest = digest_file(destination)?;
        if previous["sourcePath"].as_str() != Some(path)
            || previous["sha256"].as_str() != Some(&digest)
            || previous["archiveBytes"].as_u64() != Some(size)
        {
            return Err(format!(
                "{} no longer matches its download receipt. Choose another --output-file.",
                destination.display()
            ));
        }
        let mut result = previous;
        result["downloaded"] = json!(true);
        result["reused"] = json!(true);
        result["archive"] = json!(destination.display().to_string());
        return Ok(result);
    }

    let authorization = format!(
        "https://developer.apple.com/services-account/download?{}",
        url::form(&[("path", path)])
    );
    let response = session.client.open(
        Call::get(&authorization)
            .header("Accept", "*/*")
            .no_follow(),
    )?;
    match response.status {
        401 | 403 => {
            return Err(format!(
                "Apple's download authorization has expired. {LOGIN_HINT}"
            ));
        }
        200 => {}
        302 | 303 | 307 => {
            let redirect = url::join(&authorization, response.header("location").unwrap_or(""));
            let authorized = url::apple(&redirect)
                && url::parse(&redirect).is_some_and(|u| {
                    u.host == "download.developer.apple.com" && url::decode(u.path) == *path
                });
            if !authorized {
                return Err(format!(
                    "Apple didn't authorize the selected Xcode archive. {LOGIN_HINT}"
                ));
            }
            target = redirect;
        }
        code => {
            return Err(format!(
                "Apple's download authorization failed (HTTP {code})."
            ));
        }
    }
    drop(response);
    if persist_session {
        session.save()?;
    }

    let mut offset = 0;
    let mut state = Value::Null;
    if fs::symlink_metadata(&partial).is_ok() {
        let size = regular_owned(&partial)?;
        state = private_read(&state_path)?
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .ok_or_else(|| {
                format!(
                    "The partial download has no record of where it came from. Remove {} \
                     and run the command again.",
                    partial.display()
                )
            })?;
        if state["sourcePath"].as_str() != Some(path) {
            return Err(format!(
                "{} belongs to another Xcode archive.",
                partial.display()
            ));
        }
        offset = size;
        let expected = state["expectedTotal"].as_u64().or(release.size);
        if Some(offset) == expected {
            // The run stopped after the last byte but before the rename.
            // Fetch the tail again rather than ask for a range past the end.
            offset = offset.saturating_sub(1024 * 1024);
        }
    }
    let validator = state["validator"].as_str().filter(|v| !v.is_empty());
    let mut call = Call::get(&target)
        .header("Accept", "*/*")
        .header("Accept-Encoding", "identity")
        .timeout(24 * 3600);
    match validator {
        Some(validator) if offset > 0 => {
            call = call
                .header("Range", &format!("bytes={offset}-"))
                .header("If-Range", validator);
        }
        _ => offset = 0,
    }
    let mut response = session.client.open(call)?;
    match response.status {
        401 | 403 => return Err(format!("Apple refused the archive download. {LOGIN_HINT}")),
        200 | 206 => {}
        code => {
            return Err(format!(
                "Apple's archive download failed (HTTP {code}). Run the same command again to \
                 resume."
            ));
        }
    }
    if response
        .header("content-type")
        .is_some_and(|t| t.to_ascii_lowercase().contains("text/"))
    {
        return Err(format!(
            "Apple returned a sign-in page instead of the archive. {LOGIN_HINT}"
        ));
    }
    let length = match response.header("content-length") {
        None => None,
        Some(value) => Some(
            value
                .trim()
                .parse::<u64>()
                .map_err(|_| "Apple returned an invalid download length.")?,
        ),
    };
    let total = if response.status == 206 {
        let range = response
            .header("content-range")
            .and_then(content_range)
            .filter(|(start, end, total)| {
                offset > 0
                    && *start == offset
                    && end + 1 == *total
                    && length.is_none_or(|l| l == total - offset)
            });
        let Some((_, _, total)) = range else {
            return Err(format!(
                "Apple returned an invalid download range. Remove {} and run the command again.",
                partial.display()
            ));
        };
        let validator_header = if validator.is_some_and(|v| v.starts_with('"')) {
            "etag"
        } else {
            "last-modified"
        };
        if let Some(current) = response.header(validator_header)
            && Some(current) != validator
        {
            return Err(format!(
                "The archive changed on Apple's side during the download. Remove {} and run \
                 the command again.",
                partial.display()
            ));
        }
        Some(total)
    } else {
        offset = 0;
        length
    };
    if let (Some(expected), Some(total)) = (release.size, total)
        && expected != total
    {
        return Err(
            "The archive's length differs from Apple's catalog. Try again once the \
                    catalog updates."
                .into(),
        );
    }
    let total = total.or(release.size);
    if let Some(total) = total
        && free_space(parent)? < total - offset + 64 * 1024 * 1024
    {
        return Err(format!(
            "There isn't enough free space in {} for the Xcode archive. Choose another \
             --output-file.",
            parent.display()
        ));
    }
    let validator = response
        .header("etag")
        .filter(|v| !v.starts_with("W/"))
        .or_else(|| response.header("last-modified"))
        .map(str::to_string);
    private_write(
        &state_path,
        &json!({"sourcePath": path, "validator": validator, "expectedTotal": total}),
    )?;
    let failed = |error: std::io::Error| format!("Couldn't write {}: {error}.", partial.display());
    let mut output = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&partial)
        .map_err(failed)?;
    output
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(failed)?;
    if !output.metadata().map_err(failed)?.is_file() {
        return Err(format!("{} must be a regular file.", partial.display()));
    }
    output.seek(SeekFrom::Start(offset)).map_err(failed)?;
    output.set_len(offset).map_err(failed)?;
    note(&format!(
        "Downloading {} ({offset} bytes already here)",
        release.name
    ));
    let mut count = offset;
    let mut reported = Instant::now();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = match response.body.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                return Err(
                    "The download was interrupted. Run the same command again to resume.".into(),
                );
            }
        };
        count += read as u64;
        if total.is_some_and(|total| count > total) {
            return Err("Apple sent more archive data than expected.".into());
        }
        output.write_all(&buffer[..read]).map_err(failed)?;
        if reported.elapsed().as_secs() >= 10 {
            #[allow(clippy::cast_precision_loss)] // a progress figure
            let gib = count as f64 / f64::from(1u32 << 30);
            note(&format!("Downloaded {gib:.2} GiB"));
            reported = Instant::now();
        }
    }
    output.sync_all().map_err(failed)?;
    drop(output);
    if total.is_some_and(|total| count != total) {
        return Err("The download is incomplete. Run the same command again to resume.".into());
    }
    let mut magic = [0u8; 4];
    fs::File::open(&partial)
        .and_then(|mut file| file.read_exact(&mut magic))
        .map_err(|error| format!("Couldn't read {}: {error}.", partial.display()))?;
    let expected: &[u8; 4] = if path.to_ascii_lowercase().ends_with(".aea") {
        b"AEA1"
    } else {
        b"xar!"
    };
    if &magic != expected {
        return Err("The downloaded data isn't the expected Xcode archive format.".into());
    }
    let digest = digest_file(&partial)?;
    let result = json!({
        "downloaded": true,
        "reused": false,
        "archive": destination.display().to_string(),
        "xcodeVersion": release.name.strip_prefix("Xcode ").unwrap_or(&release.name),
        "sourcePath": path,
        "archiveBytes": count,
        "sha256": digest,
        "nextStep": "Import this archive with 'sweetpad sdk import', or run 'sweetpad setup --xcode VERSION'.",
    });
    private_write(&receipt, &result)?;
    fs::rename(&partial, destination).map_err(|error| {
        format!(
            "Couldn't move the archive to {}: {error}.",
            destination.display()
        )
    })?;
    let _ = fs::remove_file(&state_path);
    Ok(result)
}

/// `bytes <start>-<end>/<total>`.
fn content_range(value: &str) -> Option<(u64, u64, u64)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let number = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    Some((number(start)?, number(end)?, number(total)?))
}
