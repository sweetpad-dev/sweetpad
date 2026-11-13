//! Terminal Apple authentication and private session files for Linux downloads.
use std::{path::PathBuf, process::Command, process::Stdio};

use clap::{Args, Subcommand};
use serde_json::Value;

use crate::cli::{CliError, CommandResult, Context, ErrorKind, Rendered};

#[derive(Debug, Args)]
pub struct SessionArgs {
    /// Private Sweetpad session JSON. CI can mount this file read-only.
    #[arg(long, env = "SWEETPAD_APPLE_SESSION_FILE")]
    pub session_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Apple account email. The password and two-factor code use hidden TTY input.
    #[arg(long)]
    pub apple_id: String,
    #[command(flatten)]
    pub session: SessionArgs,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Sign in to Apple Developer, or validate and reuse your existing session.
    Login(LoginArgs),
    /// Remove Sweetpad's local Apple session file.
    Logout(SessionArgs),
}

pub(super) fn helper(ctx: &Context, action: &str) -> Result<Command, CliError> {
    let root = crate::cli::embedded::root()?.join("setup/sweetpad-lib/ci");
    let mut command = Command::new("python3");
    command
        .arg(root.join("linux-xcode-download.py"))
        .arg(action);
    if !ctx.out.is_interactive() {
        command.arg("--non-interactive");
    }
    command.stdin(Stdio::inherit()).stderr(Stdio::inherit());
    Ok(command)
}

pub(super) fn report(command: &mut Command) -> Result<Value, CliError> {
    let output = command.output().map_err(|error| {
        CliError::new(format!(
            "Apple authentication and downloads require python3: {error}"
        ))
        .kind(ErrorKind::ToolMissing)
    })?;
    let report: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| CliError::new("Apple helper returned an invalid report"))?;
    if !output.status.success() {
        return Err(CliError::new(
            report["error"]
                .as_str()
                .unwrap_or("Apple authentication or download failed"),
        ));
    }
    Ok(report)
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    let (name, session) = match action {
        Action::Login(args) => ("login", &args.session),
        Action::Logout(args) => ("logout", args),
    };
    let mut command = helper(ctx, name)?;
    if let Action::Login(args) = action {
        command.arg("--apple-id").arg(&args.apple_id);
    }
    if let Some(path) = &session.session_file {
        command.arg("--session-file").arg(path);
    }
    Ok(Rendered::data(super::sdk::Report(report(&mut command)?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Resource};
    use clap::Parser;

    #[test]
    fn login_accepts_session_files_but_no_password_argument() {
        let cli = Cli::try_parse_from([
            "sweetpad",
            "auth",
            "login",
            "--apple-id",
            "user@example.com",
            "--session-file",
            "/tmp/apple.json",
            "--json",
        ])
        .unwrap();
        let Some(Resource::Auth {
            action: Action::Login(args),
        }) = cli.resource
        else {
            panic!("expected Apple login");
        };
        assert_eq!(
            args.session.session_file,
            Some(PathBuf::from("/tmp/apple.json"))
        );
        assert!(
            Cli::try_parse_from([
                "sweetpad",
                "auth",
                "login",
                "--apple-id",
                "user@example.com",
                "--password",
                "secret",
            ])
            .is_err()
        );
        assert!(Cli::try_parse_from(["sweetpad", "auth", "logout", "--non-interactive"]).is_ok());
    }
}
