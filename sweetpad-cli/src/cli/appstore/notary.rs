//! Notarization through Apple's notary service, as `notarytool submit`
//! does it: a submission names the file and its SHA-256 and gets temporary
//! credentials for S3, the file goes to S3 under them, and the submission
//! moves from In Progress to Accepted, Invalid or Rejected, with a log of
//! what Apple found. The service takes only a team API key.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};

use super::api::{ApiFailure, Client, stub_base};
use super::key::ApiKey;
use super::sigv4::{self, Credentials, Signing};
use crate::cli::codesign::{hex, sha256};
use crate::cli::net::Request;
use crate::cli::{CliError, ErrorKind};

/// The region of the bucket notarytool sends files to.
const REGION: &str = "us-west-2";

/// The most one S3 PUT takes.
const MAX_PUT: u64 = 5 * 1024 * 1024 * 1024;

/// Where a submission stands.
pub(crate) struct Submission {
    pub(crate) id: String,
    pub(crate) name: Option<String>,
    /// `In Progress`, `Accepted`, `Invalid` or `Rejected`.
    pub(crate) status: String,
    pub(crate) created: Option<String>,
}

impl Submission {
    pub(crate) fn finished(&self) -> bool {
        self.status != "In Progress"
    }

    pub(crate) fn accepted(&self) -> bool {
        self.status == "Accepted"
    }
}

pub(crate) enum Progress<'a> {
    Submitted { id: &'a str, bytes: u64 },
    Sent,
    Status(&'a str),
}

/// Fail unless `key` is a team key, the only kind the notary service takes.
pub(crate) fn require_team_key(key: &ApiKey) -> Result<(), CliError> {
    if key.issuer.is_some() {
        return Ok(());
    }
    Err(CliError::new(
        "notarization needs a team API key, and this one is an individual key: pass the team \
         key's issuer ID with '--api-issuer' or SWEETPAD_ASC_ISSUER_ID",
    )
    .kind(ErrorKind::Usage))
}

/// Submit `file`, a zip, package or disk image, and send it to Apple.
pub(crate) fn submit(
    client: &Client,
    file: &Path,
    progress: &dyn Fn(Progress<'_>),
) -> Result<String, CliError> {
    let data = std::fs::read(file)
        .map_err(|e| CliError::new(format!("can't read {}: {e}", file.display())))?;
    if data.len() as u64 > MAX_PUT {
        return Err(CliError::new(format!(
            "{} is larger than the 5 GB sweetpad sends for notarization",
            file.display()
        )));
    }
    let digest = hex(&sha256(&data));
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let created = client.post(
        "/notary/v2/submissions",
        &json!({"submissionName": name, "sha256": digest}),
    )?;
    let data_field = &created["data"];
    let text = |key: &str| {
        data_field["attributes"][key]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| {
                CliError::new(format!("the notary service's new submission has no {key}"))
            })
    };
    let id = data_field["id"]
        .as_str()
        .ok_or_else(|| CliError::new("the notary service's new submission has no ID"))?
        .to_string();
    let credentials = Credentials {
        access_key: text("awsAccessKeyId")?,
        secret_key: text("awsSecretAccessKey")?,
        session_token: text("awsSessionToken").ok(),
    };
    progress(Progress::Submitted {
        id: &id,
        bytes: data.len() as u64,
    });
    put_object(
        client,
        &text("bucket")?,
        &text("object")?,
        &credentials,
        &data,
        &digest,
    )
    .map_err(|e| e.context(format!("sending submission {id}")))?;
    progress(Progress::Sent);
    Ok(id)
}

/// PUT `data` as `object` in `bucket`, through S3's acceleration endpoint
/// as notarytool does, else the region's own.
fn put_object(
    client: &Client,
    bucket: &str,
    object: &str,
    credentials: &Credentials,
    data: &[u8],
    digest: &str,
) -> Result<(), CliError> {
    let path = sigv4::encode_path(&format!("/{}", object.trim_start_matches('/')));
    let endpoints = match stub_base() {
        Some(base) => vec![format!("{base}/s3/{bucket}")],
        None => vec![
            format!("https://{bucket}.s3-accelerate.amazonaws.com"),
            format!("https://{bucket}.s3.{REGION}.amazonaws.com"),
        ],
    };
    let mut failures = Vec::new();
    for endpoint in endpoints {
        let host = endpoint
            .split_once("://")
            .and_then(|(_, rest)| rest.split('/').next())
            .unwrap_or_default()
            .to_string();
        let amz_date = sigv4::amz_date(SystemTime::now());
        let mut headers = vec![
            ("host".to_string(), host.clone()),
            ("x-amz-content-sha256".to_string(), digest.to_string()),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];
        if let Some(token) = &credentials.session_token {
            headers.push(("x-amz-security-token".to_string(), token.clone()));
        }
        let signing = Signing {
            method: "PUT",
            path: &path,
            headers: headers.clone(),
            payload_sha256: digest,
            region: REGION,
            service: "s3",
            amz_date: &amz_date,
        };
        let mut request = Request::new(minreq::Method::Put, format!("{endpoint}{path}"))
            .header("Authorization", sigv4::authorization(&signing, credentials))
            .body(data.to_vec())
            .timeout(3600);
        // The client writes Host itself.
        for (name, value) in headers.into_iter().filter(|(name, _)| name != "host") {
            request = request.header(name, value);
        }
        match client.session.send_retrying(&request, 3) {
            Ok(response) if response.ok() => return Ok(()),
            Ok(response) => failures.push(format!(
                "{host} answered {} {}: {}",
                response.status,
                response.reason,
                response.snippet()
            )),
            Err(error) => failures.push(format!("{host}: {error}")),
        }
    }
    Err(CliError::new(format!(
        "couldn't send the file to Apple's notary storage: {}",
        failures.join("; ")
    )))
}

/// Where submission `id` stands.
pub(crate) fn status(client: &Client, id: &str) -> Result<Submission, ApiFailure> {
    let value = client.get(&format!("/notary/v2/submissions/{id}"))?;
    let attributes = &value["data"]["attributes"];
    let text = |key: &str| attributes[key].as_str().map(str::to_string);
    Ok(Submission {
        id: value["data"]["id"].as_str().unwrap_or(id).to_string(),
        name: text("name"),
        status: text("status").unwrap_or_else(|| "In Progress".to_string()),
        created: text("createdDate"),
    })
}

/// Follow submission `id` until Apple decides or `timeout` passes.
pub(crate) fn wait(
    client: &Client,
    id: &str,
    timeout: Duration,
    interval: Duration,
    progress: &dyn Fn(Progress<'_>),
) -> Result<Submission, CliError> {
    let start = Instant::now();
    let mut last = String::new();
    loop {
        let submission = match status(client, id) {
            Ok(submission) => submission,
            // A new submission can take a moment to be found.
            Err(failure) if failure.status == Some(404) && start.elapsed() < interval * 6 => {
                Submission {
                    id: id.to_string(),
                    name: None,
                    status: "In Progress".to_string(),
                    created: None,
                }
            }
            Err(failure) => return Err(failure.into()),
        };
        if submission.status != last {
            progress(Progress::Status(&submission.status));
            last.clone_from(&submission.status);
        }
        if submission.finished() || start.elapsed() >= timeout {
            return Ok(submission);
        }
        std::thread::sleep(interval);
    }
}

/// The log Apple wrote of submission `id`: its status, summary and the
/// issues it found, each with a path and message.
pub(crate) fn log(client: &Client, id: &str) -> Result<Value, CliError> {
    let value = client.get(&format!("/notary/v2/submissions/{id}/logs"))?;
    let url = value["data"]["attributes"]["developerLogUrl"]
        .as_str()
        .ok_or_else(|| CliError::new(format!("submission {id} has no log yet")))?;
    let response = client
        .session
        .send_retrying(&Request::new(minreq::Method::Get, url), 3)
        .map_err(|e| e.context(format!("reading the log of submission {id}")))?;
    if !response.ok() {
        return Err(CliError::new(format!(
            "the log of submission {id} answered {} {}",
            response.status, response.reason
        )));
    }
    serde_json::from_slice(&response.body)
        .map_err(|e| CliError::new(format!("the log of submission {id} isn't JSON: {e}")))
}

/// The issues of a log, as `path: message` lines.
pub(crate) fn issues(log: &Value) -> Vec<String> {
    log["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|issue| {
            let message = issue["message"].as_str().unwrap_or("an issue");
            let architecture = issue["architecture"]
                .as_str()
                .map(|a| format!(" ({a})"))
                .unwrap_or_default();
            match issue["path"].as_str() {
                Some(path) => format!("{path}{architecture}: {message}"),
                None => message.to_string(),
            }
        })
        .collect()
}
