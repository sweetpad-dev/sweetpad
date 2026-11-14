//! `sweetpad upload`: send an `.ipa` or `.pkg` to App Store Connect and wait
//! until Apple accepts or rejects it; `upload status` follows an earlier one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::cli::appstore::api::Client;
use crate::cli::appstore::artifact::{self, Artifact, Platform};
use crate::cli::appstore::builds::{self, Follow, Message, Progress, UploadReport};
use crate::cli::appstore::key::{ApiKey, KeyArgs};
use crate::cli::output::Output;
use crate::cli::{CliError, CommandResult, Context, ErrorKind, Render, Rendered, net};

#[derive(Debug, Clone, Default, Args)]
pub struct FollowArgs {
    /// Also wait for Apple to finish processing the build, which often takes
    /// 5 to 30 minutes. Without it, upload returns once Apple accepts or
    /// rejects the file.
    #[arg(long)]
    pub wait: bool,
    /// Stop following after this many minutes (default 30, or 120 with
    /// '--wait'). The upload goes on; 'sweetpad upload status' picks it up.
    #[arg(long, value_name = "MINUTES")]
    pub timeout: Option<u64>,
}

impl FollowArgs {
    pub(crate) fn follow(&self) -> Follow {
        let minutes = self.timeout.unwrap_or(if self.wait { 120 } else { 30 });
        // The tests' stub answers at once; Apple's state moves in minutes.
        let interval = std::env::var("SWEETPAD_ASC_POLL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);
        Follow {
            processing: self.wait,
            timeout: Duration::from_secs(minutes * 60),
            interval: Duration::from_secs(interval),
        }
    }
}

#[derive(Debug, Args)]
pub struct UploadArgs {
    /// The .ipa or .pkg to upload.
    pub file: Option<PathBuf>,
    /// The app's App Store Connect ID (the 'Apple ID' on its App Information
    /// page). Looked up from the bundle identifier when left out.
    #[arg(long, value_name = "ID")]
    pub app_id: Option<String>,
    #[command(flatten)]
    pub key: KeyArgs,
    #[command(flatten)]
    pub follow: FollowArgs,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show where an earlier upload stands, following it like upload does.
    Status {
        /// The upload ID that upload printed.
        upload_id: String,
        #[command(flatten)]
        key: KeyArgs,
        #[command(flatten)]
        follow: FollowArgs,
    },
}

pub fn run(ctx: &mut Context, args: &UploadArgs, action: Option<&Action>) -> CommandResult {
    match action {
        Some(Action::Status {
            upload_id,
            key,
            follow,
        }) => {
            let key = ApiKey::resolve(key)?;
            // Before any thread starts: see `Session::from_env`.
            let client = Client::new(key, net::Session::from_env());
            let out = &ctx.out;
            let report = builds::follow_upload(&client, upload_id, &follow.follow(), &|p| {
                show_progress(out, &p);
            })?;
            Ok(rendered(Report {
                file: None,
                upload: report,
            }))
        }
        None => {
            let file = args.file.as_deref().ok_or_else(|| {
                CliError::new("upload needs a file: 'sweetpad upload App.ipa'")
                    .kind(ErrorKind::Usage)
            })?;
            let report = upload_file(ctx, file, args.app_id.as_deref(), &args.key, &args.follow)?;
            Ok(rendered(report))
        }
    }
}

/// Upload `file` and follow it. Shared with 'archive --upload'.
pub(crate) fn upload_file(
    ctx: &Context,
    file: &Path,
    app_id: Option<&str>,
    key: &KeyArgs,
    follow: &FollowArgs,
) -> Result<Report, CliError> {
    let artifact = artifact::inspect(file)?;
    let key = ApiKey::resolve(key)?;
    let client = Client::new(key, net::Session::from_env());
    let app_id = match app_id {
        Some(id) => id.to_string(),
        None => builds::app_id(&client, &artifact.bundle_id)?,
    };
    let out = &ctx.out;
    out.note(&format!(
        "Uploading {} ({} {} ({}), {}) to App Store Connect app {app_id}",
        artifact.file_name,
        artifact.bundle_id,
        artifact.version,
        artifact.build,
        artifact.platform.api()
    ));
    let upload = builds::upload(&client, &artifact, &app_id, &follow.follow(), &|p| {
        show_progress(out, &p);
    })?;
    Ok(Report {
        file: Some(artifact),
        upload,
    })
}

fn show_progress(out: &Output, progress: &Progress<'_>) {
    let (event, text) = match progress {
        Progress::Created {
            upload_id,
            parts,
            bytes,
        } => (
            json!({"event": "upload", "stage": "created", "uploadId": upload_id, "parts": parts, "bytes": bytes}),
            format!(
                "Created upload {upload_id}: {} in {parts} part{}",
                size(*bytes),
                if *parts == 1 { "" } else { "s" }
            ),
        ),
        Progress::Part { done, parts } => (
            json!({"event": "upload", "stage": "part", "done": done, "parts": parts}),
            format!("Sent part {done} of {parts}"),
        ),
        Progress::Committed => (
            json!({"event": "upload", "stage": "committed"}),
            "Sent the file; waiting for App Store Connect to check it".to_string(),
        ),
        Progress::State(state) => (
            json!({"event": "upload", "stage": "state", "state": state}),
            format!("Upload state: {state}"),
        ),
        Progress::Processing(state) => (
            json!({"event": "upload", "stage": "processing", "processingState": state}),
            format!("Build processing: {state}"),
        ),
    };
    out.ndjson_event(&event);
    out.note(&text);
}

#[allow(clippy::cast_precision_loss)] // a size shown to one decimal
fn size(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}

fn rendered(report: Report) -> Rendered {
    if report.upload.failed() {
        Rendered::data_with_exit(report, 1)
    } else {
        Rendered::data(report)
    }
}

/// An upload's outcome.
pub(crate) struct Report {
    pub(crate) file: Option<Artifact>,
    pub(crate) upload: UploadReport,
}

fn messages(list: &[Message]) -> Value {
    list.iter()
        .map(|m| json!({"code": m.code, "description": m.description}))
        .collect()
}

impl Render for Report {
    fn human(&self, out: &Output) {
        let u = &self.upload;
        let headline = match (u.state.as_str(), u.processing_state.as_deref()) {
            (_, Some("VALID")) => "Processed: the build is ready in App Store Connect",
            (_, Some("INVALID" | "FAILED")) => "Apple rejected the build while processing it",
            (_, Some(_)) => "Accepted; Apple is still processing the build",
            ("COMPLETE", None) => "Accepted: Apple is processing the build for TestFlight",
            ("FAILED", None) => "Apple rejected the upload",
            _ => "Still in progress",
        };
        out.line(headline);
        if let Some(file) = &self.file {
            out.line(&format!("  file      {}", file.path.display()));
        }
        match (&u.bundle_id, &u.app_id) {
            (Some(bundle), Some(id)) => out.line(&format!("  app       {bundle} ({id})")),
            (None, Some(id)) => out.line(&format!("  app       {id}")),
            _ => {}
        }
        if let (Some(version), Some(build)) = (&u.version, &u.build) {
            let platform = u.platform.map_or("", |p| p.api());
            out.line(&format!("  version   {version} ({build}) {platform}"));
        }
        out.line(&format!("  upload    {} ({})", u.upload_id, u.state));
        if let Some(build) = &u.build_id {
            let state = u.processing_state.as_deref().unwrap_or("not processed yet");
            out.line(&format!("  build     {build} ({state})"));
        }
        for (label, list) in [
            ("error", &u.errors),
            ("warning", &u.warnings),
            ("info", &u.infos),
        ] {
            for m in list {
                let code = if m.code.is_empty() {
                    String::new()
                } else {
                    format!("ITMS-{}: ", m.code.trim_start_matches("ITMS-"))
                };
                out.line(&format!("  {label:<9} {code}{}", m.description));
            }
        }
        if !matches!(u.state.as_str(), "COMPLETE" | "FAILED")
            || u.processing_state.as_deref() == Some("PROCESSING")
        {
            out.line(&format!(
                "  follow it with 'sweetpad upload status {}'",
                u.upload_id
            ));
        }
    }

    fn json(&self) -> Value {
        let u = &self.upload;
        let mut data = json!({
            "uploadId": u.upload_id,
            "app": u.app_id.as_ref().map(|id| json!({"id": id, "bundleId": u.bundle_id})),
            "platform": u.platform.map(Platform::api),
            "version": u.version,
            "build": u.build,
            "state": u.state,
            "errors": messages(&u.errors),
            "warnings": messages(&u.warnings),
            "infos": messages(&u.infos),
            "buildId": u.build_id,
            "processingState": u.processing_state,
        });
        if let Some(file) = &self.file {
            data["file"] = json!({"path": file.path.display().to_string(), "size": file.size});
        }
        data
    }
}
