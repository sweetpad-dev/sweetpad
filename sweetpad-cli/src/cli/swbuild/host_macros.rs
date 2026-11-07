//! Native SwiftPM builds for macro executables in generated Xcode projects.
//! Tuist turns a package's macro into a macOS tool; it must run on the build
//! host even though the project consuming it targets an Apple platform.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::cli::{CliError, process, swiftpm};

fn plugin_names(flags: &str) -> Vec<String> {
    sweetpad_lib::scheme::split_launch_argument(flags)
        .windows(2)
        .filter(|pair| pair[0] == "-load-plugin-executable")
        .filter_map(|pair| {
            Path::new(pair[1].split_once('#')?.0)
                .file_name()?
                .to_str()
                .map(str::to_string)
        })
        .collect()
}

/// Build package macro tools and point compiler flags at their native binaries.
/// The original targets remain available in the PIF, but consumers no longer
/// depend on building the macOS executables.
pub(super) fn prepare(
    pif: &mut [Value],
    scratch: &Path,
    configuration: &str,
    requested: &[String],
) -> Result<HashSet<String>, CliError> {
    let required = required_targets(pif, requested);
    let names: HashSet<String> = pif
        .iter()
        .filter(|o| {
            o["type"] == "target"
                && o["contents"]["guid"]
                    .as_str()
                    .is_some_and(|g| required.contains(g))
        })
        .flat_map(|o| {
            o["contents"]["buildConfigurations"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|c| {
                    plugin_names(
                        c["buildSettings"]["OTHER_SWIFT_FLAGS"]
                            .as_str()
                            .unwrap_or_default(),
                    )
                })
        })
        .collect();
    let mut tools = BTreeMap::new();
    for target in pif.iter().filter(|o| o["type"] == "target") {
        let c = &target["contents"];
        let Some(name) = c["name"].as_str().filter(|n| names.contains(*n)) else {
            continue;
        };
        if c["productTypeIdentifier"] != "com.apple.product-type.tool" {
            continue;
        }
        let project = pif
            .iter()
            .find(|p| {
                p["type"] == "project"
                    && p["contents"]["targets"]
                        .as_array()
                        .is_some_and(|ts| ts.contains(&target["signature"]))
            })
            .ok_or_else(|| CliError::new(format!("macro tool '{name}' has no project")))?;
        let project = &project["contents"];
        let settings = project["buildConfigurations"]
            .as_array()
            .and_then(|cs| cs.iter().find(|c| c["name"] == configuration));
        let source = settings
            .and_then(|c| c["buildSettings"]["SRCROOT"].as_str())
            .unwrap_or(".");
        let directory = Path::new(project["projectDirectory"].as_str().unwrap_or(".")).join(source);
        let directory = directory
            .canonicalize()
            .map_err(|e| CliError::new(format!("couldn't find sources for macro '{name}': {e}")))?;
        let root = directory
            .ancestors()
            .find(|p| p.join("Package.swift").is_file())
            .ok_or_else(|| {
                CliError::new(format!(
                    "macro '{name}' needs a host build, but its sources have no Package.swift: {}",
                    directory.display()
                ))
            })?;
        let build = scratch.join(project["guid"].as_str().unwrap_or(name));
        let binary = build_macro(root, &build, name, configuration)?;
        tools.insert(
            name.to_string(),
            (c["guid"].as_str().unwrap_or_default().to_string(), binary),
        );
    }
    rewrite_flags(pif, &tools);
    Ok(tools.into_values().map(|(guid, _)| guid).collect())
}

/// Workspace schemes can explicitly build every generated target, including
/// macro executables already built for the host. Do not build those for macOS.
pub(super) fn exclude_prebuilt(request: &mut Value, tools: &HashSet<String>) {
    if let Some(targets) = request["configuredTargets"].as_array_mut() {
        targets.retain(|t| !t["guid"].as_str().is_some_and(|g| tools.contains(g)));
    }
}

fn rewrite_flags(pif: &mut [Value], tools: &BTreeMap<String, (String, PathBuf)>) {
    for object in pif.iter_mut().filter(|o| o["type"] == "target") {
        let c = &mut object["contents"];
        if let Some(configs) = c["buildConfigurations"].as_array_mut() {
            for config in configs {
                let Some(flags) = config["buildSettings"]["OTHER_SWIFT_FLAGS"].as_str() else {
                    continue;
                };
                let mut words = sweetpad_lib::scheme::split_launch_argument(flags);
                let mut changed = false;
                for i in 1..words.len() {
                    if words[i - 1] != "-load-plugin-executable" {
                        continue;
                    }
                    let Some((path, modules)) = words[i].split_once('#') else {
                        continue;
                    };
                    let Some((_, binary)) = Path::new(path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .and_then(|n| tools.get(n))
                    else {
                        continue;
                    };
                    words[i] = format!("{}#{modules}", binary.display());
                    changed = true;
                }
                if changed {
                    config["buildSettings"]["OTHER_SWIFT_FLAGS"] =
                        json!(words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" "));
                }
            }
        }
        if let Some(dependencies) = c["dependencies"].as_array_mut() {
            dependencies.retain(|d| !tools.values().any(|(guid, _)| d["guid"] == *guid));
        }
    }
}

/// Follow explicit dependencies and linked products. An unrelated scheme's
/// macros must not prevent the requested targets from building.
fn required_targets(pif: &[Value], requested: &[String]) -> HashSet<String> {
    fn files(value: &Value, names: &mut BTreeMap<String, String>) {
        if let (Some(guid), Some(path)) = (value["guid"].as_str(), value["path"].as_str()) {
            names.insert(guid.to_string(), path.to_string());
        }
        for child in value["children"].as_array().into_iter().flatten() {
            files(child, names);
        }
    }
    let mut references = BTreeMap::new();
    for project in pif.iter().filter(|o| o["type"] == "project") {
        files(&project["contents"]["groupTree"], &mut references);
    }
    let targets: Vec<&Value> = pif
        .iter()
        .filter(|o| o["type"] == "target")
        .map(|o| &o["contents"])
        .collect();
    let mut pending: Vec<&Value> = targets
        .iter()
        .copied()
        .filter(|t| {
            requested
                .iter()
                .any(|n| t["name"] == *n || t["name"] == format!("{n}-product"))
        })
        .collect();
    let mut required = HashSet::new();
    while let Some(target) = pending.pop() {
        let Some(guid) = target["guid"].as_str() else {
            continue;
        };
        if !required.insert(guid.to_string()) {
            continue;
        }
        for dependency in target["dependencies"].as_array().into_iter().flatten() {
            pending.extend(
                targets
                    .iter()
                    .copied()
                    .filter(|t| t["guid"] == dependency["guid"]),
            );
        }
        for file in target["buildPhases"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|p| p["buildFiles"].as_array().into_iter().flatten())
        {
            pending.extend(targets.iter().copied().filter(|t| {
                file["targetReference"]
                    .as_str()
                    .is_some_and(|g| t["guid"] == g)
            }));
            if let Some(path) = file["fileReference"]
                .as_str()
                .and_then(|g| references.get(g))
            {
                let name = Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                pending.extend(
                    targets
                        .iter()
                        .copied()
                        .filter(|t| t["productReference"]["name"] == name),
                );
            }
        }
    }
    required
}

fn quote(word: &str) -> String {
    if word
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\\')
    {
        format!("\"{}\"", word.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        word.to_string()
    }
}

fn build_macro(
    root: &Path,
    scratch: &Path,
    name: &str,
    configuration: &str,
) -> Result<PathBuf, CliError> {
    let configuration = swiftpm::configuration_arg(configuration);
    let output = Command::new("swift")
        .args(["build", "--build-system", "native", "--package-path"])
        .arg(root)
        .arg("--scratch-path")
        .arg(scratch)
        .args(["--configuration", configuration, "--product", name])
        .env_remove("SDKROOT")
        .env_remove("DEVELOPER_DIR")
        .env_remove("SWIFTSDK")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| process::spawn_error("swift", &e))?;
    if !output.status.success() {
        return Err(CliError::new(format!(
            "couldn't build macro '{name}' for the host:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    [format!("{name}-tool"), name.to_string()]
        .into_iter()
        .map(|n| scratch.join(configuration).join(n))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            CliError::new(format!(
                "the native build of macro '{name}' produced no executable in {}",
                scratch.display()
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_roots_exclude_tools_already_built_for_the_host() {
        let mut request = json!({"configuredTargets":[{"guid":"APP"},{"guid":"MACRO"}]});
        exclude_prebuilt(&mut request, &HashSet::from(["MACRO".to_string()]));
        assert_eq!(request["configuredTargets"], json!([{"guid":"APP"}]));
    }

    #[test]
    fn quoted_plugin_paths_preserve_spaces_and_multiple_modules() {
        let flags =
            r#"$(inherited) -load-plugin-executable "$(BUILD_DIR)/My Macro#Macros,Other" -DDEBUG"#;
        assert_eq!(plugin_names(flags), ["My Macro"]);
        let words = sweetpad_lib::scheme::split_launch_argument(flags);
        assert_eq!(
            sweetpad_lib::scheme::split_launch_argument(
                &words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
            ),
            words
        );
    }

    #[test]
    fn only_the_requested_targets_and_linked_products_require_macros() {
        let pif = vec![
            json!({"type":"project","contents":{"groupTree":{"children":[{"guid":"FILE","path":"Kit.framework"}]}}}),
            json!({"type":"target","contents":{"guid":"APP","name":"App","dependencies":[{"guid":"HELPER"}],"buildPhases":[{"buildFiles":[{"fileReference":"FILE"}]}]}}),
            json!({"type":"target","contents":{"guid":"HELPER","name":"Helper"}}),
            json!({"type":"target","contents":{"guid":"KIT","name":"Kit","productReference":{"name":"Kit.framework"}}}),
            json!({"type":"target","contents":{"guid":"UNUSED","name":"OtherApp"}}),
        ];
        assert_eq!(
            required_targets(&pif, &["App".to_string()]),
            HashSet::from(["APP".to_string(), "HELPER".to_string(), "KIT".to_string()])
        );
    }
}
