//! Swift Build runs phrase training after it has extracted all app metadata.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

#[must_use]
pub fn main(args: &[String]) -> ExitCode {
    match parse_and_run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: appintentsnltrainingprocessor: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_and_run(args: &[String]) -> Result<(), String> {
    let mut product = PathBuf::new();
    let mut plist = PathBuf::new();
    let mut bundle = String::new();
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--archive-ssu-assets" | "--deployment-postprocessing" => {}
            "--product-path" => product = super::value(flag, &mut args)?.into(),
            "--infoplist-path" => plist = super::value(flag, &mut args)?.into(),
            "--bundle-id" => bundle.clone_from(super::value(flag, &mut args)?),
            "--temp-dir-path" | "--extracted-metadata-path" | "--metadata-file-list" => {
                super::value(flag, &mut args)?;
            }
            _ => {
                return Err(format!(
                    "unsupported language-model processor option '{flag}'"
                ));
            }
        }
    }
    if product.as_os_str().is_empty() || plist.as_os_str().is_empty() {
        return Err("expected --product-path and --infoplist-path".into());
    }
    run(&product, &plist, &bundle)
}

pub(super) fn run(product: &Path, plist: &Path, bundle: &str) -> Result<(), String> {
    let output = product.join("Metadata.appintents");
    let registration = output.join("extract.actionsdata");
    if !registration.exists() {
        return Ok(());
    }
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(&registration).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if metadata["autoShortcuts"]
        .as_array()
        .is_none_or(Vec::is_empty)
    {
        return Ok(());
    }
    let values = sweetpad_lib::derived_data::plist_strings(plist);
    let value = |name: &str| {
        values
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    let app = value("CFBundleDisplayName")
        .or_else(|| value("CFBundleName"))
        .ok_or("App Shortcuts require CFBundleDisplayName or CFBundleName in Info.plist")?;
    let bundle = if bundle.is_empty() {
        value("CFBundleIdentifier").ok_or("App Shortcuts require a bundle identifier")?
    } else {
        bundle
    };
    super::nlu::write(&output, bundle, app, &metadata)
}
