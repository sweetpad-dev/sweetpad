//! Terminal Apple authentication and private session files for Linux downloads.
use std::path::PathBuf;

use crate::cli::{CliError, CommandResult, Context, Rendered, apple_account};
use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct SessionArgs {
    /// Private Sweetpad session JSON. CI can mount this file read-only.
    #[arg(long, env = "SWEETPAD_APPLE_SESSION_FILE")]
    pub session_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Apple account email. The password and verification code are asked for on the terminal.
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

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    let report = match action {
        Action::Login(args) => {
            let path = apple_account::session_path(args.session.session_file.as_deref());
            let mut note = |line: &str| ctx.out.note(line);
            apple_account::login(path, &args.apple_id, ctx.out.is_interactive(), &mut note)
        }
        Action::Logout(args) => {
            apple_account::logout(&apple_account::session_path(args.session_file.as_deref()))
        }
    };
    Ok(Rendered::data(super::sdk::Report(
        report.map_err(CliError::new)?,
    )))
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
