//! Swift Build trains localized shortcut phrases after metadata extraction.

use super::nlu::{self, Expansions, LanguageModel};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Default)]
struct Args {
    product: PathBuf,
    plist: PathBuf,
    bundle: String,
    temporary: Option<PathBuf>,
    extracted: Option<PathBuf>,
    archive: bool,
}

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
    let mut out = Args::default();
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--archive-ssu-assets" => out.archive = true,
            "--deployment-postprocessing" => {}
            "--product-path" => out.product = super::value(flag, &mut args)?.into(),
            "--infoplist-path" => out.plist = super::value(flag, &mut args)?.into(),
            "--bundle-id" => out.bundle.clone_from(super::value(flag, &mut args)?),
            "--temp-dir-path" => out.temporary = Some(super::value(flag, &mut args)?.into()),
            "--extracted-metadata-path" => {
                out.extracted = Some(super::value(flag, &mut args)?.into());
            }
            "--metadata-file-list" => {
                super::value(flag, &mut args)?;
            }
            _ => {
                return Err(format!(
                    "unsupported language-model processor option '{flag}'"
                ));
            }
        }
    }
    if out.product.as_os_str().is_empty() || out.plist.as_os_str().is_empty() {
        return Err("expected --product-path and --infoplist-path".into());
    }
    let resources = super::super::swbuild::darwin_sdk_bundle().join("appintents-phrases.json");
    train(&out, &resources)
}

#[cfg(test)]
pub(super) fn run(product: &Path, plist: &Path, bundle: &str) -> Result<(), String> {
    train(
        &Args {
            product: product.into(),
            plist: plist.into(),
            bundle: bundle.into(),
            archive: true,
            ..Args::default()
        },
        &product.join("unused-sdk-phrases.json"),
    )
}

#[derive(Deserialize)]
struct Resources {
    version: u32,
    locales: BTreeMap<String, Expansions>,
}

fn train(args: &Args, resources: &Path) -> Result<(), String> {
    let output = args
        .extracted
        .clone()
        .unwrap_or_else(|| args.product.join("Metadata.appintents"));
    let temporary = args.temporary.clone().unwrap_or_else(|| output.clone());
    let registration = output.join("extract.actionsdata");
    let metadata: Value = if registration.is_file() {
        serde_json::from_slice(&std::fs::read(&registration).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
    } else {
        Value::Null
    };
    let values = strings(&args.plist)?;
    let value = |name: &str| values.get(name).map(String::as_str);
    let bundle = if args.bundle.is_empty() {
        value("CFBundleIdentifier").unwrap_or("")
    } else {
        &args.bundle
    };
    let mut locales = Vec::new();
    for entry in std::fs::read_dir(&args.product).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() && path.extension().is_some_and(|e| e == "lproj") {
            let name = path
                .file_stem()
                .and_then(|n| n.to_str())
                .ok_or("invalid localization name")?;
            if name != "Base" {
                locales.push(name.to_string());
            }
        }
    }
    locales.sort();
    let localized = !locales.is_empty();
    if !localized {
        locales.push("en".into());
    }
    let mut languages = Vec::new();
    if metadata["autoShortcuts"]
        .as_array()
        .is_some_and(|s| !s.is_empty())
    {
        let app = value("CFBundleDisplayName")
            .or_else(|| value("CFBundleName"))
            .ok_or("App Shortcuts require CFBundleDisplayName or CFBundleName in Info.plist")?;
        if bundle.is_empty() {
            return Err("App Shortcuts require a bundle identifier".into());
        }
        let resources: Option<Resources> = if resources.is_file() {
            let parsed: Resources =
                serde_json::from_slice(&std::fs::read(resources).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if parsed.version != 1 {
                return Err("unsupported App Shortcuts resource export version".into());
            }
            Some(parsed)
        } else {
            None
        };
        for locale in &locales {
            let folder = args.product.join(format!("{locale}.lproj"));
            let info = strings(&folder.join("InfoPlist.strings"))?;
            let app = info
                .get("CFBundleDisplayName")
                .or_else(|| info.get("CFBundleName"))
                .map_or(app, String::as_str);
            let translations = strings(&folder.join("AppShortcuts.strings"))?;
            let mut translated = metadata.clone();
            localize(&mut translated, &translations)?;
            let expansions = if let Some(resources) = &resources {
                resources.locales.get(locale).cloned().ok_or_else(|| format!("no App Shortcuts expansions exported for locale '{locale}'; rerun ci/linux-appintents-resources.py with this locale"))?
            } else if locale == "en" {
                Expansions::english()?
            } else {
                return Err("localized App Shortcuts require appintents-phrases.json in the Darwin SDK; run ci/linux-appintents-resources.py on macOS and copy its export into the SDK".into());
            };
            languages.push(LanguageModel::new(locale, app, &translated, &expansions)?);
        }
    }
    write_outputs(args, &output, &temporary, bundle, &languages, localized)
}

fn write_outputs(
    args: &Args,
    output: &Path,
    temporary: &Path,
    bundle: &str,
    languages: &[LanguageModel],
    localized: bool,
) -> Result<(), String> {
    // Remove only this processor's outputs, including locales removed from the
    // project and the previous module-level layout. Do this after validation.
    for entry in std::fs::read_dir(&args.product).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|e| e == "lproj") {
            let model = path.join("nlu.appintents");
            if model.is_dir()
                && !languages
                    .iter()
                    .any(|l| path.file_stem().is_some_and(|s| s == l.locale.as_str()))
            {
                std::fs::remove_dir_all(model).map_err(|e| e.to_string())?;
            }
        }
    }
    let module = output.join("nlu");
    if module.is_dir() && (localized || languages.is_empty()) {
        std::fs::remove_dir_all(&module).map_err(|e| e.to_string())?;
    }
    for language in languages {
        let directory = if localized {
            args.product
                .join(format!("{}.lproj/nlu.appintents", language.locale))
        } else {
            module.clone()
        };
        nlu::write(&directory, bundle, language)?;
    }
    let yaml = nlu::ssu(bundle, languages)?;
    nlu::write_if_changed(&temporary.join("root.ssu.yaml"), yaml.as_bytes())?;
    let archived = output.join("root.ssu.yaml");
    if args.archive && !languages.is_empty() {
        nlu::write_if_changed(&archived, yaml.as_bytes())?;
    } else if archived.is_file() && archived != temporary.join("root.ssu.yaml") {
        std::fs::remove_file(archived).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn localize(value: &mut Value, strings: &BTreeMap<String, String>) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(key)) = map.get_mut("key")
                && let Some(translation) = strings.get(key)
            {
                if key.contains("${applicationName}") && !translation.contains("${applicationName}")
                {
                    return Err(format!(
                        "localized shortcut phrase '{key}' loses ${{applicationName}}"
                    ));
                }
                key.clone_from(translation);
            }
            for member in map.values_mut() {
                localize(member, strings)?;
            }
        }
        Value::Array(items) => {
            for member in items {
                localize(member, strings)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn strings(path: &Path) -> Result<BTreeMap<String, String>, String> {
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if data.starts_with(b"bplist00") || data.starts_with(b"<?xml") {
        return Ok(sweetpad_lib::derived_data::plist_strings(path)
            .into_iter()
            .collect());
    }
    let text = if data.starts_with(&[0xff, 0xfe]) || data.starts_with(&[0xfe, 0xff]) {
        if !data.len().is_multiple_of(2) {
            return Err(format!(
                "{}: incomplete UTF-16 string table",
                path.display()
            ));
        }
        let little = data[0] == 0xff;
        let units = data[2..]
            .chunks_exact(2)
            .map(|b| {
                if little {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect::<Vec<_>>();
        String::from_utf16(&units).map_err(|e| e.to_string())?
    } else {
        String::from_utf8(data).map_err(|e| e.to_string())?
    };
    let value = sweetpad_lib::pbxproj::parse(&format!("{{{text}}}"))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let dict = value.as_dict().ok_or("strings table is not a dictionary")?;
    dict.iter()
        .map(|(k, v)| {
            v.as_str()
                .map(|v| (k.clone(), v.to_string()))
                .ok_or_else(|| format!("{}: non-string value for '{k}'", path.display()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;
    use serde_json::json;

    #[test]
    fn utf16_shortcut_translations_preserve_the_application_placeholder() {
        let dir = TempDir::new("shortcut-strings");
        let table = dir.join("AppShortcuts.strings");
        let text = "\"Greet in ${applicationName}\" = \"Saluer dans ${applicationName}\";";
        let mut data = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            data.extend(unit.to_le_bytes());
        }
        std::fs::write(&table, data).unwrap();
        let mut metadata = json!({"phraseTemplates": [{"key": "Greet in ${applicationName}"}]});
        localize(&mut metadata, &strings(&table).unwrap()).unwrap();
        assert_eq!(
            metadata["phraseTemplates"][0]["key"],
            "Saluer dans ${applicationName}"
        );
        let mut invalid = json!({"key": "Greet in ${applicationName}"});
        let translations =
            BTreeMap::from([("Greet in ${applicationName}".into(), "Saluer".into())]);
        assert!(
            localize(&mut invalid, &translations)
                .unwrap_err()
                .contains("loses ${applicationName}")
        );
    }

    #[test]
    fn localized_training_writes_its_declared_output_and_removes_obsolete_models() {
        let dir = TempDir::new("localized-shortcuts");
        let product = dir.join("App.app");
        let metadata = product.join("Metadata.appintents");
        std::fs::create_dir_all(&metadata).unwrap();
        let registration = metadata.join("extract.actionsdata");
        std::fs::write(
            &registration,
            include_bytes!("../../../fixtures/appintents/extract.actionsdata"),
        )
        .unwrap();
        let plist = product.join("Info.plist");
        std::fs::write(
            &plist,
            crate::cli::xml_plist::document(
                "<dict><key>CFBundleName</key><string>Probe</string></dict>",
            ),
        )
        .unwrap();
        let folder = product.join("fr.lproj");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(
            folder.join("AppShortcuts.strings"),
            b"\"Greet in ${applicationName}\" = \"Saluer dans ${applicationName}\";",
        )
        .unwrap();
        let resources = dir.join("phrases.json");
        std::fs::write(
            &resources,
            serde_json::to_vec(
                &json!({"version": 1, "locales": {"fr": {"prefixes": [""], "negatives": []}}}),
            )
            .unwrap(),
        )
        .unwrap();
        let args = Args {
            product,
            plist,
            bundle: "dev.probe".into(),
            temporary: Some(dir.join("temp/ssu")),
            ..Args::default()
        };
        train(&args, &resources).unwrap();
        let model = folder.join("nlu.appintents/nlu.lzfse");
        let bytes = std::fs::read(&model).unwrap();
        assert!(
            bytes
                .windows(b"Saluer dans Probe".len())
                .any(|s| s == b"Saluer dans Probe")
        );
        assert!(dir.join("temp/ssu/root.ssu.yaml").is_file());
        assert!(!metadata.join("root.ssu.yaml").exists());
        std::fs::write(registration, br#"{"autoShortcuts":[]}"#).unwrap();
        train(&args, &resources).unwrap();
        assert!(!model.exists());
        assert!(dir.join("temp/ssu/root.ssu.yaml").is_file());
    }
}
