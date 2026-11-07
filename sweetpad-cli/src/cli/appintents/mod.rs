//! App Intents registration metadata from the Swift compiler's constant values.
//! This runs as `appintentsmetadataprocessor` in a portable Swift Build.
#![allow(clippy::print_stderr)]

mod metadata;
mod nlu;
mod training;
mod validation;

pub use training::main as training_main;
pub use validation::main as validation_main;

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Default)]
struct Args {
    output: PathBuf,
    module: String,
    bundle: String,
    version: String,
    constants: Vec<PathBuf>,
    strings: Vec<PathBuf>,
    dynamic_dependencies: Vec<PathBuf>,
    static_dependencies: Vec<PathBuf>,
    disabled: bool,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--compile-time-extraction"
            | "--deployment-aware-processing"
            | "--validate-assistant-intents"
            | "--no-app-shortcuts-localization"
            | "--app-shortcuts-app-name-override"
            | "--force"
            | "--quiet-warnings"
            | "--force-metadata-output" => {}
            "-d" | "--disable" => out.disabled = true,
            "--output" | "-o" => out.output = PathBuf::from(value(flag, &mut args)?),
            "--module-name" | "-m" => out.module.clone_from(value(flag, &mut args)?),
            "--bundle-identifier" => out.bundle.clone_from(value(flag, &mut args)?),
            "--xcode-version" | "-xcv" => out.version.clone_from(value(flag, &mut args)?),
            "--swift-const-vals-list" | "-scl" => {
                out.constants.push(PathBuf::from(value(flag, &mut args)?));
            }
            "--stringsdata-file" | "-g" => {
                out.strings.push(PathBuf::from(value(flag, &mut args)?));
            }
            "--metadata-file-list" | "-mfl" => {
                out.dynamic_dependencies
                    .push(PathBuf::from(value(flag, &mut args)?));
            }
            "--static-metadata-file-list" | "-smfl" => {
                out.static_dependencies
                    .push(PathBuf::from(value(flag, &mut args)?));
            }
            "--binary-file"
            | "--toolchain-dir"
            | "-c"
            | "--sdk-root"
            | "-r"
            | "--platform-family"
            | "-pf"
            | "--deployment-target"
            | "-dt"
            | "--target-triple"
            | "-t"
            | "--dependency-file"
            | "-p"
            | "--source-file-list"
            | "-sl" => {
                value(flag, &mut args)?;
            }
            _ => return Err(format!("unsupported metadata processor option '{flag}'")),
        }
    }
    if out.module.is_empty() || out.output.as_os_str().is_empty() {
        return Err("expected --module-name and --output".into());
    }
    Ok(out)
}

fn value<'a>(
    flag: &str,
    args: &mut impl Iterator<Item = &'a String>,
) -> Result<&'a String, String> {
    args.next().ok_or_else(|| format!("'{flag}' takes a value"))
}

#[must_use]
pub fn main(args: &[String]) -> ExitCode {
    match parse(args).and_then(|args| run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: appintentsmetadataprocessor: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    if args.disabled {
        return Ok(());
    }
    for list in &args.static_dependencies {
        let dependencies =
            std::fs::read_to_string(list).map_err(|e| format!("{}: {e}", list.display()))?;
        // Swift Build lists potential metadata paths even for dependencies
        // that have no App Intents and never create that file.
        if dependencies.lines().any(|l| Path::new(l.trim()).is_file()) {
            return Err(
                "App Intents metadata from statically linked dependency packages or frameworks is not supported yet"
                    .into(),
            );
        }
    }
    // Dynamic frameworks and embedded extensions register their own actions.
    // Their metadata supplies context; it is not copied into this product's
    // registration, as the native Ice Cubes app/widget outputs demonstrate.
    for list in &args.dynamic_dependencies {
        let dependencies =
            std::fs::read_to_string(list).map_err(|e| format!("{}: {e}", list.display()))?;
        for path in dependencies
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !Path::new(path).is_file() {
                continue;
            }
            let value: Value =
                serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("{path}: {e}"))?)
                    .map_err(|e| format!("{path}: {e}"))?;
            if value["version"] != 1 || !value["actions"].is_object() {
                return Err(format!(
                    "{path}: unsupported dynamic App Intents metadata schema"
                ));
            }
        }
    }
    let mut types = BTreeMap::new();
    for list in &args.constants {
        for file in std::fs::read_to_string(list)
            .map_err(|e| format!("{}: {e}", list.display()))?
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            let path = file.trim().replace('\\', "");
            let values: Vec<Value> =
                serde_json::from_slice(&std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?)
                    .map_err(|e| format!("{path}: {e}"))?;
            for value in values {
                if let Some(name) = value["typeName"].as_str() {
                    types.entry(name.to_string()).or_insert(value);
                }
            }
        }
    }
    let types: Vec<Value> = types.into_values().collect();
    let metadata = metadata::extract(&types)?;
    for path in &args.strings {
        if let Some(value) = metadata::strings_data(&types, &metadata) {
            write_json(path, &value)?;
        } else if path.exists() {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    let output = args.output.join("Metadata.appintents");
    if output.exists() {
        std::fs::remove_dir_all(&output).map_err(|e| e.to_string())?;
    }
    if metadata["actions"]
        .as_object()
        .is_none_or(serde_json::Map::is_empty)
        && metadata["entities"]
            .as_object()
            .is_none_or(serde_json::Map::is_empty)
        && metadata["enums"].as_array().is_none_or(Vec::is_empty)
        && metadata["queries"]
            .as_object()
            .is_none_or(serde_json::Map::is_empty)
    {
        return Ok(());
    }
    write_json(&output.join("extract.actionsdata"), &metadata)?;
    write_json(
        &output.join("version.json"),
        &json!({"toolsVersion":args.version,"version":"3.0"}),
    )?;
    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serde_json::to_vec(value).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dependency_lists_only_reject_metadata_that_actually_exists() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-appintents-dependencies");
        let dependency = dir.join("Framework/Metadata.appintents/extract.actionsdata");
        let list = dir.join("dependencies.list");
        std::fs::write(&list, dependency.display().to_string()).unwrap();
        let args = Args {
            output: dir.join("App.app"),
            static_dependencies: vec![list],
            ..Args::default()
        };
        run(&args).unwrap();
        std::fs::create_dir_all(dependency.parent().unwrap()).unwrap();
        std::fs::write(dependency, b"{}").unwrap();
        assert!(
            run(&args)
                .unwrap_err()
                .contains("statically linked dependency packages or frameworks")
        );
    }
    #[test]
    fn embedded_dynamic_metadata_keeps_its_own_registration() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-appintents-dynamic");
        let dependency = dir.join("Widget.appex/Metadata.appintents/extract.actionsdata");
        std::fs::create_dir_all(dependency.parent().unwrap()).unwrap();
        std::fs::write(
            &dependency,
            include_bytes!("../../../fixtures/appintents/Types.actionsdata"),
        )
        .unwrap();
        let dependencies = dir.join("dynamic.list");
        std::fs::write(&dependencies, dependency.display().to_string()).unwrap();
        let values = dir.join("values.swiftconstvalues");
        std::fs::write(
            &values,
            include_bytes!("../../../fixtures/appintents/DataApp.swiftconstvalues"),
        )
        .unwrap();
        let constants = dir.join("constants.list");
        std::fs::write(&constants, values.display().to_string()).unwrap();
        let mut args = Args {
            output: dir.join("App.app"),
            constants: vec![constants],
            ..Args::default()
        };
        run(&args).unwrap();
        let registration = args.output.join("Metadata.appintents/extract.actionsdata");
        let expected = std::fs::read(&registration).unwrap();
        args.dynamic_dependencies.push(dependencies);
        run(&args).unwrap();
        assert_eq!(std::fs::read(&registration).unwrap(), expected);
        std::fs::write(dependency, br#"{"version":99,"actions":{}}"#).unwrap();
        assert!(run(&args).unwrap_err().contains("unsupported dynamic"));
    }
    #[test]
    fn removing_intents_removes_the_previous_registration_and_language_model() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-appintents-incremental");
        let constants = dir.join("values.swiftconstvalues");
        let list = dir.join("values.list");
        std::fs::write(
            &constants,
            include_bytes!("../../../fixtures/appintents/DataApp.swiftconstvalues"),
        )
        .unwrap();
        std::fs::write(&list, format!("{}\n", constants.display())).unwrap();
        let args = Args {
            output: dir.join("DataApp.app"),
            module: "DataApp".into(),
            bundle: "dev.test".into(),
            constants: vec![list],
            ..Args::default()
        };
        run(&args).unwrap();
        std::fs::write(
            args.output.join("Info.plist"),
            crate::cli::xml_plist::document(
                "<dict><key>CFBundleName</key><string>DataApp</string></dict>",
            ),
        )
        .unwrap();
        training::run(&args.output, &args.output.join("Info.plist"), &args.bundle).unwrap();
        assert!(
            args.output
                .join("Metadata.appintents/nlu/nlu.lzfse")
                .is_file()
        );
        std::fs::write(constants, b"[]").unwrap();
        run(&args).unwrap();
        assert!(!args.output.join("Metadata.appintents").exists());
    }
}
