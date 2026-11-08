//! The structured catalog diagnostics consumed by Swift Build.
#![allow(clippy::print_stdout)]

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Default)]
struct Args {
    sources: Vec<PathBuf>,
    metadata: PathBuf,
    disabled: bool,
    app_name_override: bool,
}

#[must_use]
pub fn main(arguments: &[String]) -> ExitCode {
    let diagnostics = match parse(arguments).and_then(|args| validate(&args)) {
        Ok(diagnostics) => diagnostics,
        Err(message) => vec![json!({"path": "", "status": "error", "message": message})],
    };
    let failed = diagnostics.iter().any(|d| d["status"] == "error");
    println!("{}", serde_json::to_string(&diagnostics).unwrap());
    if failed {
        ExitCode::from(255)
    } else {
        ExitCode::SUCCESS
    }
}

fn parse(arguments: &[String]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut args = arguments.iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--source-file" => out.sources.push(super::value(flag, &mut args)?.into()),
            "--input-data-path" => out.metadata = super::value(flag, &mut args)?.into(),
            "--disable" => out.disabled = true,
            "--app-name-override" => out.app_name_override = true,
            "--validate-assistant-intents" => {}
            "--platform-family" | "--deployment-target" | "--metadata-file-list" => {
                super::value(flag, &mut args)?;
            }
            _ => {
                return Err(format!(
                    "unsupported appshortcutstringsprocessor option '{flag}'"
                ));
            }
        }
    }
    Ok(out)
}

fn validate(args: &Args) -> Result<Vec<Value>, String> {
    if args.disabled {
        return Ok(Vec::new());
    }
    if args.sources.is_empty() {
        return Err("expected --source-file".into());
    }
    let path = args.metadata.join("extract.actionsdata");
    let metadata: Value = serde_json::from_slice(
        &std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
    )
    .map_err(|e| e.to_string())?;
    let mut used = BTreeMap::new();
    for shortcut in metadata["autoShortcuts"].as_array().into_iter().flatten() {
        for phrase in shortcut["phraseTemplates"].as_array().into_iter().flatten() {
            if let Some(key) = phrase["key"].as_str() {
                used.insert(
                    key,
                    shortcut["actionIdentifier"].as_str().unwrap_or_default(),
                );
            }
        }
    }
    let mut diagnostics = Vec::new();
    for source in &args.sources {
        if !matches!(
            source.file_name().and_then(|s| s.to_str()),
            Some("AppShortcuts.strings" | "AppShortcuts.xcstrings")
        ) {
            return Err("expected a path to AppShortcuts.strings or AppShortcuts.xcstrings; AssistantIntents catalogs are not supported yet".into());
        }
        for (key, locale, text) in entries(source)? {
            let mut diagnostic =
                json!({"path": source, "key": key, "languageCode": locale, "line": 1});
            if let Some(action) = used.get(key.as_str()) {
                if let Some(message) =
                    phrase_error(&text, &metadata["actions"][*action], args.app_name_override)?
                {
                    diagnostic["status"] = "error".into();
                    diagnostic["message"] = message.into();
                    diagnostics.push(diagnostic);
                }
            } else {
                diagnostic["status"] = "warning".into();
                diagnostic["message"] =
                    "This phrase is not used in any App Shortcut or as a Negative Phrase".into();
                diagnostics.push(diagnostic);
            }
        }
        if !diagnostics
            .iter()
            .any(|d| d["path"] == json!(source) && d["status"] == "error")
        {
            diagnostics.push(json!({"path": source, "status": "success", "message": ""}));
        }
    }
    Ok(diagnostics)
}

fn phrase_error(
    text: &str,
    action: &Value,
    app_name_override: bool,
) -> Result<Option<String>, String> {
    if !app_name_override && !text.contains("${applicationName}") {
        return Ok(Some(
            "Every App Shortcut utterance should contain '${applicationName}'".into(),
        ));
    }
    let mut remaining = text;
    while let Some((_, tail)) = remaining.split_once("${") {
        let (name, tail) = tail
            .split_once('}')
            .ok_or("unterminated App Shortcut parameter")?;
        remaining = tail;
        if name == "applicationName" {
            continue;
        }
        let parameter = action["parameters"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["name"] == name);
        if parameter.is_none_or(|p| {
            !(p["valueType"]["entity"].is_object() || p["valueType"]["enum"].is_object())
        }) {
            return Ok(Some(format!(
                "'AppEntity' and 'AppEnum' are the only allowed types for '{name}'"
            )));
        }
    }
    Ok(None)
}

fn entries(source: &Path) -> Result<Vec<(String, String, String)>, String> {
    if source.extension().is_some_and(|s| s == "strings") {
        let locale = source
            .parent()
            .and_then(Path::file_stem)
            .and_then(|s| s.to_str())
            .unwrap_or("en");
        return Ok(super::training::strings(source)?
            .into_iter()
            .map(|(k, v)| (k, locale.to_string(), v))
            .collect());
    }
    let value: Value = serde_json::from_slice(&std::fs::read(source).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let strings = value["strings"]
        .as_object()
        .ok_or("String Catalog requires a strings dictionary")?;
    let mut result = Vec::new();
    for (key, entry) in strings {
        for (locale, value) in entry["localizations"].as_object().into_iter().flatten() {
            let text = value["stringUnit"]["value"]
                .as_str()
                .ok_or("App Shortcuts catalog variations are not supported")?;
            result.push((key.clone(), locale.clone(), text.to_string()));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_validation_controls_reject_missing_names_and_primitive_parameters() {
        let metadata: Value = serde_json::from_str(include_str!(
            "../../../fixtures/appintents/extract.actionsdata"
        ))
        .unwrap();
        let action = &metadata["actions"]["Greet"];
        assert_eq!(
            phrase_error("Saluer dans ${applicationName}", action, false).unwrap(),
            None
        );
        assert!(phrase_error("Saluer", action, false).unwrap().is_some());
        assert!(
            phrase_error("Saluer ${name} ${applicationName}", action, false)
                .unwrap()
                .is_some()
        );
        assert_eq!(phrase_error("Saluer", action, true).unwrap(), None);
    }
}
