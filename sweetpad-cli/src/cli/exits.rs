//! Why an app stopped running, read from launchd's own account of the exit.
//!
//! launchd (`launchd_sim` inside a simulator) logs one line when a job it
//! manages ends, in one of three shapes:
//!
//! - `exited due to exit(3), ran for 1361ms` — the process called `exit`.
//! - `exited due to SIGABRT | sent by ExitProbe[59571], ran for 1351ms` — a
//!   signal, and who sent it (`exc handler[pid]` for a hardware fault).
//! - `exited with exit reason (namespace: 10 code: 0xfbfbfbfb) -
//!   OS_REASON_SPRINGBOARD | <RBSTerminateContext| … explanation:…>` — the
//!   system ended it, with a reason namespace, a code, and often an
//!   explanation.
//!
//! The job's label and pid ride in the entry's `subsystem` field, not the
//! message: `user/503/UIKitApplication:<bundle id>[…][rb-legacy] [pid]` on a
//! simulator, `gui/503/application.<bundle id>.<n>.<n>… [pid]` on macOS. A
//! process launchd did not start (a macOS app spawned directly, as `app run
//! --mac` does) has no job, and so no exit line.
//!
//! This is the evidence when there is no crash report: a host or watchdog
//! kill writes none, and launchd's line is the only record of it.
//!
//! Two more accounts fill in what launchd misses. A crash report
//! ([`crash_reports`]) names the signal and exception of any crash, however
//! the app was started. sweetpad's own record ([`from_record`]) covers the
//! macOS apps it spawns and waits on, whose clean exits and kills nothing else
//! notes. [`merge`] folds the three into one list.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;

use crate::cli::state::RecordedExit;
use crate::cli::{CliError, process};

/// Where the exit lines are read from: a simulator's own unified log (through
/// `simctl spawn`), or the host's for a macOS app.
pub enum Source<'a> {
    Simulator(&'a str),
    Mac,
}

/// The stretch of log history to search: the last `DUR` (`log show --last`
/// syntax, e.g. `10m`), or an absolute range in seconds since the epoch.
pub enum Window<'a> {
    Last(&'a str),
    Between { start: f64, end: f64 },
}

/// How the process ended, as launchd tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// The process called `exit` with this status.
    Status(i32),
    /// A signal (`SIGABRT`), and the sender launchd named (`exc handler[60122]`).
    Signal {
        name: String,
        sent_by: Option<String>,
    },
    /// An OS exit reason: its namespace, code, and launchd's name for the
    /// namespace (`OS_REASON_SPRINGBOARD`).
    Reason {
        namespace: u32,
        code: u64,
        name: String,
    },
}

/// Which account an [`Exit`] was read from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Origin {
    /// launchd's exit line in the unified log.
    #[default]
    Launchd,
    /// sweetpad's record of a process it spawned and waited on.
    Sweetpad,
    /// A crash report in `~/Library/Logs/DiagnosticReports`.
    CrashReport,
}

impl Origin {
    /// The name the JSON `source` field carries.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Launchd => "launchd",
            Origin::Sweetpad => "sweetpad",
            Origin::CrashReport => "crashReport",
        }
    }
}

/// One termination of an app's process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exit {
    /// launchd's timestamp as logged (`2026-09-26 17:33:10.087799+0200`).
    pub time: String,
    /// The bundle id the job was launched for.
    pub bundle_id: String,
    pub pid: Option<u32>,
    pub cause: Cause,
    /// Why, in launchd's words: a terminate context's `explanation:`, or the
    /// circumstance a plain exit carries (`during system shutdown`).
    pub explanation: Option<String>,
    pub ran_for_ms: Option<u64>,
    /// The whole message, for whatever the parse does not reach.
    pub message: String,
    pub origin: Origin,
}

/// The fields of a `log show --style ndjson` entry this module reads.
#[derive(Deserialize)]
struct Entry {
    timestamp: Option<String>,
    subsystem: Option<String>,
    #[serde(rename = "eventMessage")]
    event_message: Option<String>,
}

/// Every exit of `bundle_ids` launchd logged within `window`, oldest first —
/// of every app job when `bundle_ids` is empty. The query is bounded by
/// `timeout`, so a wedged `log` can cost a caller at most that long.
pub fn query(
    source: &Source,
    bundle_ids: &[&str],
    window: &Window,
    timeout: Duration,
) -> Result<Vec<Exit>, CliError> {
    let (program, args) = log_show_command(source, bundle_ids, window);
    let text = run_bounded(program, &args, timeout)?;
    Ok(text
        .lines()
        .filter_map(|line| parse_ndjson_line(line, bundle_ids))
        .collect())
}

/// The `log show` invocation for [`query`]. The predicate narrows on the
/// launchd process and the message's leading `exited`, then on the bundle ids
/// in the job label (or on any app label, for none); [`parse_ndjson_line`]
/// checks the label exactly, since a `CONTAINS` also matches a longer bundle
/// id that shares the prefix.
fn log_show_command(
    source: &Source,
    bundle_ids: &[&str],
    window: &Window,
) -> (&'static str, Vec<String>) {
    let (launchd, label) = match source {
        Source::Simulator(_) => ("launchd_sim", "UIKitApplication:"),
        Source::Mac => ("launchd", "application."),
    };
    let mut jobs: Vec<String> = bundle_ids
        .iter()
        .map(|id| format!("subsystem CONTAINS \"{label}{}\"", predicate_escape(id)))
        .collect();
    if jobs.is_empty() {
        jobs.push(format!("subsystem CONTAINS \"{label}\""));
    }
    let predicate = format!(
        "process == \"{launchd}\" AND eventMessage BEGINSWITH \"exited \" AND ({})",
        jobs.join(" OR ")
    );
    let mut show = vec![
        "show".to_string(),
        "--style".to_string(),
        "ndjson".to_string(),
    ];
    match window {
        Window::Last(last) => {
            show.push("--last".to_string());
            show.push((*last).to_string());
        }
        // `log show` takes an epoch as `@<seconds>`, which sidesteps the local
        // time zone its date formats assume.
        Window::Between { start, end } => {
            show.push("--start".to_string());
            show.push(format!("@{}", start.floor()));
            show.push("--end".to_string());
            show.push(format!("@{}", end.ceil()));
        }
    }
    show.push("--predicate".to_string());
    show.push(predicate);
    match source {
        Source::Mac => ("log", show),
        Source::Simulator(udid) => {
            let mut args = vec![
                "simctl".to_string(),
                "spawn".to_string(),
                (*udid).to_string(),
                "log".to_string(),
            ];
            args.append(&mut show);
            ("xcrun", args)
        }
    }
}

/// Escape a value for a double-quoted NSPredicate string literal.
fn predicate_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Run a command to completion and return its stdout, killing it once
/// `timeout` passes. stderr is kept for the error only: `simctl spawn … log`
/// writes host-user lookup noise there on every call.
fn run_bounded(program: &str, args: &[String], timeout: Duration) -> Result<String, CliError> {
    fn drain(pipe: Option<impl std::io::Read + Send + 'static>) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut child = process::spawn_piped_both(program, &refs, None)?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CliError::new(format!(
                    "log show took longer than {}s",
                    timeout.as_secs()
                )));
            }
            Err(e) => return Err(CliError::new(format!("failed to wait for log show: {e}"))),
        }
    };
    let text = stdout.join().unwrap_or_default();
    if !status.success() {
        let errors = stderr.join().unwrap_or_default();
        let detail = errors
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty() && !l.starts_with("getpwuid_r"))
            .map_or_else(|| format!("exited with {status}"), str::to_string);
        return Err(CliError::new(format!("log show failed — {detail}")));
    }
    Ok(text)
}

/// Parse one `log show --style ndjson` line into an [`Exit`] when it is
/// launchd's exit line for an app job — one of `bundle_ids`, or any app when
/// that is empty. Anything else — the trailing `{"count":…}` summary, another
/// job whose label merely shares a prefix, a message this module does not
/// recognize — is `None`.
#[must_use]
pub fn parse_ndjson_line(line: &str, bundle_ids: &[&str]) -> Option<Exit> {
    let entry: Entry = serde_json::from_str(line).ok()?;
    let subsystem = entry.subsystem?;
    let bundle_id = job_bundle_id(&subsystem)?;
    if !bundle_ids.is_empty() && !bundle_ids.contains(&bundle_id) {
        return None;
    }
    let bundle_id = bundle_id.to_string();
    let message = entry.event_message?;
    let (cause, explanation, ran_for_ms) = parse_message(&message)?;
    Some(Exit {
        time: entry.timestamp.unwrap_or_default(),
        bundle_id,
        pid: job_pid(&subsystem),
        cause,
        explanation,
        ran_for_ms,
        message,
        origin: Origin::Launchd,
    })
}

/// The app bundle id in a launchd job label: `UIKitApplication:<id>[…]` in a
/// simulator; on macOS `application.<id>.<n>.<n>`, optionally followed by a
/// UUID, which is peeled from the right so an id with a numeric component of
/// its own stays whole.
fn job_bundle_id(subsystem: &str) -> Option<&str> {
    if let Some((_, rest)) = subsystem.split_once("UIKitApplication:") {
        return rest.split_once('[').map(|(id, _)| id);
    }
    let (_, rest) = subsystem.split_once("/application.")?;
    let mut label = rest.split(' ').next()?;
    if let Some((head, uuid)) = label.rsplit_once('.')
        && uuid.contains('-')
    {
        label = head;
    }
    for _ in 0..2 {
        let (head, serial) = label.rsplit_once('.')?;
        if serial.is_empty() || !serial.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        label = head;
    }
    Some(label)
}

/// The pid in a job label's trailing ` [pid]`.
fn job_pid(subsystem: &str) -> Option<u32> {
    let (_, tail) = subsystem.rsplit_once(" [")?;
    tail.strip_suffix(']')?.parse().ok()
}

/// Split launchd's exit message into its cause, explanation, and run time.
fn parse_message(message: &str) -> Option<(Cause, Option<String>, Option<u64>)> {
    // `, ran for Nms` closes every shape; an explanation may contain commas,
    // so it is cut from the end.
    let (body, ran_for_ms) = match message.rsplit_once(", ran for ") {
        Some((body, tail)) => (body, tail.strip_suffix("ms").and_then(|n| n.parse().ok())),
        None => (message, None),
    };
    if let Some(rest) = body.strip_prefix("exited with exit reason (namespace: ") {
        let (namespace, rest) = rest.split_once(" code: ")?;
        let (code, rest) = rest.split_once(')')?;
        let name = rest
            .strip_prefix(" - ")?
            .split([' ', '|'])
            .next()
            .unwrap_or_default();
        let cause = Cause::Reason {
            namespace: namespace.trim().parse().ok()?,
            code: parse_hex(code.trim())?,
            name: name.to_string(),
        };
        return Some((cause, context_explanation(rest), ran_for_ms));
    }
    let rest = body.strip_prefix("exited due to ")?;
    if let Some(status) = rest.strip_prefix("exit(") {
        let (status, circumstance) = status.split_once(')')?;
        return Some((
            Cause::Status(status.parse().ok()?),
            non_empty(circumstance),
            ran_for_ms,
        ));
    }
    let (name, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    if !name.starts_with("SIG") {
        return None;
    }
    // `| sent by <name>[<pid>]` — a sender name can hold spaces (`exc
    // handler`), so it runs to the bracketed pid; what follows is the
    // circumstance (`during system shutdown`).
    let (sent_by, circumstance) = match rest.trim_start().strip_prefix("| sent by ") {
        Some(sender) => match sender.find(']') {
            Some(close) => (
                Some(sender[..=close].to_string()),
                non_empty(&sender[close + 1..]),
            ),
            None => (non_empty(sender), None),
        },
        None => (None, non_empty(rest)),
    };
    Some((
        Cause::Signal {
            name: name.to_string(),
            sent_by,
        },
        circumstance,
        ran_for_ms,
    ))
}

/// The `explanation:` a terminate context carries, up to the next field.
fn context_explanation(context: &str) -> Option<String> {
    let (_, text) = context.split_once("explanation:")?;
    let end = [text.find('\n'), text.find(" reportType:"), text.find('>')]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(text.len());
    non_empty(&text[..end])
}

fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn parse_hex(text: &str) -> Option<u64> {
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))?;
    u64::from_str_radix(digits, 16).ok()
}

/// `OS_REASON_JETSAM`: the kernel ended the process to reclaim memory.
const NAMESPACE_JETSAM: u32 = 1;
/// `OS_REASON_SIGNAL`: the code is the signal number.
const NAMESPACE_SIGNAL: u32 = 2;

/// Signals that mean the process faulted or aborted, rather than that
/// something asked it to stop.
const CRASH_SIGNALS: [(&str, u64); 7] = [
    ("SIGILL", 4),
    ("SIGTRAP", 5),
    ("SIGABRT", 6),
    ("SIGFPE", 8),
    ("SIGBUS", 10),
    ("SIGSEGV", 11),
    ("SIGSYS", 12),
];

impl Exit {
    /// Whether this exit is a crash — a fault or abort signal — which the
    /// system backs with a crash report.
    #[must_use]
    pub fn is_crash(&self) -> bool {
        match &self.cause {
            Cause::Signal { name, .. } => CRASH_SIGNALS.iter().any(|(n, _)| n == name),
            Cause::Reason {
                namespace, code, ..
            } => *namespace == NAMESPACE_SIGNAL && CRASH_SIGNALS.iter().any(|(_, n)| n == code),
            Cause::Status(_) => false,
        }
    }

    /// A plain-words reading of the exit, only where its meaning is
    /// well established: a status, a crash signal, memory pressure, and
    /// the codes Apple documents (`0x8badf00d`, `0xdead10cc`). Anything else
    /// has no label — its reason and explanation stand on their own.
    #[must_use]
    pub fn label(&self) -> Option<String> {
        match &self.cause {
            Cause::Status(0) => Some("exited normally".to_string()),
            Cause::Status(status) => Some(format!("exited with status {status}")),
            Cause::Signal { name, .. } => self.is_crash().then(|| format!("crashed with {name}")),
            Cause::Reason {
                namespace, code, ..
            } => match (*namespace, *code) {
                (NAMESPACE_JETSAM, _) => Some("killed for memory pressure (jetsam)".to_string()),
                (NAMESPACE_SIGNAL, signal) => Some(
                    CRASH_SIGNALS
                        .iter()
                        .find(|(_, n)| *n == signal)
                        .map_or_else(
                            || format!("ended by signal {signal}"),
                            |(name, _)| format!("crashed with {name}"),
                        ),
                ),
                (_, 0x8bad_f00d) => {
                    Some("watchdog: took too long to launch, resume, or respond".to_string())
                }
                (_, 0xdead_10cc) => {
                    Some("held a file or database lock while suspended".to_string())
                }
                _ => None,
            },
        }
    }

    /// The short token for how it ended: the reason namespace's name, the
    /// signal, or `exit(N)`.
    #[must_use]
    pub fn reason(&self) -> String {
        match &self.cause {
            Cause::Status(status) => format!("exit({status})"),
            Cause::Signal { name, .. } | Cause::Reason { name, .. } => name.clone(),
        }
    }

    /// The reason code as launchd prints it (`0xfbfbfbfb`), for a reason exit.
    #[must_use]
    pub fn code(&self) -> Option<String> {
        match &self.cause {
            Cause::Reason { code, .. } => Some(format!("{code:#x}")),
            _ => None,
        }
    }

    /// One line: the label (or launchd's explanation, or the bare reason),
    /// then the raw reason in parentheses — `Termination requested by simulator
    /// host (OS_REASON_SPRINGBOARD 0xfbfbfbfb)`, `crashed with SIGABRT (sent by
    /// ExitProbe[62583])`.
    #[must_use]
    pub fn summary(&self) -> String {
        let label = self.label();
        match &self.cause {
            Cause::Reason { name, code, .. } => {
                let headline = label
                    .or_else(|| self.explanation.clone())
                    .unwrap_or_else(|| name.clone());
                format!("{headline} ({name} {code:#x})")
            }
            Cause::Signal { name, sent_by } => {
                let mut detail: Vec<String> = Vec::new();
                if let Some(sender) = sent_by {
                    detail.push(format!("sent by {sender}"));
                }
                detail.extend(self.explanation.clone());
                let headline = label.unwrap_or_else(|| name.clone());
                if detail.is_empty() {
                    headline
                } else {
                    format!("{headline} ({})", detail.join(" "))
                }
            }
            Cause::Status(_) => {
                let headline = label.unwrap_or_else(|| self.reason());
                match &self.explanation {
                    Some(circumstance) => format!("{headline} ({circumstance})"),
                    None => headline,
                }
            }
        }
    }

    /// The machine form: every field present, `null` where this exit's cause
    /// has none, so a consumer reads one shape whatever the cause.
    #[must_use]
    pub fn json(&self, crash_report: Option<&Path>) -> serde_json::Value {
        let (kind, namespace, exit_status, sent_by) = match &self.cause {
            Cause::Status(status) => ("exit", None, Some(*status), None),
            Cause::Signal { sent_by, .. } => ("signal", None, None, sent_by.clone()),
            Cause::Reason { namespace, .. } => ("reason", Some(*namespace), None, None),
        };
        serde_json::json!({
            "time": self.time,
            "bundleId": self.bundle_id,
            "pid": self.pid,
            "kind": kind,
            "reason": self.reason(),
            "namespace": namespace,
            "code": self.code(),
            "exitStatus": exit_status,
            "sentBy": sent_by,
            "explanation": self.explanation,
            "label": self.label(),
            "ranForMs": self.ran_for_ms,
            "crashReport": crash_report.map(|p| p.display().to_string()),
            "message": self.message,
            "source": self.origin.as_str(),
        })
    }

    /// When launchd logged the exit, in seconds since the epoch.
    #[must_use]
    pub fn epoch_seconds(&self) -> Option<f64> {
        epoch_seconds(&self.time)
    }
}

/// Seconds since the epoch for a unified-log timestamp
/// (`2026-09-26 17:33:10.087799+0200`), so an exit can be ordered against a
/// time from elsewhere, like the moment a test failed.
fn epoch_seconds(timestamp: &str) -> Option<f64> {
    let (date, rest) = timestamp.split_once(' ')?;
    let mut ymd = date.split('-').map(str::parse::<i64>);
    let (year, month, day) = (ymd.next()?.ok()?, ymd.next()?.ok()?, ymd.next()?.ok()?);
    let (clock, offset) = rest.split_at(rest.len().checked_sub(5)?);
    let sign = match offset.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let offset_minutes =
        offset[1..3].parse::<i64>().ok()? * 60 + offset[3..5].parse::<i64>().ok()?;
    let (hms, fraction) = clock.split_once('.').unwrap_or((clock, "0"));
    let mut parts = hms.split(':').map(str::parse::<i64>);
    let (hour, minute, second) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    let fraction: f64 = format!("0.{fraction}").parse().ok()?;
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
        - sign * offset_minutes * 60;
    #[allow(clippy::cast_precision_loss)] // epoch seconds fit an f64's mantissa exactly
    Some(seconds as f64 + fraction)
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The crash report the system wrote for this exit, if there is one: an
/// `.ips` in `~/Library/Logs/DiagnosticReports` — where simulator crashes land
/// too — whose header names the bundle id and whose body names the pid. Only
/// reports written since `not_before` are opened, which keeps a directory of
/// hundreds of reports to the few that could match.
#[must_use]
pub fn crash_report(exit: &Exit, not_before: Option<SystemTime>) -> Option<PathBuf> {
    let pid = exit.pid?;
    let dir = sweetpad_core::paths::home_dir()?.join("Library/Logs/DiagnosticReports");
    let mut matches: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "ips"))
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            not_before
                .is_none_or(|t| modified >= t)
                .then(|| (modified, e.path()))
        })
        .filter(|(_, path)| {
            std::fs::read_to_string(path).is_ok_and(|text| report_is(&text, &exit.bundle_id, pid))
        })
        .collect();
    matches.sort();
    matches.pop().map(|(_, path)| path)
}

/// Whether an `.ips` crash report's text is the one for `bundle_id`'s `pid`:
/// a one-line JSON header (`bundleID`) followed by a JSON body (`pid`).
fn report_is(text: &str, bundle_id: &str, pid: u32) -> bool {
    let Some((header, body)) = text.split_once('\n') else {
        return false;
    };
    let named = serde_json::from_str::<serde_json::Value>(header)
        .ok()
        .is_some_and(|h| h.get("bundleID").and_then(serde_json::Value::as_str) == Some(bundle_id));
    named
        && serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .is_some_and(|b| {
                b.get("pid").and_then(serde_json::Value::as_u64) == Some(u64::from(pid))
            })
}

/// Every crash report the system wrote since `not_before` for one of
/// `bundle_ids` (any app when empty) on `source`, as the exit it records,
/// oldest first. Simulator reports land in the same directory as the Mac's;
/// the process path tells them apart, since a simulator app runs out of
/// `CoreSimulator/Devices/<udid>/`. Only files written since `not_before` are
/// opened, and a report captured before it is dropped.
#[must_use]
pub fn crash_reports(
    source: &Source,
    bundle_ids: &[&str],
    not_before: Option<SystemTime>,
) -> Vec<(Exit, PathBuf)> {
    let Some(dir) =
        sweetpad_core::paths::home_dir().map(|h| h.join("Library/Logs/DiagnosticReports"))
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let since = not_before.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok());
    let mut found: Vec<(Exit, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "ips"))
        .filter(|e| {
            not_before.is_none_or(|t| {
                e.metadata()
                    .and_then(|m| m.modified())
                    .is_ok_and(|m| m >= t)
            })
        })
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            let exit = report_exit(&text, source, bundle_ids)?;
            since
                .is_none_or(|s| exit.epoch_seconds().is_some_and(|t| t >= s.as_secs_f64()))
                .then(|| (exit, e.path()))
        })
        .collect();
    found.sort_by(|(a, _), (b, _)| by_time(a, b));
    found
}

/// The exit an `.ips` crash report records, when it belongs to one of
/// `bundle_ids` (any app when empty) on `source`: the signal that ended the
/// process and who sent it, the exception type when it says more than
/// `EXC_CRASH` does (a fault's `EXC_BAD_ACCESS` and address, a Swift trap's
/// `EXC_BREAKPOINT`), and how long the process ran. The report is a one-line
/// JSON header (`bundleID`) followed by a JSON body.
fn report_exit(text: &str, source: &Source, bundle_ids: &[&str]) -> Option<Exit> {
    use serde_json::Value;
    let str_of = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    let (header, body) = text.split_once('\n')?;
    let header: Value = serde_json::from_str(header).ok()?;
    let bundle_id = str_of(&header, "bundleID")?;
    if !bundle_ids.is_empty() && !bundle_ids.contains(&bundle_id.as_str()) {
        return None;
    }
    let body: Value = serde_json::from_str(body).ok()?;
    let path = str_of(&body, "procPath").unwrap_or_default();
    let on_this_source = match source {
        Source::Simulator(udid) => path.contains(&format!("/CoreSimulator/Devices/{udid}/")),
        Source::Mac => !path.contains("/CoreSimulator/Devices/"),
    };
    if !on_this_source {
        return None;
    }
    let pid = body
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok())?;
    let null = Value::Null;
    let exception = body.get("exception").unwrap_or(&null);
    let termination = body.get("termination").unwrap_or(&null);
    let namespace = str_of(termination, "namespace");
    let indicator = str_of(termination, "indicator");
    let name = str_of(exception, "signal").or_else(|| {
        (namespace.as_deref() == Some("SIGNAL"))
            .then(|| termination.get("code").and_then(Value::as_i64))
            .flatten()
            .and_then(|n| i32::try_from(n).ok())
            .map(signal_name)
    })?;
    let sent_by = str_of(termination, "byProc").map(|by| {
        match termination.get("byPid").and_then(Value::as_u64) {
            Some(by_pid) => format!("{by}[{by_pid}]"),
            None => by,
        }
    });
    let explanation = match namespace.as_deref() {
        // A signal's own indicator ("Abort trap: 6") repeats the signal; the
        // exception type is what adds to it, unless it is the generic one.
        Some("SIGNAL") | None => str_of(exception, "type")
            .filter(|t| t != "EXC_CRASH")
            .map(|t| match str_of(exception, "subtype") {
                Some(subtype) => format!("{t} {subtype}"),
                None => t,
            }),
        Some(other) => Some(match &indicator {
            Some(indicator) => format!("{other} {indicator}"),
            None => other.to_string(),
        }),
    };
    let captured = str_of(&body, "captureTime").map(|t| log_style_time(&t))?;
    let ran_for_ms = str_of(&body, "procLaunch").and_then(|launched| {
        let secs = epoch_seconds(&captured)? - epoch_seconds(&log_style_time(&launched))?;
        millis(secs)
    });
    Some(Exit {
        time: captured,
        bundle_id,
        pid: Some(pid),
        cause: Cause::Signal { name, sent_by },
        explanation,
        ran_for_ms,
        message: indicator.unwrap_or_default(),
        origin: Origin::CrashReport,
    })
}

/// A crash report's `2026-09-26 19:29:39.3528 +0200` in the unified log's
/// spelling, `2026-09-26 19:29:39.3528+0200`, so one parser reads both.
fn log_style_time(time: &str) -> String {
    match time.rsplit_once(' ') {
        Some((clock, offset)) if offset.starts_with(['+', '-']) => format!("{clock}{offset}"),
        _ => time.to_string(),
    }
}

/// The exit sweetpad recorded for a macOS app it spawned and waited on, or
/// `None` for a record that names neither a status nor a signal.
#[must_use]
pub fn from_record(record: &RecordedExit) -> Option<Exit> {
    let cause = match (record.status, record.signal) {
        (Some(status), _) => Cause::Status(status),
        (None, Some(signal)) => Cause::Signal {
            name: signal_name(signal),
            sent_by: record.sent_by.clone(),
        },
        (None, None) => return None,
    };
    Some(Exit {
        time: log_timestamp(record.ended),
        bundle_id: record.bundle_identifier.clone(),
        pid: Some(record.pid),
        cause,
        explanation: None,
        ran_for_ms: millis(record.ended - record.started),
        message: "recorded by sweetpad, the process's parent".to_string(),
        origin: Origin::Sweetpad,
    })
}

/// A span in seconds as whole milliseconds; `None` when it is negative.
fn millis(secs: f64) -> Option<u64> {
    let span = Duration::try_from_secs_f64(secs).ok()?;
    u64::try_from(span.as_millis()).ok()
}

/// A signal's name from its number, as macOS numbers them; `signal N` past the
/// table.
#[must_use]
pub fn signal_name(signal: i32) -> String {
    const NAMES: [&str; 31] = [
        "SIGHUP",
        "SIGINT",
        "SIGQUIT",
        "SIGILL",
        "SIGTRAP",
        "SIGABRT",
        "SIGEMT",
        "SIGFPE",
        "SIGKILL",
        "SIGBUS",
        "SIGSEGV",
        "SIGSYS",
        "SIGPIPE",
        "SIGALRM",
        "SIGTERM",
        "SIGURG",
        "SIGSTOP",
        "SIGTSTP",
        "SIGCONT",
        "SIGCHLD",
        "SIGTTIN",
        "SIGTTOU",
        "SIGIO",
        "SIGXCPU",
        "SIGXFSZ",
        "SIGVTALRM",
        "SIGPROF",
        "SIGWINCH",
        "SIGINFO",
        "SIGUSR1",
        "SIGUSR2",
    ];
    usize::try_from(signal)
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| NAMES.get(i))
        .map_or_else(|| format!("signal {signal}"), |name| (*name).to_string())
}

/// Seconds since the epoch as a local unified-log timestamp,
/// `2026-09-26 17:33:10.087799+0200`, so a recorded exit reads and sorts like
/// launchd's.
fn log_timestamp(epoch: f64) -> String {
    let at = Duration::try_from_secs_f64(epoch).unwrap_or_default();
    let t: libc::time_t = i64::try_from(at.as_secs()).unwrap_or(i64::MAX);
    // SAFETY: `localtime_r` fills the caller-owned `tm` from `t`; the reentrant
    // form, since exits are recorded from session threads.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&raw const t, &raw mut tm);
    }
    let offset = tm.tm_gmtoff / 60;
    let sign = if offset < 0 { '-' } else { '+' };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06}{sign}{:02}{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        at.subsec_micros(),
        offset.abs() / 60,
        offset.abs() % 60,
    )
}

/// How far apart two accounts of one exit can be timed. A crash report is
/// captured at the fault, launchd logs the exit once the report is written,
/// and sweetpad notices at its next poll: seconds apart, while a pid is not
/// reused that fast.
const SAME_EXIT_WITHIN_SECS: f64 = 15.0;

/// Whether two accounts describe the same termination: one pid, close in time.
fn same_exit(a: &Exit, b: &Exit) -> bool {
    a.pid.is_some()
        && a.pid == b.pid
        && match (a.epoch_seconds(), b.epoch_seconds()) {
            (Some(x), Some(y)) => (x - y).abs() <= SAME_EXIT_WITHIN_SECS,
            _ => true,
        }
}

fn by_time(a: &Exit, b: &Exit) -> std::cmp::Ordering {
    let at = |e: &Exit| e.epoch_seconds().unwrap_or_default();
    at(a).total_cmp(&at(b))
}

/// The three accounts of an app's exits as one list, oldest first, each with
/// the crash report behind it when there is one. A termination two accounts
/// saw appears once: launchd's line over sweetpad's record over the crash
/// report's reading. The report's path is carried onto whichever it matched,
/// along with the sender and exception the other account lacks.
#[must_use]
pub fn merge(
    launchd: Vec<Exit>,
    recorded: Vec<Exit>,
    reports: Vec<(Exit, PathBuf)>,
) -> Vec<(Exit, Option<PathBuf>)> {
    let mut merged: Vec<(Exit, Option<PathBuf>)> =
        launchd.into_iter().map(|exit| (exit, None)).collect();
    for exit in recorded {
        if !merged.iter().any(|(seen, _)| same_exit(seen, &exit)) {
            merged.push((exit, None));
        }
    }
    for (exit, path) in reports {
        match merged.iter_mut().find(|(seen, _)| same_exit(seen, &exit)) {
            Some((seen, report)) => {
                report.get_or_insert(path);
                if let (
                    Cause::Signal { name, sent_by },
                    Cause::Signal {
                        name: reported,
                        sent_by: reported_by,
                    },
                ) = (&mut seen.cause, exit.cause)
                    && *name == reported
                    && sent_by.is_none()
                {
                    *sent_by = reported_by;
                }
                if seen.explanation.is_none() {
                    seen.explanation = exit.explanation;
                }
            }
            None => merged.push((exit, Some(path))),
        }
    }
    merged.sort_by(|(a, _), (b, _)| by_time(a, b));
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "dev.sweetpad.exitprobe.app";

    /// A captured `log show --style ndjson` entry, trimmed to the fields read.
    fn entry(timestamp: &str, subsystem: &str, message: &str) -> String {
        serde_json::json!({
            "timestamp": timestamp,
            "subsystem": subsystem,
            "eventMessage": message,
        })
        .to_string()
    }

    fn parse(subsystem: &str, message: &str) -> Exit {
        parse_ndjson_line(
            &entry("2026-09-26 17:31:15.990409+0200", subsystem, message),
            &[APP],
        )
        .expect("an exit line")
    }

    /// An exit read from its message alone, for lines captured from other
    /// jobs: the parse of the message does not depend on whose job it was.
    fn exit_of(message: &str) -> Exit {
        let (cause, explanation, ran_for_ms) = parse_message(message).expect("an exit message");
        Exit {
            time: String::new(),
            bundle_id: APP.into(),
            pid: None,
            cause,
            explanation,
            ran_for_ms,
            message: message.into(),
            origin: Origin::Launchd,
        }
    }

    const SIM_JOB: &str =
        "user/503/UIKitApplication:dev.sweetpad.exitprobe.app[0959][rb-legacy] [57171]";

    #[test]
    fn a_simctl_terminate_is_a_springboard_reason_with_its_explanation() {
        // `xcrun simctl terminate`, iOS 27 simulator — the line the incident
        // behind this module turned on.
        let exit = parse(
            SIM_JOB,
            "exited with exit reason (namespace: 10 code: 0xfbfbfbfb) - OS_REASON_SPRINGBOARD | \
             <RBSTerminateContext| domain:10 code:0xFBFBFBFB explanation:Termination requested by \
             simulator host\n\nProcessVisibility: Foreground\nProcessState: Running \
             reportType:None maxTerminationResistance:Interactive>, ran for 3754ms",
        );
        assert_eq!(exit.pid, Some(57171));
        assert_eq!(
            exit.cause,
            Cause::Reason {
                namespace: 10,
                code: 0xfbfb_fbfb,
                name: "OS_REASON_SPRINGBOARD".into()
            }
        );
        assert_eq!(
            exit.explanation.as_deref(),
            Some("Termination requested by simulator host")
        );
        assert_eq!(exit.ran_for_ms, Some(3754));
        assert_eq!(exit.label(), None);
        assert_eq!(exit.code().as_deref(), Some("0xfbfbfbfb"));
        assert_eq!(
            exit.summary(),
            "Termination requested by simulator host (OS_REASON_SPRINGBOARD 0xfbfbfbfb)"
        );
    }

    #[test]
    fn a_context_without_a_domain_still_yields_its_explanation() {
        // A poster extension RunningBoard ended, iOS 27 simulator.
        let exit = exit_of(
            "exited with exit reason (namespace: 15 code: 0x0) - OS_REASON_RUNNINGBOARD | \
             <RBSTerminateContext| explanation:Provider tracker invalidated reportType:None \
             maxTerminationResistance:Interactive>, ran for 37710ms",
        );
        assert_eq!(exit.reason(), "OS_REASON_RUNNINGBOARD");
        assert_eq!(
            exit.explanation.as_deref(),
            Some("Provider tracker invalidated")
        );
        assert_eq!(
            exit.summary(),
            "Provider tracker invalidated (OS_REASON_RUNNINGBOARD 0x0)"
        );
    }

    #[test]
    fn a_watchdog_reason_keeps_its_own_name() {
        // A widget extension the watchdog ended, iOS 27 simulator.
        let exit = exit_of(
            "exited with exit reason (namespace: 20 code: 0x3e9) - OS_REASON_WATCHDOG | \
             <RBSTerminateContext| domain:20 code:0x000003E9 \
             explanation:ChronodWidgetExtensionWatchdog reportType:None \
             maxTerminationResistance:Interactive>, ran for 10636ms",
        );
        assert_eq!(exit.code().as_deref(), Some("0x3e9"));
        assert_eq!(exit.label(), None);
        assert_eq!(
            exit.summary(),
            "ChronodWidgetExtensionWatchdog (OS_REASON_WATCHDOG 0x3e9)"
        );
    }

    #[test]
    fn a_sigkill_names_its_sender_and_is_not_a_crash() {
        // `kill -9` of the simulator app's pid from a host shell.
        let exit = parse(
            SIM_JOB,
            "exited due to SIGKILL | sent by bash[59141], ran for 3030ms",
        );
        assert_eq!(
            exit.cause,
            Cause::Signal {
                name: "SIGKILL".into(),
                sent_by: Some("bash[59141]".into())
            }
        );
        assert!(!exit.is_crash());
        assert_eq!(exit.label(), None);
        assert_eq!(exit.summary(), "SIGKILL (sent by bash[59141])");
    }

    #[test]
    fn an_abort_is_a_crash_sent_by_the_process_itself() {
        // The app calling `abort()`.
        let exit = parse(
            SIM_JOB,
            "exited due to SIGABRT | sent by ExitProbe[59571], ran for 1351ms",
        );
        assert!(exit.is_crash());
        assert_eq!(exit.label().as_deref(), Some("crashed with SIGABRT"));
        assert_eq!(
            exit.summary(),
            "crashed with SIGABRT (sent by ExitProbe[59571])"
        );
    }

    #[test]
    fn a_fault_is_sent_by_the_exception_handler() {
        // A write through a bad pointer; the sender's name holds a space.
        let exit = parse(
            SIM_JOB,
            "exited due to SIGSEGV | sent by exc handler[60122], ran for 1366ms",
        );
        assert_eq!(
            exit.cause,
            Cause::Signal {
                name: "SIGSEGV".into(),
                sent_by: Some("exc handler[60122]".into())
            }
        );
        assert_eq!(exit.label().as_deref(), Some("crashed with SIGSEGV"));
    }

    #[test]
    fn a_signal_keeps_the_circumstance_after_its_sender() {
        // A Metal compiler service reaped with its host, iOS 27 simulator.
        let exit = exit_of(
            "exited due to SIGKILL | sent by launchd_sim[36695] during teardown of \
             process-scoped services after host exited, ran for 35353ms",
        );
        assert_eq!(
            exit.explanation.as_deref(),
            Some("during teardown of process-scoped services after host exited")
        );
        assert_eq!(
            exit.summary(),
            "SIGKILL (sent by launchd_sim[36695] during teardown of process-scoped services \
             after host exited)"
        );
    }

    #[test]
    fn a_normal_exit_reads_as_its_status() {
        let clean = parse(SIM_JOB, "exited due to exit(0), ran for 1363ms");
        assert_eq!(clean.cause, Cause::Status(0));
        assert_eq!(clean.summary(), "exited normally");
        let failed = parse(SIM_JOB, "exited due to exit(3), ran for 1361ms");
        assert_eq!(failed.label().as_deref(), Some("exited with status 3"));
        assert_eq!(failed.reason(), "exit(3)");
        // A daemon at simulator shutdown.
        let shutdown = exit_of("exited due to exit(0) during system shutdown, ran for 47938ms");
        assert_eq!(
            shutdown.summary(),
            "exited normally (during system shutdown)"
        );
    }

    #[test]
    fn a_macos_app_is_matched_by_its_launch_services_label() {
        // `open -n ExitProbeMac.app --args -die abort`, macOS 27.
        let line = entry(
            "2026-09-26 17:35:20.017071+0200",
            "gui/503/application.dev.sweetpad.exitprobe.mac.263468718.263468848.\
             15213BAD-824A-4875-9CD8-39DDC4F69375 [61476]",
            "exited due to SIGABRT | sent by ExitProbeMac[61476], ran for 1305ms",
        );
        let exit = parse_ndjson_line(&line, &["dev.sweetpad.exitprobe.mac"]).expect("an exit");
        assert_eq!(exit.pid, Some(61476));
        assert_eq!(
            parse_ndjson_line(&line, &[])
                .map(|e| e.bundle_id)
                .as_deref(),
            Some("dev.sweetpad.exitprobe.mac")
        );
        assert_eq!(exit.label().as_deref(), Some("crashed with SIGABRT"));
        // A bundle id that is only a prefix of the label's is someone else's.
        assert!(parse_ndjson_line(&line, &["dev.sweetpad.exitprobe"]).is_none());
        // A LaunchServices label without the trailing UUID, macOS 27.
        assert_eq!(
            job_bundle_id("gui/503/application.com.cmuxterm.app.243196168.243196175 [40940]"),
            Some("com.cmuxterm.app")
        );
    }

    #[test]
    fn only_the_named_bundles_jobs_match() {
        let runner = entry(
            "2026-09-26 17:37:00.140938+0200",
            "user/503/UIKitApplication:dev.sweetpad.exitprobe.uitests.xctrunner[2292][rb-legacy] \
             [62512]",
            "exited due to exit(1), ran for 32992ms",
        );
        assert!(parse_ndjson_line(&runner, &[APP]).is_none());
        let exit = parse_ndjson_line(&runner, &[APP, "dev.sweetpad.exitprobe.uitests.xctrunner"])
            .expect("the runner's exit");
        assert_eq!(exit.bundle_id, "dev.sweetpad.exitprobe.uitests.xctrunner");
        // No bundle ids means every app job.
        assert!(parse_ndjson_line(&runner, &[]).is_some());
        let daemon = entry(
            "2026-09-26 16:30:42.359386+0200",
            "user/503/com.apple.migrationpluginwrapper [31332]",
            "exited due to exit(0), ran for 26227ms",
        );
        assert!(parse_ndjson_line(&daemon, &[]).is_none());
        assert!(parse_ndjson_line(r#"{"count":8,"finished":1}"#, &[APP]).is_none());
        assert!(parse_ndjson_line("getpwuid_r did not find a match for uid 503", &[APP]).is_none());
    }

    #[test]
    fn well_known_codes_get_labels_and_others_do_not() {
        let reason = |namespace, code| Exit {
            time: String::new(),
            bundle_id: APP.into(),
            pid: None,
            cause: Cause::Reason {
                namespace,
                code,
                name: String::new(),
            },
            explanation: None,
            ran_for_ms: None,
            message: String::new(),
            origin: Origin::Launchd,
        };
        assert_eq!(
            reason(1, 0x6).label().as_deref(),
            Some("killed for memory pressure (jetsam)")
        );
        assert_eq!(
            reason(10, 0x8bad_f00d).label().as_deref(),
            Some("watchdog: took too long to launch, resume, or respond")
        );
        assert_eq!(
            reason(10, 0xdead_10cc).label().as_deref(),
            Some("held a file or database lock while suspended")
        );
        assert_eq!(
            reason(2, 6).label().as_deref(),
            Some("crashed with SIGABRT")
        );
        assert!(reason(2, 6).is_crash());
        assert_eq!(reason(2, 9).label().as_deref(), Some("ended by signal 9"));
        assert!(!reason(2, 9).is_crash());
        assert_eq!(reason(10, 0xfbfb_fbfb).label(), None);
    }

    #[test]
    fn the_json_form_has_one_shape_for_every_cause() {
        let exit = parse(
            SIM_JOB,
            "exited due to SIGABRT | sent by ExitProbe[59571], ran for 1351ms",
        );
        let json = exit.json(Some(Path::new("/tmp/ExitProbe-2026-09-26-173313.ips")));
        assert_eq!(json["kind"], "signal");
        assert_eq!(json["reason"], "SIGABRT");
        assert_eq!(json["sentBy"], "ExitProbe[59571]");
        assert_eq!(json["label"], "crashed with SIGABRT");
        assert_eq!(json["crashReport"], "/tmp/ExitProbe-2026-09-26-173313.ips");
        assert!(json["namespace"].is_null() && json["code"].is_null());
        let status = parse(SIM_JOB, "exited due to exit(3), ran for 1361ms").json(None);
        assert_eq!(status["kind"], "exit");
        assert_eq!(status["exitStatus"], 3);
    }

    #[test]
    fn a_log_timestamp_becomes_epoch_seconds() {
        // The same instant the result bundle recorded as 1790436998.87.
        let t = epoch_seconds("2026-09-26 17:36:38.871593+0200").expect("parses");
        assert!((t - 1_790_436_998.871_593).abs() < 1e-3, "{t}");
        let west = epoch_seconds("2026-09-26 08:36:38.871593-0700").expect("parses");
        assert!((west - t).abs() < 1e-3, "{west}");
        assert!(epoch_seconds("not a time").is_none());
    }

    #[test]
    fn a_crash_report_is_matched_by_bundle_and_pid() {
        let report = "{\"app_name\":\"ExitProbe\",\"bundleID\":\"dev.sweetpad.exitprobe.app\",\
                      \"bug_type\":\"309\"}\n{\n  \"pid\" : 59571,\n  \"procName\" : \"ExitProbe\"\n}";
        assert!(report_is(report, APP, 59571));
        assert!(!report_is(report, APP, 59572));
        assert!(!report_is(report, "dev.sweetpad.other", 59571));
        assert!(!report_is("not a report", APP, 59571));
    }

    #[test]
    fn the_query_narrows_to_launchd_and_the_bundle() {
        let (program, args) = log_show_command(
            &Source::Simulator("UDID"),
            &[APP],
            &Window::Between {
                start: 100.4,
                end: 200.2,
            },
        );
        assert_eq!(program, "xcrun");
        assert_eq!(&args[..4], ["simctl", "spawn", "UDID", "log"]);
        assert!(args.windows(2).any(|w| w == ["--start", "@100"]));
        assert!(args.windows(2).any(|w| w == ["--end", "@201"]));
        let predicate = args.last().expect("a predicate");
        assert_eq!(
            predicate,
            "process == \"launchd_sim\" AND eventMessage BEGINSWITH \"exited \" AND \
             (subsystem CONTAINS \"UIKitApplication:dev.sweetpad.exitprobe.app\")"
        );
        let (_, args) = log_show_command(&Source::Simulator("UDID"), &[], &Window::Last("1m"));
        assert!(
            args.last()
                .is_some_and(|p| p.ends_with("(subsystem CONTAINS \"UIKitApplication:\")"))
        );
        let (program, args) = log_show_command(&Source::Mac, &["a.b", "c.d"], &Window::Last("10m"));
        assert_eq!(program, "log");
        assert!(args.windows(2).any(|w| w == ["--last", "10m"]));
        assert!(args.last().is_some_and(|p| p.contains(
            "(subsystem CONTAINS \"application.a.b\" OR subsystem CONTAINS \"application.c.d\")"
        )));
    }

    /// A macOS app's `abort()`, captured on macOS 27 from an app sweetpad
    /// spawned directly (launchd logged no exit line for it), trimmed to the
    /// fields read.
    const MAC_ABORT_IPS: &str = r#"{"app_name":"PersistProbe","timestamp":"2026-09-26 19:29:40.00 +0200","platform":1,"bundleID":"dev.sweetpad.b2app.persistb3","bug_type":"309","os_version":"macOS 27.0 (26A428)","name":"PersistProbe"}
{
  "pid" : 43573,
  "procName" : "PersistProbe",
  "procPath" : "\/Users\/USER\/Library\/Developer\/Xcode\/DerivedData\/PersistProbe-fjzlintwqtbtdwdtshtsqveulyiq\/Build\/Products\/Debug\/PersistProbe.app\/Contents\/MacOS\/PersistProbe",
  "parentPid" : 1,
  "parentProc" : "launchd",
  "captureTime" : "2026-09-26 19:29:39.3528 +0200",
  "procLaunch" : "2026-09-26 19:29:38.7361 +0200",
  "exception" : {"codes":"0x0000000000000000, 0x0000000000000000","rawCodes":[0,0],"type":"EXC_CRASH","signal":"SIGABRT"},
  "termination" : {"flags":0,"code":6,"namespace":"SIGNAL","indicator":"Abort trap: 6","byProc":"PersistProbe","byPid":43573}
}"#;

    /// A null dereference in a simulator app, iOS 27 simulator, trimmed.
    const SIM_FAULT_IPS: &str = r#"{"app_name":"ExitProbe","timestamp":"2026-09-26 17:33:30.00 +0200","platform":7,"bundleID":"dev.sweetpad.exitprobe.app","bug_type":"309","os_version":"macOS 27.0 (26A428)","name":"ExitProbe"}
{
  "pid" : 60122,
  "procName" : "ExitProbe",
  "procPath" : "\/Users\/USER\/Library\/Developer\/CoreSimulator\/Devices\/554432FC-9FD7-4A7F-8217-46BAEA425896\/data\/Containers\/Bundle\/Application\/3B250438-8B5C-4636-BD2C-4AB7229BF846\/ExitProbe.app\/ExitProbe",
  "parentProc" : "launchd_sim",
  "captureTime" : "2026-09-26 17:33:29.9195 +0200",
  "procLaunch" : "2026-09-26 17:33:28.4565 +0200",
  "exception" : {"codes":"0x0000000000000001, 0x0000000000000010","rawCodes":[1,16],"type":"EXC_BAD_ACCESS","signal":"SIGSEGV","subtype":"KERN_INVALID_ADDRESS at 0x0000000000000010"},
  "termination" : {"flags":0,"code":11,"namespace":"SIGNAL","indicator":"Segmentation fault: 11","byProc":"exc handler","byPid":60122}
}"#;

    const SIM_UDID: &str = "554432FC-9FD7-4A7F-8217-46BAEA425896";

    #[test]
    fn a_crash_report_reads_as_the_signal_that_ended_the_app() {
        let exit = report_exit(MAC_ABORT_IPS, &Source::Mac, &[]).expect("a crash");
        assert_eq!(exit.bundle_id, "dev.sweetpad.b2app.persistb3");
        assert_eq!(exit.pid, Some(43573));
        assert_eq!(exit.origin, Origin::CrashReport);
        assert_eq!(exit.time, "2026-09-26 19:29:39.3528+0200");
        assert_eq!(exit.ran_for_ms, Some(616));
        // `EXC_CRASH` is how every abort reads, so it adds nothing.
        assert_eq!(exit.explanation, None);
        assert_eq!(
            exit.summary(),
            "crashed with SIGABRT (sent by PersistProbe[43573])"
        );
        assert_eq!(exit.json(None)["source"], "crashReport");
        // A Mac app's report is not a simulator's, nor another bundle's.
        assert!(report_exit(MAC_ABORT_IPS, &Source::Simulator(SIM_UDID), &[]).is_none());
        assert!(report_exit(MAC_ABORT_IPS, &Source::Mac, &["dev.sweetpad.other"]).is_none());
    }

    #[test]
    fn a_simulator_fault_keeps_its_exception_and_its_device() {
        let exit = report_exit(
            SIM_FAULT_IPS,
            &Source::Simulator(SIM_UDID),
            &["dev.sweetpad.exitprobe.app"],
        )
        .expect("a crash");
        assert_eq!(
            exit.summary(),
            "crashed with SIGSEGV (sent by exc handler[60122] EXC_BAD_ACCESS \
             KERN_INVALID_ADDRESS at 0x0000000000000010)"
        );
        assert_eq!(exit.ran_for_ms, Some(1463));
        assert!(report_exit(SIM_FAULT_IPS, &Source::Simulator("OTHER-UDID"), &[]).is_none());
        assert!(report_exit(SIM_FAULT_IPS, &Source::Mac, &[]).is_none());
    }

    #[test]
    fn a_report_ended_outside_the_signal_namespace_says_which() {
        // tccd's real termination block (a SIGTERM that timed out, iOS 27
        // simulator) under an app's header: tccd's own report carries no
        // bundle id, so it is no app's crash at all.
        let body = r#"{
  "pid" : 2666,
  "procPath" : "\/Library\/Developer\/CoreSimulator\/Volumes\/iOS_24A5\/usr\/libexec\/tccd",
  "captureTime" : "2026-09-26 18:49:01.7371 +0200",
  "procLaunch" : "2026-09-26 18:48:31.8163 +0200",
  "exception" : {"codes":"0x0000000000000000, 0x0000000000000000","rawCodes":[0,0],"type":"EXC_CRASH","signal":"SIGKILL"},
  "termination" : {"flags":6,"code":4,"namespace":"LIBXPC","indicator":"XPC_EXIT_REASON_SIGTERM_TIMEOUT"}
}"#;
        let tccd = format!(
            "{}\n{body}",
            r#"{"app_name":"tccd","platform":7,"bug_type":"309","name":"tccd"}"#
        );
        assert!(report_exit(&tccd, &Source::Mac, &[]).is_none());
        let app = format!("{}\n{body}", r#"{"bundleID":"dev.sweetpad.app"}"#);
        let exit = report_exit(&app, &Source::Mac, &[]).expect("an exit");
        assert_eq!(
            exit.summary(),
            "SIGKILL (LIBXPC XPC_EXIT_REASON_SIGTERM_TIMEOUT)"
        );
        assert!(!exit.is_crash());
    }

    fn recorded(pid: u32, ended: f64, status: Option<i32>, signal: Option<i32>) -> RecordedExit {
        RecordedExit {
            bundle_identifier: "dev.sweetpad.app".into(),
            pid,
            started: ended - 2.5,
            ended,
            status,
            signal,
            sent_by: None,
        }
    }

    #[test]
    fn a_recorded_exit_reads_like_launchds() {
        let ended = 1_790_443_780.25;
        let exit = from_record(&recorded(44746, ended, Some(3), None)).expect("an exit");
        assert_eq!(exit.summary(), "exited with status 3");
        assert_eq!(exit.ran_for_ms, Some(2500));
        assert_eq!(exit.origin, Origin::Sweetpad);
        // The local timestamp reads back as the same instant.
        let back = exit.epoch_seconds().expect("a log-style time");
        assert!((back - ended).abs() < 1e-3, "{} → {back}", exit.time);

        let crash = from_record(&recorded(1, ended, None, Some(6))).expect("an exit");
        assert_eq!(crash.summary(), "crashed with SIGABRT");
        assert!(crash.is_crash());
        let mut killed = recorded(2, ended, None, Some(9));
        killed.sent_by = Some("sweetpad".into());
        let killed = from_record(&killed).expect("an exit");
        assert_eq!(killed.summary(), "SIGKILL (sent by sweetpad)");
        assert_eq!(killed.json(None)["source"], "sweetpad");
        assert!(from_record(&recorded(3, ended, None, None)).is_none());
        assert_eq!(signal_name(15), "SIGTERM");
        assert_eq!(signal_name(40), "signal 40");
    }

    #[test]
    fn one_termination_seen_twice_is_listed_once() {
        let at = |epoch: f64| log_timestamp(epoch);
        let launchd = |pid: u32, epoch: f64| Exit {
            time: at(epoch),
            pid: Some(pid),
            ..exit_of("exited due to SIGABRT | sent by App[1], ran for 900ms")
        };
        let report = |pid: u32, epoch: f64, name: &str| {
            let mut exit = from_record(&recorded(pid, epoch, None, Some(6))).expect("an exit");
            exit.origin = Origin::CrashReport;
            (exit, PathBuf::from(name))
        };
        let merged = merge(
            vec![launchd(100, 1000.0)],
            vec![
                // launchd saw this one too: launchd's line wins.
                from_record(&recorded(100, 1001.0, None, Some(6))).expect("an exit"),
                // Only sweetpad saw this one.
                from_record(&recorded(200, 2000.0, Some(3), None)).expect("an exit"),
            ],
            vec![
                report(100, 999.5, "a.ips"),
                report(200, 2000.2, "b.ips"),
                // A crash only its report knows of.
                report(300, 1500.0, "c.ips"),
                // Pid 100 again, much later: a different process.
                report(100, 5000.0, "d.ips"),
            ],
        );
        let seen: Vec<(Option<u32>, Origin, Option<&str>)> = merged
            .iter()
            .map(|(exit, path)| {
                (
                    exit.pid,
                    exit.origin,
                    path.as_deref().and_then(Path::to_str),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                (Some(100), Origin::Launchd, Some("a.ips")),
                (Some(300), Origin::CrashReport, Some("c.ips")),
                (Some(200), Origin::Sweetpad, Some("b.ips")),
                (Some(100), Origin::CrashReport, Some("d.ips")),
            ]
        );

        // sweetpad saw the signal; the report adds who sent it and why.
        let (mut fault, path) = report(400, 3000.0, "e.ips");
        fault.cause = Cause::Signal {
            name: "SIGABRT".into(),
            sent_by: Some("App[400]".into()),
        };
        fault.explanation = Some("EXC_BAD_ACCESS KERN_INVALID_ADDRESS at 0x10".into());
        let merged = merge(
            Vec::new(),
            vec![from_record(&recorded(400, 3000.5, None, Some(6))).expect("an exit")],
            vec![(fault, path)],
        );
        let [(exit, Some(path))] = merged.as_slice() else {
            panic!("{merged:?}");
        };
        assert_eq!(exit.origin, Origin::Sweetpad);
        assert_eq!(path, Path::new("e.ips"));
        assert_eq!(
            exit.summary(),
            "crashed with SIGABRT (sent by App[400] EXC_BAD_ACCESS KERN_INVALID_ADDRESS at 0x10)"
        );
    }
}
