//! Notarization through Apple's notary service, as `notarytool submit`
//! does it: a submission names the file and its SHA-256 and gets temporary
//! credentials for S3, the file goes to S3 under them, in one PUT or, when
//! large, in parts, and the submission moves from In Progress to Accepted,
//! Invalid or Rejected, with a log of what Apple found. The service takes
//! only a team API key.

use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};
use sweetpad_lib::xcode_hash::Sha256;

use super::api::{ApiFailure, Client, stub_base};
use super::key::ApiKey;
use super::sigv4::{self, Credentials, Signing};
use crate::cli::codesign::{hex, sha256};
use crate::cli::net::{Request, Response};
use crate::cli::{CliError, ErrorKind};

/// The region of the bucket notarytool sends files to.
const REGION: &str = "us-west-2";

/// Files up to this size go to S3 in one PUT, larger ones in parts of it.
const PART_BYTES: u64 = 64 * 1024 * 1024;
/// The most parts S3 puts together.
const MAX_PARTS: u64 = 10_000;

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
    Part { done: u64, parts: u64 },
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

/// The part size; `SWEETPAD_NOTARY_PART_BYTES` makes it smaller, for
/// tests to send a small file in parts.
fn part_bytes() -> u64 {
    std::env::var("SWEETPAD_NOTARY_PART_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(PART_BYTES)
}

/// Submit `file`, a zip, package or disk image, and send it to Apple.
pub(crate) fn submit(
    client: &Client,
    file: &Path,
    progress: &dyn Fn(Progress<'_>),
) -> Result<String, CliError> {
    let read_error =
        |e: std::io::Error| CliError::new(format!("can't read {}: {e}", file.display()));
    let size = std::fs::metadata(file).map_err(read_error)?.len();
    let part = part_bytes();
    if size.div_ceil(part) > MAX_PARTS {
        return Err(CliError::new(format!(
            "{} is larger than the {} GB sweetpad sends for notarization",
            file.display(),
            part * MAX_PARTS / 1_000_000_000
        )));
    }
    let digest = hex(&file_sha256(file).map_err(read_error)?);
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
        bytes: size,
    });
    let upload = Upload {
        file,
        size,
        part,
        digest: &digest,
        progress,
    };
    send(
        client,
        &text("bucket")?,
        &text("object")?,
        &credentials,
        &upload,
    )
    .map_err(|e| e.context(format!("sending submission {id}")))?;
    progress(Progress::Sent);
    Ok(id)
}

/// The SHA-256 of `file`, read a piece at a time.
fn file_sha256(file: &Path) -> std::io::Result<[u8; 32]> {
    let mut reader = File::open(file)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(hasher.finish());
        }
        hasher.update(&buffer[..read]);
    }
}

/// The file a submission sends, and how.
struct Upload<'a> {
    file: &'a Path,
    size: u64,
    /// Up to this size the file goes in one PUT, else in parts of it.
    part: u64,
    /// The file's hex SHA-256.
    digest: &'a str,
    progress: &'a dyn Fn(Progress<'_>),
}

/// Send the file as `object` in `bucket`, through S3's acceleration
/// endpoint as notarytool does, else the region's own.
fn send(
    client: &Client,
    bucket: &str,
    object: &str,
    credentials: &Credentials,
    upload: &Upload<'_>,
) -> Result<(), CliError> {
    let endpoints = match stub_base() {
        Some(base) => vec![format!("{base}/s3/{bucket}")],
        None => vec![
            format!("https://{bucket}.s3-accelerate.amazonaws.com"),
            format!("https://{bucket}.s3.{REGION}.amazonaws.com"),
        ],
    };
    let mut failures = Vec::new();
    for endpoint in endpoints {
        let target = Target {
            host: endpoint
                .split_once("://")
                .and_then(|(_, rest)| rest.split('/').next())
                .unwrap_or_default()
                .to_string(),
            endpoint,
            path: sigv4::encode_path(&format!("/{}", object.trim_start_matches('/'))),
            credentials,
        };
        let sent = if upload.size <= upload.part {
            target.put(client, upload)
        } else {
            target.multipart(client, upload)
        };
        match sent {
            Ok(()) => return Ok(()),
            Err(failure) => failures.push(failure),
        }
    }
    Err(CliError::new(format!(
        "couldn't send the file to Apple's notary storage: {}",
        failures.join("; ")
    )))
}

/// The submission's object at one S3 endpoint, and the credentials that
/// sign requests to it.
struct Target<'a> {
    endpoint: String,
    host: String,
    /// The object's path, URI-encoded.
    path: String,
    credentials: &'a Credentials,
}

impl Target<'_> {
    /// The whole file in one PUT.
    fn put(&self, client: &Client, upload: &Upload<'_>) -> Result<(), String> {
        let body = std::fs::read(upload.file)
            .map_err(|e| format!("can't read {}: {e}", upload.file.display()))?;
        self.send(client, &self.request("PUT", "", body, Some(upload.digest)))
            .map(|_| ())
    }

    /// The file in parts, as S3's multipart upload takes it: started, sent
    /// a part at a time, then put together, or abandoned on a failure.
    fn multipart(&self, client: &Client, upload: &Upload<'_>) -> Result<(), String> {
        let started = self.send(client, &self.request("POST", "uploads=", Vec::new(), None))?;
        let body = String::from_utf8_lossy(&started.body);
        let id = between(&body, "<UploadId>", "</UploadId>")
            .ok_or_else(|| format!("{} started a multipart upload with no ID", self.host))?;
        let id = sigv4::encode_value(id);
        let sent = self
            .send_parts(client, upload, &id)
            .and_then(|tags| self.complete(client, &id, &tags));
        if sent.is_err() {
            let query = format!("uploadId={id}");
            let _ = self.send(client, &self.request("DELETE", &query, Vec::new(), None));
        }
        sent
    }

    /// Each part's entity tag, in order.
    fn send_parts(
        &self,
        client: &Client,
        upload: &Upload<'_>,
        id: &str,
    ) -> Result<Vec<String>, String> {
        let read_error = |e: std::io::Error| format!("can't read {}: {e}", upload.file.display());
        let mut reader = File::open(upload.file).map_err(read_error)?;
        let parts = upload.size.div_ceil(upload.part);
        let mut tags = Vec::new();
        for number in 1..=parts {
            let length = upload.part.min(upload.size - (number - 1) * upload.part);
            let mut body = vec![0; usize::try_from(length).map_err(|e| e.to_string())?];
            reader.read_exact(&mut body).map_err(read_error)?;
            let query = format!("partNumber={number}&uploadId={id}");
            let response = self.send(client, &self.request("PUT", &query, body, None))?;
            let tag = response
                .header("ETag")
                .ok_or_else(|| format!("{} gave part {number} no ETag", self.host))?;
            tags.push(tag.to_string());
            (upload.progress)(Progress::Part {
                done: number,
                parts,
            });
        }
        Ok(tags)
    }

    fn complete(&self, client: &Client, id: &str, tags: &[String]) -> Result<(), String> {
        let mut body = String::from("<CompleteMultipartUpload>");
        for (index, tag) in tags.iter().enumerate() {
            let _ = write!(
                body,
                "<Part><PartNumber>{}</PartNumber><ETag>{tag}</ETag></Part>",
                index + 1
            );
        }
        body.push_str("</CompleteMultipartUpload>");
        let query = format!("uploadId={id}");
        let response = self.send(
            client,
            &self.request("POST", &query, body.into_bytes(), None),
        )?;
        // S3 can answer 200 and still refuse, with an error in the body.
        if String::from_utf8_lossy(&response.body).contains("<Error>") {
            return Err(format!(
                "{} couldn't put the parts together: {}",
                self.host,
                response.snippet()
            ));
        }
        Ok(())
    }

    /// A request to the object, signed over `body`, whose hex SHA-256 is
    /// `digest` when known. `query` is canonical; a name without a value
    /// goes in the URL bare.
    fn request(&self, verb: &str, query: &str, body: Vec<u8>, digest: Option<&str>) -> Request {
        let digest = digest.map_or_else(|| hex(&sha256(&body)), str::to_string);
        let amz_date = sigv4::amz_date(SystemTime::now());
        let mut headers = vec![
            ("host".to_string(), self.host.clone()),
            ("x-amz-content-sha256".to_string(), digest.clone()),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];
        if let Some(token) = &self.credentials.session_token {
            headers.push(("x-amz-security-token".to_string(), token.clone()));
        }
        let signing = Signing {
            method: verb,
            path: &self.path,
            query,
            headers: headers.clone(),
            payload_sha256: &digest,
            region: REGION,
            service: "s3",
            amz_date: &amz_date,
        };
        let method = match verb {
            "POST" => minreq::Method::Post,
            "DELETE" => minreq::Method::Delete,
            _ => minreq::Method::Put,
        };
        let url_query: Vec<&str> = query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| pair.strip_suffix('=').unwrap_or(pair))
            .collect();
        let url = match url_query.as_slice() {
            [] => format!("{}{}", self.endpoint, self.path),
            pairs => format!("{}{}?{}", self.endpoint, self.path, pairs.join("&")),
        };
        let mut request = Request::new(method, url)
            .header(
                "Authorization",
                sigv4::authorization(&signing, self.credentials),
            )
            .body(body)
            .timeout(3600);
        // The client writes Host itself.
        for (name, value) in headers.into_iter().filter(|(name, _)| name != "host") {
            request = request.header(name, value);
        }
        request
    }

    /// Send `request`, retrying, and fail on any answer but a success.
    fn send(&self, client: &Client, request: &Request) -> Result<Response, String> {
        match client.session.send_retrying(request, 3) {
            Ok(response) if response.ok() => Ok(response),
            Ok(response) => Err(format!(
                "{} answered {} {}: {}",
                self.host,
                response.status,
                response.reason,
                response.snippet()
            )),
            Err(error) => Err(format!("{}: {error}", self.host)),
        }
    }
}

/// The text between `open` and `close` in `text`.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(&text[start..end])
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
