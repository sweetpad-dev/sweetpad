//! `sweetpad notarize`: send an app, zip or package to Apple's notary
//! service, wait for its verdict and staple the ticket; `notarize status`
//! and `notarize log` follow an earlier submission, and `notarize staple`
//! staples its ticket once Apple accepts it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::cli::appstore::api::Client;
use crate::cli::appstore::key::{ApiKey, KeyArgs};
use crate::cli::appstore::notary::{self, Progress, Submission};
use crate::cli::codesign::staple;
use crate::cli::output::Output;
use crate::cli::{CliError, CommandResult, Context, ErrorKind, Render, Rendered, net, process};
use sweetpad_core::scratch::ScratchDir;

#[derive(Debug, Clone, Default, Args)]
pub struct NotarizeOptions {
    /// Return once Apple has the file, with the submission ID to follow it
    /// by. Nothing is stapled: run 'sweetpad notarize staple' once Apple
    /// accepts it.
    #[arg(long)]
    pub no_wait: bool,
    /// Leave the ticket unstapled.
    #[arg(long)]
    pub no_staple: bool,
    /// Stop waiting after this many minutes (default 30). The submission
    /// goes on; 'sweetpad notarize status' picks it up.
    #[arg(long, value_name = "MINUTES")]
    pub timeout: Option<u64>,
}

#[derive(Debug, Args)]
pub struct NotarizeArgs {
    /// The .app, .zip, .pkg or .dmg to notarize. An app goes to Apple as a
    /// zip. Apps, packages and signed disk images get the ticket stapled.
    pub file: Option<PathBuf>,
    #[command(flatten)]
    pub options: NotarizeOptions,
    #[command(flatten)]
    pub key: KeyArgs,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show where an earlier submission stands.
    Status {
        /// The submission ID that notarize printed.
        submission_id: String,
        /// Wait for Apple's verdict.
        #[arg(long)]
        wait: bool,
        #[command(flatten)]
        key: KeyArgs,
    },
    /// Show the log Apple wrote of a submission: what it checked and the
    /// issues it found.
    Log {
        /// The submission ID that notarize printed.
        submission_id: String,
        #[command(flatten)]
        key: KeyArgs,
    },
    /// Staple the ticket of an app, package or signed disk image Apple has
    /// accepted, so Gatekeeper finds it offline. Needs no API key.
    Staple {
        /// The .app, .pkg or .dmg that was notarized.
        file: PathBuf,
    },
}

pub fn run(ctx: &mut Context, args: &NotarizeArgs, action: Option<&Action>) -> CommandResult {
    match action {
        Some(Action::Status {
            submission_id,
            wait,
            key,
        }) => {
            let client = client(key)?;
            let submission = if *wait {
                notary::wait(
                    &client,
                    submission_id,
                    Duration::from_secs(30 * 60),
                    poll_interval(),
                    &|p| show_progress(&ctx.out, &p),
                )?
            } else {
                notary::status(&client, submission_id)?
            };
            let issues = issues_of(&client, &submission);
            Ok(rendered(Report {
                file: None,
                submission,
                issues,
                stapled: Vec::new(),
                notes: Vec::new(),
            }))
        }
        Some(Action::Log { submission_id, key }) => {
            let client = client(key)?;
            let log = notary::log(&client, submission_id)?;
            Ok(Rendered::data(LogReport(log)))
        }
        Some(Action::Staple { file }) => {
            if kind(file)? == Kind::Zip {
                return Err(CliError::new(format!(
                    "{} takes no ticket: sweetpad staples apps, packages and disk images",
                    file.display()
                ))
                .kind(ErrorKind::Usage));
            }
            let stapled = staple::staple(file)
                .map_err(|e| CliError::new(e).context("stapling the ticket"))?;
            Ok(Rendered::data(StapleReport(stapled)))
        }
        None => {
            let file = args.file.as_deref().ok_or_else(|| {
                CliError::new("notarize needs a file: 'sweetpad notarize App.app'")
                    .kind(ErrorKind::Usage)
            })?;
            let report = notarize(ctx, file, &args.options, &args.key)?;
            Ok(rendered(report))
        }
    }
}

fn client(key: &KeyArgs) -> Result<Client, CliError> {
    let key = ApiKey::resolve(key)?;
    notary::require_team_key(&key)?;
    // Before any thread starts: see `Session::from_env`.
    Ok(Client::notary(key, net::Session::from_env()))
}

/// How often to ask where a submission stands.
fn poll_interval() -> Duration {
    // The tests' stub answers at once; Apple decides in minutes.
    let secs = std::env::var("SWEETPAD_ASC_POLL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    Duration::from_secs(secs)
}

/// What a file is, for sending and stapling.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    App,
    Zip,
    Package,
    DiskImage,
}

fn kind(file: &Path) -> Result<Kind, CliError> {
    let extension = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    if !file.exists() {
        return Err(
            CliError::new(format!("{} doesn't exist", file.display())).kind(ErrorKind::Usage)
        );
    }
    match (file.is_dir(), extension) {
        (true, "app") => Ok(Kind::App),
        (false, "zip") => Ok(Kind::Zip),
        (false, "pkg") => Ok(Kind::Package),
        (false, "dmg") => Ok(Kind::DiskImage),
        _ => Err(CliError::new(format!(
            "{} isn't an app, zip, package or disk image, the files Apple notarizes",
            file.display()
        ))
        .kind(ErrorKind::Usage)),
    }
}

/// Notarize `file` and staple it. Shared with 'archive --notarize'.
pub(crate) fn notarize(
    ctx: &Context,
    file: &Path,
    options: &NotarizeOptions,
    key: &KeyArgs,
) -> Result<Report, CliError> {
    let kind = kind(file)?;
    let client = client(key)?;
    let stage = ScratchDir::new("sweetpad-notarize").map_err(|e| CliError::new(e.to_string()))?;
    let sent = if kind == Kind::App {
        zip_app(file, &stage)?
    } else {
        file.to_path_buf()
    };
    let out = &ctx.out;
    out.note(&format!("Notarizing {}", file.display()));
    let id = notary::submit(&client, &sent, &|p| show_progress(out, &p))?;
    let mut report = Report {
        file: Some(file.to_path_buf()),
        submission: Submission {
            id: id.clone(),
            name: sent.file_name().map(|n| n.to_string_lossy().into_owned()),
            status: "In Progress".to_string(),
            created: None,
        },
        issues: Vec::new(),
        stapled: Vec::new(),
        notes: Vec::new(),
    };
    if options.no_wait {
        return Ok(report);
    }
    let timeout = Duration::from_secs(options.timeout.unwrap_or(30) * 60);
    report.submission = notary::wait(&client, &id, timeout, poll_interval(), &|p| {
        show_progress(out, &p);
    })?;
    report.issues = issues_of(&client, &report.submission);
    if !report.submission.accepted() || options.no_staple {
        return Ok(report);
    }
    match kind {
        Kind::DiskImage if !staple::disk_image_is_signed(file).map_err(CliError::new)? => {
            report.notes.push(
                "an unsigned disk image takes no ticket; Gatekeeper looks the app's ticket up \
                 online, or sign the disk image with Developer ID and notarize it again"
                    .to_string(),
            );
        }
        Kind::App | Kind::Package | Kind::DiskImage => {
            let stapled = staple_when_filed(file)?;
            out.note(&format!(
                "Stapled the ticket to {} ({})",
                stapled.path.display(),
                stapled.record
            ));
            report.stapled.push(stapled.path);
        }
        Kind::Zip => report.notes.push(
            "a zip takes no ticket: staple the app inside it with 'sweetpad notarize' on the app, \
             or ship the zip as it is"
                .to_string(),
        ),
    }
    Ok(report)
}

/// Zip `app` with its parent folder's name in each path, as `ditto -c -k
/// --keepParent` does, links kept as links.
pub(crate) fn zip_app(app: &Path, dir: &Path) -> Result<PathBuf, CliError> {
    let name = app
        .file_name()
        .ok_or_else(|| CliError::new(format!("{} has no name", app.display())))?;
    let parent = app.parent().unwrap_or(Path::new("."));
    let zip = dir.join(Path::new(name).with_extension("zip"));
    process::capture(
        "zip",
        &["-qry", &zip.display().to_string(), &name.to_string_lossy()],
        Some(parent),
    )?;
    Ok(zip)
}

/// Staple `file`, giving Apple's ticket service a minute to file a ticket
/// it has only just issued.
fn staple_when_filed(file: &Path) -> Result<staple::Stapled, CliError> {
    let mut attempt = 0;
    loop {
        match staple::staple(file) {
            Ok(stapled) => return Ok(stapled),
            Err(error) if attempt < 6 && error.starts_with("Apple has no notarization ticket") => {
                attempt += 1;
                std::thread::sleep(poll_interval().min(Duration::from_secs(10)));
            }
            Err(error) => return Err(CliError::new(error).context("stapling the ticket")),
        }
    }
}

/// The issues of a decided submission's log; none when it can't be read.
fn issues_of(client: &Client, submission: &Submission) -> Vec<String> {
    if !submission.finished() {
        return Vec::new();
    }
    notary::log(client, &submission.id)
        .map(|log| notary::issues(&log))
        .unwrap_or_default()
}

fn show_progress(out: &Output, progress: &Progress<'_>) {
    let (event, text) = match progress {
        Progress::Submitted { id, bytes } => (
            json!({"event": "notarize", "stage": "submitted", "submissionId": id, "bytes": bytes}),
            format!("Created submission {id}; sending {bytes} bytes"),
        ),
        Progress::Part { done, parts } => (
            json!({"event": "notarize", "stage": "part", "done": done, "parts": parts}),
            format!("Sent part {done} of {parts}"),
        ),
        Progress::Sent => (
            json!({"event": "notarize", "stage": "sent"}),
            "Sent the file; waiting for Apple's verdict".to_string(),
        ),
        Progress::Status(status) => (
            json!({"event": "notarize", "stage": "status", "status": status}),
            format!("Notarization: {status}"),
        ),
    };
    out.ndjson_event(&event);
    out.note(&text);
}

fn rendered(report: Report) -> Rendered {
    if report.submission.finished() && !report.submission.accepted() {
        Rendered::data_with_exit(report, 1)
    } else {
        Rendered::data(report)
    }
}

/// A submission's outcome.
pub(crate) struct Report {
    pub(crate) file: Option<PathBuf>,
    pub(crate) submission: Submission,
    pub(crate) issues: Vec<String>,
    pub(crate) stapled: Vec<PathBuf>,
    pub(crate) notes: Vec<String>,
}

impl Report {
    pub(crate) fn rejected(&self) -> bool {
        self.submission.finished() && !self.submission.accepted()
    }
}

impl Render for Report {
    fn human(&self, out: &Output) {
        let s = &self.submission;
        out.line(match s.status.as_str() {
            "Accepted" if self.stapled.is_empty() => "Notarized",
            "Accepted" => "Notarized and stapled",
            "In Progress" => "Apple is still checking the file",
            _ => "Apple refused to notarize the file",
        });
        if let Some(file) = &self.file {
            out.line(&format!("  file        {}", file.display()));
        }
        out.line(&format!("  submission  {} ({})", s.id, s.status));
        for path in &self.stapled {
            out.line(&format!("  stapled     {}", path.display()));
        }
        for issue in &self.issues {
            out.line(&format!("  issue       {issue}"));
        }
        for note in &self.notes {
            out.line(&format!("  note        {note}"));
        }
        if !s.finished() {
            out.line(&format!(
                "  follow it with 'sweetpad notarize status {} --wait'",
                s.id
            ));
        } else if !s.accepted() {
            out.line(&format!(
                "  read Apple's log with 'sweetpad notarize log {}'",
                s.id
            ));
        }
    }

    fn json(&self) -> Value {
        let s = &self.submission;
        json!({
            "submissionId": s.id,
            "name": s.name,
            "status": s.status,
            "createdDate": s.created,
            "file": self.file.as_ref().map(|f| f.display().to_string()),
            "issues": self.issues,
            "stapled": self.stapled.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "notes": self.notes,
        })
    }
}

/// What 'notarize staple' did.
struct StapleReport(staple::Stapled);

impl Render for StapleReport {
    fn human(&self, out: &Output) {
        out.line("Stapled the ticket");
        out.line(&format!("  file    {}", self.0.path.display()));
        out.line(&format!("  record  {}", self.0.record));
    }

    fn json(&self) -> Value {
        json!({"file": self.0.path.display().to_string(), "record": self.0.record})
    }
}

/// Apple's log, as it wrote it.
struct LogReport(Value);

impl Render for LogReport {
    fn human(&self, out: &Output) {
        let log = &self.0;
        out.line(&format!(
            "{} ({})",
            log["statusSummary"].as_str().unwrap_or("Notarization log"),
            log["status"].as_str().unwrap_or("unknown")
        ));
        let issues = notary::issues(log);
        if issues.is_empty() {
            out.line("  no issues");
        }
        for issue in issues {
            out.line(&format!("  issue  {issue}"));
        }
    }

    fn json(&self) -> Value {
        self.0.clone()
    }
}
