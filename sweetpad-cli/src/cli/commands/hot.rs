//! `sweetpad hot …` — inspect and clear the hot-reload injection listener.
//!
//! One `--hot` session owns `:8887` exclusively (CLI_DESIGN §9d). A session that
//! dies without unwinding leaves the listener bound, and every later `--hot` run
//! fails to start one with "Address already in use" until something clears it.
//! `status` says who holds the port; `reset` ends a holder that is ours.

use clap::Subcommand;

use crate::cli::inject::{protocol, server};
use crate::cli::output::Output;
use crate::cli::{CliError, CommandResult, Context, Render, Rendered, process};

/// How long to wait for a signalled holder to release the port before reporting
/// the reset as incomplete — a SIGTERM'd listener unbinds promptly or not at all.
const RELEASE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Report whether the hot-reload port is free, and who holds it.
    Status,
    /// End a leftover hot-reload listener so the next '--hot' run can bind.
    Reset {
        /// End the holder even when it isn't a sweetpad process.
        #[arg(long)]
        force: bool,
    },
}

pub fn run(_ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Status => Ok(status()),
        Action::Reset { force } => reset(*force),
    }
}

use server::Holder;

fn holders() -> Vec<Holder> {
    server::port_holders()
        .into_iter()
        .map(Holder::look_up)
        .collect()
}

fn json_holders(holders: &[Holder]) -> Vec<serde_json::Value> {
    holders
        .iter()
        .map(|h| {
            serde_json::json!({
                "pid": h.pid,
                "process": h.name,
                "sweetpad": h.ours,
            })
        })
        .collect()
}

/// `hot status`: whether the port is bindable, and who is on it when not.
struct StatusResult {
    free: bool,
    holders: Vec<Holder>,
}

impl Render for StatusResult {
    fn human(&self, out: &Output) {
        if self.free {
            out.line(&format!("hot-reload port {} is free", protocol::PORT));
            return;
        }
        let who = if self.holders.is_empty() {
            // Bound by something `lsof` won't attribute — another user's
            // process, most likely.
            "an unidentified process".to_string()
        } else {
            self.holders
                .iter()
                .map(Holder::label)
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.line(&format!(
            "hot-reload port {} is held by {who}",
            protocol::PORT
        ));
        // `reset` refuses unless *every* holder is ours, so the note keys on
        // the same condition — telling someone plain `reset` will do it when
        // it is about to refuse just costs them a round-trip.
        if let [only] = self.holders.as_slice() {
            out.note(only.how_to_end());
        } else if !self.holders.is_empty() {
            out.note(if self.holders.iter().all(|h| h.ours) {
                "run 'sweetpad hot reset' to end them"
            } else {
                "not all of them are sweetpad processes — run 'sweetpad hot reset --force' to \
                 end them anyway"
            });
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "port": protocol::PORT,
            "free": self.free,
            "holders": json_holders(&self.holders),
        })
    }
}

fn status() -> Rendered {
    let free = server::port_available();
    let holders = if free { Vec::new() } else { holders() };
    Rendered::data(StatusResult { free, holders })
}

/// `hot reset`: what was ended, and whether the port came back.
struct ResetResult {
    freed: bool,
    ended: Vec<Holder>,
}

impl Render for ResetResult {
    fn human(&self, out: &Output) {
        if self.ended.is_empty() {
            out.line(&format!(
                "hot-reload port {} was already free",
                protocol::PORT
            ));
            return;
        }
        for h in &self.ended {
            out.line(&format!("ended {}", h.label()));
        }
        if self.freed {
            out.line(&format!("hot-reload port {} is free", protocol::PORT));
        } else {
            out.note(&format!(
                "port {} is still held — the process may be unkillable or restarting",
                protocol::PORT
            ));
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "port": protocol::PORT,
            "freed": self.freed,
            "ended": json_holders(&self.ended),
        })
    }
}

fn reset(force: bool) -> CommandResult {
    if server::port_available() {
        return Ok(Rendered::data(ResetResult {
            freed: true,
            ended: Vec::new(),
        }));
    }
    let holders = holders();
    if holders.is_empty() {
        return Err(CliError::new(format!(
            "port {} is bound, but no owning process could be identified — it may belong \
             to another user",
            protocol::PORT
        )));
    }
    // Ending an unrelated listener is the one genuinely destructive thing this
    // command can do, so it takes a deliberate --force rather than a guess.
    if !force && let Some(other) = holders.iter().find(|h| !h.ours) {
        return Err(CliError::new(format!(
            "{} holds port {}, and it isn't a sweetpad process — pass '--force' to end it anyway",
            other.label(),
            protocol::PORT
        )));
    }
    for h in &holders {
        process::terminate(h.pid);
    }
    Ok(Rendered::data(ResetResult {
        freed: wait_for_release(),
        ended: holders,
    }))
}

/// Poll until the port binds again or [`RELEASE_TIMEOUT`] passes, so the result
/// reports what actually happened rather than what was signalled.
fn wait_for_release() -> bool {
    const SLICE: std::time::Duration = std::time::Duration::from_millis(50);
    let deadline = std::time::Instant::now() + RELEASE_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if server::port_available() {
            return true;
        }
        std::thread::sleep(SLICE);
    }
    server::port_available()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_own_binary_is_recognized_by_its_trailing_component() {
        let ours = Holder {
            pid: 1,
            name: Some("/opt/homebrew/bin/sweetpad".to_string()),
            ours: true,
        };
        assert_eq!(ours.label(), "sweetpad (pid 1)");
        // `look_up` derives `ours`; check the same rule it applies.
        for (name, expected) in [
            ("/opt/homebrew/bin/sweetpad", true),
            ("sweetpad", true),
            (
                "/Applications/InjectionNext.app/Contents/MacOS/InjectionNext",
                false,
            ),
            ("/usr/bin/sweetpad-helper", false),
        ] {
            let is_ours = std::path::Path::new(name)
                .file_name()
                .is_some_and(|f| f == "sweetpad");
            assert_eq!(is_ours, expected, "{name}");
        }
    }

    #[test]
    fn an_unnamed_holder_still_reads_as_a_pid() {
        let h = Holder {
            pid: 4242,
            name: None,
            ours: false,
        };
        assert_eq!(h.label(), "pid 4242");
    }

    #[test]
    fn each_holder_names_the_command_that_actually_frees_the_port() {
        // The whole value of this string is that following it works: plain
        // `reset` refuses on a non-sweetpad holder, so only one of these two
        // spellings is right for a given holder.
        let ours = Holder {
            pid: 1,
            name: Some("sweetpad".into()),
            ours: true,
        };
        assert_eq!(ours.how_to_end(), "run 'sweetpad hot reset' to end it");

        let theirs = Holder {
            pid: 2,
            name: Some("InjectionNext".into()),
            ours: false,
        };
        assert!(theirs.how_to_end().contains("'sweetpad hot reset --force'"));
        // Backticks render literally in a terminal; the fix must be copyable.
        assert!(!ours.how_to_end().contains('`'));
        assert!(!theirs.how_to_end().contains('`'));
    }
}
