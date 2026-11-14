//! Uploading a build with App Store Connect's Build Upload API, and following
//! it until Apple accepts or rejects it.
//!
//! An upload is a `buildUploads` record for the app, version, build number
//! and platform; a `buildUploadFiles` record for the file, whose answer lists
//! the byte ranges to PUT and where; then a PATCH marking the file uploaded.
//! Apple then checks the build (state `PROCESSING`) and ends in `COMPLETE`
//! with a build, or `FAILED` with the ITMS errors behind it.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::api::Client;
use super::artifact::{Artifact, Platform};
use crate::cli::net::Request;
use crate::cli::{CliError, ErrorKind};

/// Parts sent at once.
const PARALLEL_PARTS: usize = 4;

/// What happens while an upload runs, for the command to show.
pub(crate) enum Progress<'a> {
    Created {
        upload_id: &'a str,
        parts: usize,
        bytes: u64,
    },
    Part {
        done: usize,
        parts: usize,
    },
    Committed,
    State(&'a str),
    Processing(&'a str),
}

/// One message Apple attached to an upload's state.
#[derive(Debug, Clone)]
pub(crate) struct Message {
    pub(crate) code: String,
    pub(crate) description: String,
}

/// Where an upload stands.
#[derive(Debug, Clone)]
pub(crate) struct UploadReport {
    pub(crate) upload_id: String,
    /// The app and its bundle identifier, which an upload doesn't name:
    /// known when this run made the upload.
    pub(crate) app_id: Option<String>,
    pub(crate) bundle_id: Option<String>,
    pub(crate) platform: Option<Platform>,
    pub(crate) version: Option<String>,
    pub(crate) build: Option<String>,
    /// `AWAITING_UPLOAD`, `PROCESSING`, `COMPLETE` or `FAILED`.
    pub(crate) state: String,
    pub(crate) errors: Vec<Message>,
    pub(crate) warnings: Vec<Message>,
    pub(crate) infos: Vec<Message>,
    pub(crate) build_id: Option<String>,
    /// The build's own `processingState` once it exists: `PROCESSING`,
    /// `FAILED`, `INVALID` or `VALID`.
    pub(crate) processing_state: Option<String>,
}

impl UploadReport {
    pub(crate) fn failed(&self) -> bool {
        self.state == "FAILED"
            || matches!(self.processing_state.as_deref(), Some("FAILED" | "INVALID"))
    }
}

/// How long to follow an upload, and how far.
pub(crate) struct Follow {
    /// Also wait for the build's processing to end.
    pub(crate) processing: bool,
    pub(crate) timeout: Duration,
    pub(crate) interval: Duration,
}

/// The App Store Connect app id for `bundle_id`. `filter[bundleId]` also
/// matches prefixes, so the answer is checked for an exact match.
pub(crate) fn app_id(client: &Client, bundle_id: &str) -> Result<String, CliError> {
    let query = format!(
        "/v1/apps?filter%5BbundleId%5D={}&fields%5Bapps%5D=bundleId&limit=200",
        percent(bundle_id)
    );
    let apps = client.get(&query)?;
    apps["data"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|app| app["attributes"]["bundleId"].as_str() == Some(bundle_id))
        .and_then(|app| app["id"].as_str().map(str::to_string))
        .ok_or_else(|| {
            CliError::new(format!(
                "App Store Connect has no app with the bundle identifier {bundle_id} that this API \
                 key can see; create the app record first, or pass '--app-id'"
            ))
            .kind(ErrorKind::TargetResolution)
        })
}

/// Upload `artifact` to `app_id`, then follow it.
pub(crate) fn upload(
    client: &Client,
    artifact: &Artifact,
    app_id: &str,
    follow: &Follow,
    progress: &dyn Fn(Progress<'_>),
) -> Result<UploadReport, CliError> {
    let created = client
        .post(
            "/v1/buildUploads",
            &json!({"data": {
                "type": "buildUploads",
                "attributes": {
                    "cfBundleShortVersionString": artifact.version,
                    "cfBundleVersion": artifact.build,
                    "platform": artifact.platform.api(),
                },
                "relationships": {"app": {"data": {"type": "apps", "id": app_id}}},
            }}),
        )
        .map_err(|failure| {
            if failure.status == Some(409)
                && failure.codes.iter().any(|code| code.contains("DUPLICATE"))
            {
                return CliError::new(format!(
                    "build {} of version {} is already uploaded for {}; raise CFBundleVersion \
                     (CURRENT_PROJECT_VERSION) and build again",
                    artifact.build,
                    artifact.version,
                    artifact.platform.api()
                ))
                .context("App Store Connect refused the upload");
            }
            *failure.error
        })?;
    let upload_id = created["data"]["id"]
        .as_str()
        .ok_or_else(|| CliError::new("App Store Connect created a build upload without an id"))?
        .to_string();
    let file = client.post(
        "/v1/buildUploadFiles",
        &json!({"data": {
            "type": "buildUploadFiles",
            "attributes": {
                "assetType": "ASSET",
                "fileName": artifact.file_name,
                "fileSize": artifact.size,
                "uti": artifact.uti,
            },
            "relationships": {"buildUpload": {"data": {"type": "buildUploads", "id": upload_id}}},
        }}),
    )?;
    let file_id = file["data"]["id"]
        .as_str()
        .ok_or_else(|| CliError::new("App Store Connect reserved an upload file without an id"))?
        .to_string();
    let operations = file["data"]["attributes"]["uploadOperations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if operations.is_empty() {
        return Err(CliError::new(
            "App Store Connect gave no upload operations for the file",
        ));
    }
    progress(Progress::Created {
        upload_id: &upload_id,
        parts: operations.len(),
        bytes: artifact.size,
    });
    send_parts(client, artifact, &operations, progress)?;
    client.patch(
        &format!("/v1/buildUploadFiles/{file_id}"),
        &json!({"data": {
            "type": "buildUploadFiles",
            "id": file_id,
            "attributes": {"uploaded": true},
        }}),
    )?;
    progress(Progress::Committed);
    let mut report = follow_upload(client, &upload_id, follow, progress)?;
    report.app_id = Some(app_id.to_string());
    report.bundle_id = Some(artifact.bundle_id.clone());
    report.platform.get_or_insert(artifact.platform);
    report
        .version
        .get_or_insert_with(|| artifact.version.clone());
    report.build.get_or_insert_with(|| artifact.build.clone());
    Ok(report)
}

/// PUT every part, a few at a time, each from its own file handle.
fn send_parts(
    client: &Client,
    artifact: &Artifact,
    operations: &[Value],
    progress: &dyn Fn(Progress<'_>),
) -> Result<(), CliError> {
    let queue = Mutex::new((0..operations.len()).collect::<Vec<_>>());
    let (sender, receiver) = std::sync::mpsc::channel::<Result<(), CliError>>();
    let session = &client.session;
    std::thread::scope(|scope| {
        for _ in 0..PARALLEL_PARTS.min(operations.len()) {
            let sender = sender.clone();
            let queue = &queue;
            scope.spawn(move || {
                loop {
                    let next = queue.lock().ok().and_then(|mut q| q.pop());
                    let Some(index) = next else { break };
                    let result = send_part(session, artifact, &operations[index], index);
                    let failed = result.is_err();
                    if sender.send(result).is_err() || failed {
                        // Stop this worker; the main thread reports the error.
                        if let Ok(mut q) = queue.lock() {
                            q.clear();
                        }
                        break;
                    }
                }
            });
        }
        drop(sender);
        let mut done = 0;
        let mut first_error = None;
        for result in receiver {
            match result {
                Ok(()) => {
                    done += 1;
                    progress(Progress::Part {
                        done,
                        parts: operations.len(),
                    });
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None if done == operations.len() => Ok(()),
            None => Err(CliError::new(
                "the upload stopped before every part was sent",
            )),
        }
    })
}

fn send_part(
    session: &crate::cli::net::Session,
    artifact: &Artifact,
    operation: &Value,
    index: usize,
) -> Result<(), CliError> {
    let context = || format!("uploading part {} of {}", index + 1, artifact.file_name);
    let url = operation["url"]
        .as_str()
        .ok_or_else(|| CliError::new("an upload operation has no URL").context(context()))?;
    let offset = operation["offset"].as_u64().unwrap_or(0);
    let length = operation["length"]
        .as_u64()
        .unwrap_or(artifact.size - offset.min(artifact.size));
    let method = match operation["method"].as_str().unwrap_or("PUT") {
        "POST" => minreq::Method::Post,
        "PATCH" => minreq::Method::Patch,
        _ => minreq::Method::Put,
    };
    let mut bytes = vec![0u8; usize::try_from(length).map_err(|e| CliError::new(e.to_string()))?];
    let mut file = File::open(&artifact.path)
        .map_err(|e| CliError::new(format!("can't read {}: {e}", artifact.path.display())))?;
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(&mut bytes))
        .map_err(|e| CliError::new(format!("can't read {}: {e}", artifact.path.display())))?;
    let mut request = Request::new(method, url).body(bytes).timeout(600);
    for header in operation["requestHeaders"].as_array().into_iter().flatten() {
        if let (Some(name), Some(value)) = (header["name"].as_str(), header["value"].as_str()) {
            request = request.header(name, value);
        }
    }
    let response = session
        .send_retrying(&request, 5)
        .map_err(|e| e.context(context()))?;
    if !response.ok() {
        return Err(CliError::new(format!(
            "the upload server answered HTTP {} {}",
            response.status, response.reason
        ))
        .context(context()));
    }
    Ok(())
}

/// Poll an upload until Apple accepts or rejects it, and with
/// `follow.processing` until its build is processed.
pub(crate) fn follow_upload(
    client: &Client,
    upload_id: &str,
    follow: &Follow,
    progress: &dyn Fn(Progress<'_>),
) -> Result<UploadReport, CliError> {
    let started = Instant::now();
    let mut last_state = String::new();
    loop {
        let report = read_upload(client, upload_id)?;
        if report.state != last_state {
            progress(Progress::State(&report.state));
            last_state.clone_from(&report.state);
        }
        let settled = matches!(report.state.as_str(), "COMPLETE" | "FAILED");
        if settled && (!follow.processing || report.state == "FAILED") {
            return Ok(report);
        }
        if settled {
            return follow_build(client, report, started, follow, progress);
        }
        if started.elapsed() >= follow.timeout {
            return Ok(report);
        }
        std::thread::sleep(follow.interval);
    }
}

fn follow_build(
    client: &Client,
    mut report: UploadReport,
    started: Instant,
    follow: &Follow,
    progress: &dyn Fn(Progress<'_>),
) -> Result<UploadReport, CliError> {
    let mut last = String::new();
    loop {
        if report.build_id.is_none() {
            report.build_id = read_upload(client, &report.upload_id)?.build_id;
        }
        if let Some(build_id) = &report.build_id {
            let build = client.get(&format!(
                "/v1/builds/{build_id}?fields%5Bbuilds%5D=processingState,version"
            ))?;
            if let Some(state) = build["data"]["attributes"]["processingState"].as_str() {
                if state != last {
                    progress(Progress::Processing(state));
                    last = state.to_string();
                }
                report.processing_state = Some(state.to_string());
                if state != "PROCESSING" {
                    return Ok(report);
                }
            }
        }
        if started.elapsed() >= follow.timeout {
            return Ok(report);
        }
        std::thread::sleep(follow.interval);
    }
}

/// One read of an upload and its build, if it has one.
pub(crate) fn read_upload(client: &Client, upload_id: &str) -> Result<UploadReport, CliError> {
    let value = client.get(&format!("/v1/buildUploads/{upload_id}?include=build"))?;
    let data = &value["data"];
    let attributes = &data["attributes"];
    let state = &attributes["state"];
    let messages = |key: &str| -> Vec<Message> {
        state[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| Message {
                code: m["code"].as_str().unwrap_or_default().to_string(),
                description: m["description"].as_str().unwrap_or_default().to_string(),
            })
            .collect()
    };
    let related = |name: &str| {
        data["relationships"][name]["data"]["id"]
            .as_str()
            .map(str::to_string)
    };
    let included = value["included"].as_array().cloned().unwrap_or_default();
    let processing_state = included
        .iter()
        .find(|i| i["type"] == "builds")
        .and_then(|b| {
            b["attributes"]["processingState"]
                .as_str()
                .map(str::to_string)
        });
    let text = |key: &str| attributes[key].as_str().map(str::to_string);
    Ok(UploadReport {
        upload_id: upload_id.to_string(),
        app_id: None,
        bundle_id: None,
        platform: text("platform").as_deref().and_then(Platform::from_api),
        version: text("cfBundleShortVersionString"),
        build: text("cfBundleVersion"),
        state: state["state"].as_str().unwrap_or("UNKNOWN").to_string(),
        errors: messages("errors"),
        warnings: messages("warnings"),
        infos: messages("infos"),
        build_id: related("build"),
        processing_state,
    })
}

/// Percent-encode a query value.
fn percent(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
