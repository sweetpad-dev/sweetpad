//! `sweetpad pbxproj pif` — the workspace or project as Swift Build's project
//! interchange format, the input the open-source build engine takes in place
//! of an `.xcworkspace` or `.xcodeproj`. The counterpart of
//! `xcodebuild -dumpPIF`, which needs Xcode.

use std::fs;
use std::path::PathBuf;

use clap::Args;

use crate::cli::output::Output;
use crate::cli::resolve::{self, Container};
use crate::cli::{CliError, CommandResult, ContainerArgs, Context, Render, Rendered};

/// Flags for `pbxproj pif`.
#[derive(Debug, Args)]
pub struct PifArgs {
    #[command(flatten)]
    pub container: ContainerArgs,

    /// Write the PIF to this file instead of printing it.
    #[arg(long, value_name = "PATH")]
    pub file: Option<PathBuf>,
}

struct PifResult {
    pif: serde_json::Value,
    written: Option<PathBuf>,
}

impl Render for PifResult {
    fn human(&self, out: &Output) {
        match &self.written {
            Some(path) => out.line(&format!("Wrote PIF to {}", path.display())),
            None => out.line(&serde_json::to_string_pretty(&self.pif).unwrap_or_default()),
        }
    }

    fn json(&self) -> serde_json::Value {
        match &self.written {
            Some(path) => serde_json::json!({ "path": path }),
            None => serde_json::json!({ "pif": self.pif }),
        }
    }
}

pub fn run(ctx: &mut Context, args: &PifArgs) -> CommandResult {
    ctx.targeting = args.container.clone().into();
    let path = match resolve::container(ctx)? {
        Container::Workspace(path) | Container::Project(path) => path,
        Container::SwiftPackage(_) => {
            return Err(CliError::new(
                "a Swift package has no Xcode project to convert to PIF",
            ));
        }
    };
    let objects = sweetpad_lib::pif::pif(&path).map_err(|e| CliError::new(e.to_string()))?;
    let pif = serde_json::Value::Array(objects);
    let written = match &args.file {
        Some(path) => {
            let text =
                serde_json::to_string_pretty(&pif).map_err(|e| CliError::new(e.to_string()))?;
            fs::write(path, text)
                .map_err(|e| CliError::new(format!("cannot write {}: {e}", path.display())))?;
            Some(path.clone())
        }
        None => None,
    };
    Ok(Rendered::data(PifResult { pif, written }))
}
