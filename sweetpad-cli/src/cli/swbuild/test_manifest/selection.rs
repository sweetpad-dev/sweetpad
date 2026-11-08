//! Scheme/test-plan options, kept separate from product discovery.
use super::{Selection, child};
use serde_json::Value;
use std::path::{Path, PathBuf};
use sweetpad_lib::xcscheme::Element;

pub(super) fn scheme(scheme: &Element) -> Result<Selection, String> {
    let test = child(scheme, "TestAction").ok_or("scheme has no TestAction")?;
    let mut result = Selection {
        language: test.attr("language").map(str::to_string),
        region: test.attr("region").map(str::to_string),
        expansion_target: child(test, "MacroExpansion")
            .and_then(|m| child(m, "BuildableReference"))
            .and_then(|b| b.attr("BlueprintName"))
            .map(str::to_string),
        ..Selection::default()
    };
    if let Some(testables) = child(test, "Testables") {
        for testable in testables
            .children
            .iter()
            .filter(|t| t.name == "TestableReference" && t.attr("skipped") != Some("YES"))
        {
            let buildable = child(testable, "BuildableReference")
                .ok_or("testable has no BuildableReference")?;
            let name = buildable
                .attr("BlueprintName")
                .ok_or("testable has no BlueprintName")?
                .to_string();
            for (tag, identifiers) in [
                ("SkippedTests", &mut result.skipped),
                ("SelectedTests", &mut result.only),
            ] {
                if let Some(tests) = child(testable, tag) {
                    identifiers.insert(
                        name.clone(),
                        tests
                            .children
                            .iter()
                            .filter_map(|e| e.attr("Identifier").map(str::to_string))
                            .collect(),
                    );
                }
            }
            result.names.push(name);
        }
    }
    let launch = if test.attr("shouldUseLaunchSchemeArgsEnv") == Some("YES") {
        child(scheme, "LaunchAction")
            .ok_or("test action uses missing launch arguments/environment")?
    } else {
        test
    };
    if let Some(environment) = child(launch, "EnvironmentVariables") {
        for entry in environment
            .children
            .iter()
            .filter(|e| e.attr("isEnabled") != Some("NO"))
        {
            result.environment.insert(
                entry
                    .attr("key")
                    .ok_or("test environment variable has no key")?
                    .into(),
                entry.attr("value").unwrap_or_default().into(),
            );
        }
    }
    if let Some(arguments) = child(launch, "CommandLineArguments") {
        // XCTest splits each expanded row. Preserve quotes here.
        result.arguments = arguments
            .children
            .iter()
            .filter(|e| e.attr("isEnabled") != Some("NO"))
            .map(|e| e.attr("argument").unwrap_or_default().to_string())
            .collect();
    }
    Ok(result)
}

pub(super) fn container_base(file: &Path) -> &Path {
    file.ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == ".swiftpm"))
        .and_then(Path::parent)
        .or_else(|| {
            file.ancestors()
                .find(|p| {
                    p.extension()
                        .is_some_and(|e| e == "xcodeproj" || e == "xcworkspace")
                })
                .and_then(Path::parent)
        })
        .unwrap_or(Path::new("."))
}

pub(super) fn configurations(
    scheme_xml: &Element,
    file: &Path,
    requested: Option<&str>,
) -> Result<Vec<Selection>, String> {
    let test = child(scheme_xml, "TestAction").ok_or("scheme has no TestAction")?;
    let plans: Vec<_> = child(test, "TestPlans")
        .into_iter()
        .flat_map(|p| &p.children)
        .map(|reference| {
            let path = reference
                .attr("reference")
                .ok_or("test plan reference has no path")?;
            Ok(sweetpad_lib::scheme::TestPlanRef {
                path: path.strip_prefix("container:").unwrap_or(path).to_string(),
                default: reference.attr("default") == Some("YES"),
            })
        })
        .collect::<Result<_, String>>()?;
    let Some(reference) = sweetpad_lib::scheme::select_test_plan(&plans, requested)? else {
        return Ok(vec![scheme(scheme_xml)?]);
    };
    let path: PathBuf = container_base(file).join(&reference.path);
    let value: Value = serde_json::from_slice(
        &std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    plan(
        &value,
        path.file_stem().and_then(|s| s.to_str()).unwrap_or("Tests"),
        reference.default,
    )
}

fn strings(value: &Value, field: &str) -> Result<Vec<String>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("test-plan {field} must be an array"))?
        .iter()
        .map(|s| {
            s.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("test-plan {field} has a non-string identifier"))
        })
        .collect()
}

pub(super) fn plan(value: &Value, name: &str, default: bool) -> Result<Vec<Selection>, String> {
    if value["version"] != 1 {
        return Err("only version 1 .xctestplan files are supported".into());
    }
    let mut base = Selection {
        plan: Some((name.into(), default)),
        ..Selection::default()
    };
    for target in value["testTargets"]
        .as_array()
        .ok_or("test plan has no testTargets array")?
    {
        if target["enabled"] == false {
            continue;
        }
        let name = target["target"]["name"]
            .as_str()
            .ok_or("test-plan target has no name")?
            .to_string();
        for key in target
            .as_object()
            .ok_or("test-plan target must be an object")?
            .keys()
        {
            if !matches!(
                key.as_str(),
                "target" | "enabled" | "selectedTests" | "skippedTests" | "parallelizable"
            ) {
                return Err(format!("unsupported test-plan target option '{key}'"));
            }
        }
        if let Some(only) = target.get("selectedTests") {
            base.only
                .insert(name.clone(), strings(only, "selectedTests")?);
        }
        if let Some(skip) = target.get("skippedTests") {
            base.skipped
                .insert(name.clone(), strings(skip, "skippedTests")?);
        }
        if let Some(parallel) = target.get("parallelizable") {
            base.target_options.entry(name.clone()).or_default().insert(
                "ParallelizationEnabled".into(),
                super::Plist::Bool(parallel.as_bool().ok_or("parallelizable must be boolean")?),
            );
        }
        base.names.push(name);
    }
    let defaults = value
        .get("defaultOptions")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    value["configurations"].as_array().ok_or("test plan has no configurations array")?.iter().map(|configuration| {
        let mut selected = base.clone();
        selected.configuration = Some(configuration["name"].as_str().ok_or("test-plan configuration has no name")?.into());
        // Native Xcode replaces arrays, including environmentVariableEntries.
        let mut options = defaults.clone();
        if let Some(overrides) = configuration.get("options") {
            options.extend(overrides.as_object().ok_or("test-plan options must be an object")?.clone());
        }
        for (key, value) in &options {
            match key.as_str() {
                "environmentVariableEntries" => for entry in value.as_array().ok_or("environmentVariableEntries must be an array")? {
                    if entry["enabled"] == false { continue; }
                    selected.environment.insert(entry["key"].as_str().ok_or("test-plan environment has no key")?.into(), entry["value"].as_str().unwrap_or_default().into());
                },
                "commandLineArgumentEntries" => for entry in value.as_array().ok_or("commandLineArgumentEntries must be an array")? {
                    if entry["enabled"] != false { selected.arguments.push(entry["argument"].as_str().ok_or("test-plan argument has no value")?.into()); }
                },
                "language" => selected.language = value.as_str().map(str::to_string),
                "region" => selected.region = value.as_str().map(str::to_string),
                "targetForVariableExpansion" => selected.expansion_target = value["name"].as_str().map(str::to_string),
                "testTimeoutsEnabled" | "defaultTestExecutionTimeAllowance" | "maximumTestExecutionTimeAllowance" | "systemAttachmentLifetime" | "userAttachmentLifetime" => {
                    let property = format!("{}{}", key[..1].to_ascii_uppercase(), &key[1..]);
                    selected.options.insert(property, match value {
                        Value::Bool(v) => super::Plist::Bool(*v),
                        Value::Number(v) => super::Plist::Int(v.as_i64().ok_or("invalid test timeout")?),
                        Value::String(v) => super::text(v),
                        _ => return Err(format!("unsupported value for test-plan '{key}'")),
                    });
                }
                _ => return Err(format!("unsupported test-plan option '{key}'; supply an .xctestrun with the required testing policy")),
            }
        }
        Ok(selected)
    }).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    #[test]
    fn configurations_replace_arrays_and_keep_filters_and_disabled_rows() {
        let plan = json!({"version":1,"defaultOptions":{"environmentVariableEntries":[{"key":"DEFAULT","value":"old"}],"commandLineArgumentEntries":[{"argument":"--value \"two words\""},{"argument":"disabled","enabled":false}]},"configurations":[{"name":"A","options":{"environmentVariableEntries":[{"key":"OVERRIDE","value":"$(PRODUCT_NAME)"}]}},{"name":"B","options":{}}],"testTargets":[{"target":{"name":"Tests"},"selectedTests":["Suite/test()"]},{"target":{"name":"Disabled"},"enabled":false}]});
        let selected = super::plan(&plan, "Smoke", true).unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].names, ["Tests"]);
        assert!(!selected[0].environment.contains_key("DEFAULT"));
        assert_eq!(selected[0].environment["OVERRIDE"], "$(PRODUCT_NAME)");
        assert_eq!(selected[1].environment["DEFAULT"], "old");
        assert_eq!(selected[0].arguments, [r#"--value "two words""#]);
        assert_eq!(selected[0].only["Tests"], ["Suite/test()"]);
    }
    #[test]
    fn unsupported_plan_policy_is_reported_not_dropped() {
        let value = json!({"version":1,"configurations":[{"name":"A","options":{"addressSanitizerEnabled":true}}],"testTargets":[]});
        assert!(
            super::plan(&value, "Plan", true)
                .err()
                .unwrap()
                .contains("addressSanitizerEnabled")
        );
    }
}
