//! Relocatable XCTest manifests for Swift Build's iOS unit-test products.
//!
//! Selection comes from the scheme, never from every bundle left in Products.
//! Unsupported test plans/UI tests produce a precise note after a successful
//! build, rather than silently running a different set of tests.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sweetpad_lib::xcscheme::Element;

use super::{Invocation, pif_target};
use crate::cli::property_list::{self as plist, Plist};
use crate::cli::xcodebuild::BuildPlan;

fn dict(entries: impl IntoIterator<Item = (impl Into<String>, Plist)>) -> Plist {
    Plist::Dict(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn text(value: impl Into<String>) -> Plist {
    Plist::String(value.into())
}
fn array(values: impl IntoIterator<Item = Plist>) -> Plist {
    Plist::Array(values.into_iter().collect())
}
fn child<'a>(element: &'a Element, name: &str) -> Option<&'a Element> {
    element.children.iter().find(|c| c.name == name)
}
fn read_info(bundle: &Path) -> Result<Plist, String> {
    let path = bundle.join("Info.plist");
    plist::read_info(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)
}
fn test_path(products: &Path, path: &Path) -> Result<String, String> {
    path.strip_prefix(products)
        .map(|p| format!("__TESTROOT__/{}", p.to_string_lossy()))
        .map_err(|_| {
            format!(
                "{} lies outside test products {}",
                path.display(),
                products.display()
            )
        })
}

#[derive(Default)]
struct Selection {
    names: Vec<String>,
    environment: BTreeMap<String, String>,
    arguments: Vec<String>,
    skipped: BTreeMap<String, Vec<String>>,
    only: BTreeMap<String, Vec<String>>,
}

fn selection(scheme: &Element) -> Result<Selection, String> {
    let test = child(scheme, "TestAction").ok_or("scheme has no TestAction")?;
    if child(test, "TestPlans").is_some_and(|p| !p.children.is_empty()) {
        return Err("automatic .xctestrun generation does not yet support test plans; supply a manifest to 'test run --xctestrun'".into());
    }
    if ["language", "region"]
        .iter()
        .any(|key| test.attr(key).is_some_and(|v| !v.is_empty()))
    {
        return Err("test language or region overrides require a supplied .xctestrun".into());
    }
    let mut result = Selection::default();
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
            let key = entry
                .attr("key")
                .ok_or("test environment variable has no key")?;
            let value = entry.attr("value").unwrap_or_default();
            if value.contains("$(") || value.contains("${") {
                return Err(format!(
                    "test environment '{key}' needs build-setting expansion; supply an .xctestrun"
                ));
            }
            result.environment.insert(key.into(), value.into());
        }
    }
    if let Some(arguments) = child(launch, "CommandLineArguments") {
        for entry in arguments
            .children
            .iter()
            .filter(|e| e.attr("isEnabled") != Some("NO"))
        {
            let value = entry.attr("argument").unwrap_or_default();
            if value.contains("$(") || value.contains("${") {
                return Err(
                    "test arguments need build-setting expansion; supply an .xctestrun".into(),
                );
            }
            // XCTest parses each scheme row when launching the runner.
            // Splitting here would make it split a quoted argument twice.
            result.arguments.push(value.to_string());
        }
    }
    Ok(result)
}

/// Only the current configuration and selected product basename participate.
/// A duplicate is an error: guessing between stale or independently hosted
/// copies would turn a successful test build into a misleading test run.
fn locate(products: &Path, configuration: &str, basename: &str) -> Result<PathBuf, String> {
    if Path::new(basename).file_name().and_then(|n| n.to_str()) != Some(basename)
        || !basename.ends_with(".xctest")
    {
        return Err(format!("unsupported test product name '{basename}'"));
    }
    let root = products.join(configuration);
    let mut candidates = Vec::new();
    let direct = root.join(basename);
    if direct.is_dir() {
        candidates.push(direct);
    }
    for entry in std::fs::read_dir(&root).map_err(|e| format!("{}: {e}", root.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|e| e == "app") {
            let bundle = path.join("PlugIns").join(basename);
            if bundle.is_dir() {
                candidates.push(bundle);
            }
        }
    }
    match candidates.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(format!(
            "built test product '{basename}' is missing from {}",
            root.display()
        )),
        _ => Err(format!(
            "multiple built copies of '{basename}' in {}; use fresh DerivedData",
            root.display()
        )),
    }
}

fn testing_environment(base: &str, scheme: &str, simulator: bool) -> BTreeMap<String, String> {
    let developer = "__PLATFORMS__/iPhoneSimulator.platform/Developer";
    let mut testing = BTreeMap::from([("XCODE_SCHEME_NAME".to_string(), scheme.to_string())]);
    if simulator {
        testing.extend([
            (
                "DYLD_FRAMEWORK_PATH".into(),
                format!("{base}:{base}/PackageFrameworks:{developer}/Library/Frameworks"),
            ),
            (
                "DYLD_LIBRARY_PATH".into(),
                format!("{base}:{developer}/usr/lib"),
            ),
            (
                "DYLD_INSERT_LIBRARIES".into(),
                "/usr/lib/libRPAC.dylib".into(),
            ),
            ("__XCODE_BUILT_PRODUCTS_DIR_PATHS".into(), base.to_string()),
            ("__XPC_DYLD_FRAMEWORK_PATH".into(), base.to_string()),
            ("__XPC_DYLD_LIBRARY_PATH".into(), base.to_string()),
        ]);
    }
    testing
}

fn target(
    products: &Path,
    bundle: &Path,
    name: &str,
    scheme: &str,
    simulator: bool,
    selected: &Selection,
) -> Result<Plist, String> {
    let info = read_info(bundle)?;
    let executable =
        plist::string(&info, "CFBundleExecutable").ok_or("test bundle has no executable name")?;
    if !bundle.join(executable).is_file() {
        return Err(format!("{} has no built executable", bundle.display()));
    }
    let developer = "__PLATFORMS__/iPhoneSimulator.platform/Developer";
    let host = bundle
        .ancestors()
        .skip(1)
        .take_while(|p| p.starts_with(products))
        .find(|p| p.extension().is_some_and(|e| e == "app"));
    if !simulator && host.is_none() {
        return Err("device unit tests require an app host".into());
    }
    let configuration_root = host
        .unwrap_or(bundle)
        .parent()
        .ok_or("test product has no parent")?;
    let base = test_path(products, configuration_root)?;
    let mut testing = testing_environment(&base, scheme, simulator);
    let bundle_path = test_path(products, bundle)?;
    let mut dependent = vec![text(&bundle_path)];
    let mut result = dict([
        ("BlueprintName", text(name)),
        ("ProductModuleName", text(executable)),
        ("TestBundlePath", text(bundle_path)),
        (
            "TestHostPath",
            text(format!("{developer}/Library/Xcode/Agents/xctest")),
        ),
        (
            "EnvironmentVariables",
            dict(
                selected
                    .environment
                    .iter()
                    .map(|(k, v)| (k.clone(), text(v))),
            ),
        ),
        (
            "CommandLineArguments",
            array(selected.arguments.iter().map(text)),
        ),
        ("SystemAttachmentLifetime", text("deleteOnSuccess")),
        ("UserAttachmentLifetime", text("deleteOnSuccess")),
    ]);
    if let Some(host) = host {
        let host_info = read_info(host)?;
        let identifier = plist::string(&host_info, "CFBundleIdentifier")
            .ok_or("test host has no bundle identifier")?;
        dependent.insert(0, text(test_path(products, host)?));
        plist::set(&mut result, "IsAppHostedTestBundle", Plist::Bool(true));
        plist::set(
            &mut result,
            "TestHostPath",
            text(test_path(products, host)?),
        );
        plist::set(&mut result, "TestHostBundleIdentifier", text(identifier));
        plist::set(
            &mut result,
            "TestBundlePath",
            text(format!(
                "__TESTHOST__/{}",
                bundle.strip_prefix(host).unwrap().display()
            )),
        );
        testing.insert(
            "DYLD_INSERT_LIBRARIES".into(),
            "__TESTHOST__/Frameworks/libXCTestBundleInject.dylib:/usr/lib/libRPAC.dylib".into(),
        );
        testing.insert("XCInjectBundleInto".into(), "unused".into());
    }
    if let Some(skipped) = selected.skipped.get(name) {
        plist::set(
            &mut result,
            "SkipTestIdentifiers",
            array(skipped.iter().map(text)),
        );
    }
    if let Some(only) = selected.only.get(name) {
        plist::set(
            &mut result,
            "OnlyTestIdentifiers",
            array(only.iter().map(text)),
        );
    }
    plist::set(&mut result, "DependentProductPaths", array(dependent));
    plist::set(
        &mut result,
        "TestingEnvironmentVariables",
        dict(testing.into_iter().map(|(k, v)| (k, text(v)))),
    );
    Ok(result)
}

pub(super) fn write(plan: &BuildPlan<'_>, invocation: &Invocation) -> Result<PathBuf, String> {
    let sdk = invocation.parameters["activeRunDestination"]["platform"]
        .as_str()
        .unwrap_or_default();
    let simulator = sdk == "iphonesimulator";
    if !simulator && sdk != "iphoneos" {
        return Err("automatic .xctestrun generation currently supports iOS unit tests".into());
    }
    let configuration = format!("{}-{sdk}", plan.configuration);
    let pif: Vec<Value> =
        serde_json::from_slice(&std::fs::read(&invocation.pif).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut selected = if let Some(path) = sweetpad_lib::scheme::locate_with_derived_data(
        &invocation.container,
        plan.scheme,
        Some(&invocation.locations.folder),
    ) {
        selection(
            &sweetpad_lib::xcscheme::parse(
                &std::fs::read_to_string(path).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?,
        )?
    } else {
        Selection {
            names: vec![plan.scheme.into()],
            ..Selection::default()
        }
    };
    selected.names.sort();
    selected.names.dedup();
    let products = &invocation.locations.products;
    let mut targets = Vec::new();
    for name in &selected.names {
        let pif_target = pif_target(&pif, name)
            .ok_or_else(|| format!("test target '{name}' is absent from the built PIF"))?;
        let kind = pif_target["productTypeIdentifier"]
            .as_str()
            .unwrap_or_default();
        if kind == "com.apple.product-type.bundle.ui-testing" {
            return Err(
                "UI tests require a supplied .xctestrun specifying the target application".into(),
            );
        }
        if kind != "com.apple.product-type.bundle.unit-test" {
            return Err(format!("'{name}' is not a unit-test product"));
        }
        let basename = pif_target["productReference"]["name"]
            .as_str()
            .ok_or("test product has no filename")?;
        let bundle = locate(products, &configuration, basename)?;
        targets.push(target(
            products,
            &bundle,
            name,
            plan.scheme,
            simulator,
            &selected,
        )?);
    }
    if targets.is_empty() {
        return Err("scheme has no enabled unit-test targets".into());
    }
    let manifest = dict([
        (
            "__xctestrun_metadata__",
            dict([("FormatVersion", Plist::Int(2))]),
        ),
        (
            "ContainerInfo",
            dict([
                ("ContainerName", text(plan.scheme)),
                ("SchemeName", text(plan.scheme)),
            ]),
        ),
        (
            "TestConfigurations",
            array([dict([
                ("Name", text(plan.configuration)),
                ("IsEnabled", Plist::Bool(true)),
                ("TestTargets", array(targets)),
            ])]),
        ),
    ]);
    // The generated filename must remain a filename even for unusual scheme names.
    let safe_name: String = plan
        .scheme
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = products.join(format!("{safe_name}-{configuration}.xctestrun"));
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, crate::cli::keyed_archive::write(&manifest))
        .and_then(|()| std::fs::rename(&temporary, &path))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    #[test]
    fn selection_honors_skipped_targets_launch_environment_and_test_filters() {
        let scheme = sweetpad_lib::xcscheme::parse(r#"<Scheme><TestAction shouldUseLaunchSchemeArgsEnv="YES"><Testables><TestableReference skipped="YES"><BuildableReference BlueprintName="Other"/></TestableReference><TestableReference><BuildableReference BlueprintName="Tests"/><SkippedTests><Test Identifier="Tests/slow"/></SkippedTests><SelectedTests><Test Identifier="Tests/fast"/></SelectedTests></TestableReference></Testables></TestAction><LaunchAction><EnvironmentVariables><EnvironmentVariable key="ON" value="yes"/><EnvironmentVariable key="OFF" value="no" isEnabled="NO"/></EnvironmentVariables><CommandLineArguments><CommandLineArgument argument="--name &quot;two words&quot;"/></CommandLineArguments></LaunchAction></Scheme>"#).unwrap();
        let selected = selection(&scheme).unwrap();
        assert_eq!(selected.names, ["Tests"]);
        assert_eq!(
            selected.environment,
            BTreeMap::from([("ON".into(), "yes".into())])
        );
        assert_eq!(selected.arguments, [r#"--name "two words""#]);
        assert_eq!(selected.skipped["Tests"], ["Tests/slow"]);
        assert_eq!(selected.only["Tests"], ["Tests/fast"]);
    }

    #[test]
    fn unsupported_plans_cannot_silently_drop_options() {
        let scheme = sweetpad_lib::xcscheme::parse(
            "<Scheme><TestAction><TestPlans><TestPlanReference/></TestPlans></TestAction></Scheme>",
        )
        .unwrap();
        assert!(selection(&scheme).err().unwrap().contains("test plans"));
        let localized =
            sweetpad_lib::xcscheme::parse("<Scheme><TestAction language=\"fr\"/></Scheme>")
                .unwrap();
        assert!(
            selection(&localized)
                .err()
                .unwrap()
                .contains("language or region")
        );
    }

    #[test]
    fn selection_cannot_pick_a_stale_configuration_or_ambiguous_host() {
        let root = TempDir::new("manifest-selection");
        for path in [
            "Release-iphonesimulator/Tests.xctest",
            "Debug-iphonesimulator/Host.app/PlugIns/Tests.xctest",
        ] {
            std::fs::create_dir_all(root.join(path)).unwrap();
        }
        assert_eq!(
            locate(&root, "Debug-iphonesimulator", "Tests.xctest").unwrap(),
            root.join("Debug-iphonesimulator/Host.app/PlugIns/Tests.xctest")
        );
        std::fs::create_dir_all(root.join("Debug-iphonesimulator/Tests.xctest")).unwrap();
        assert!(
            locate(&root, "Debug-iphonesimulator", "Tests.xctest")
                .err()
                .unwrap()
                .contains("multiple")
        );
        assert!(locate(&root, "Debug-iphonesimulator", "../Tests.xctest").is_err());
    }
}
