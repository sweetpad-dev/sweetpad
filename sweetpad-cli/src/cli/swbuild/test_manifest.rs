//! Relocatable XCTest manifests for Swift Build's iOS test products.
//!
//! Selection comes from the scheme, never from every bundle left in Products.
//! Unsupported testing policies produce a precise note after a successful build.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sweetpad_lib::xcscheme::Element;

use super::{Invocation, pif_target};
use crate::cli::property_list::{self as plist, Plist};
use crate::cli::xcodebuild::{self, BuildPlan};

mod selection;

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
fn executable(bundle: &Path) -> Result<String, String> {
    let info = read_info(bundle)?;
    let name = plist::string(&info, "CFBundleExecutable").ok_or("bundle has no executable name")?;
    if !bundle.join(name).is_file() {
        return Err(format!("{} has no built executable", bundle.display()));
    }
    Ok(name.into())
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

#[derive(Clone, Default)]
struct Selection {
    configuration: Option<String>,
    plan: Option<(String, bool)>,
    language: Option<String>,
    region: Option<String>,
    expansion_target: Option<String>,
    options: BTreeMap<String, Plist>,
    target_options: BTreeMap<String, BTreeMap<String, Plist>>,
    names: Vec<String>,
    environment: BTreeMap<String, String>,
    arguments: Vec<String>,
    skipped: BTreeMap<String, Vec<String>>,
    only: BTreeMap<String, Vec<String>>,
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

fn ui_options(
    products: &Path,
    app: &Path,
    base: &str,
    scheme: &str,
    simulator: bool,
    selected: &Selection,
) -> Result<Plist, String> {
    let mut result = dict(std::iter::empty::<(&str, Plist)>());
    executable(app)?;
    plist::set(&mut result, "IsUITestBundle", Plist::Bool(true));
    plist::set(
        &mut result,
        "IsXCTRunnerHostedTestBundle",
        Plist::Bool(true),
    );
    plist::set(
        &mut result,
        "UITargetAppPath",
        text(test_path(products, app)?),
    );
    plist::set(
        &mut result,
        "UITargetAppCommandLineArguments",
        array(
            sweetpad_lib::scheme::locale_arguments(
                selected.language.as_deref(),
                selected.region.as_deref(),
                sweetpad_lib::scheme::host_language,
            )
            .iter()
            .map(text),
        ),
    );
    let mut app_env = testing_environment(base, scheme, simulator);
    for key in ["DYLD_FRAMEWORK_PATH", "DYLD_LIBRARY_PATH"] {
        if simulator {
            app_env.insert(key.into(), base.into());
        }
    }
    app_env.remove("DYLD_INSERT_LIBRARIES");
    if let Some((plan, _)) = &selected.plan {
        app_env.insert("XCODE_TEST_PLAN_NAME".into(), plan.clone());
    }
    plist::set(
        &mut result,
        "UITargetAppEnvironmentVariables",
        dict(app_env.into_iter().map(|(k, v)| (k, text(v)))),
    );
    Ok(result)
}

fn test_options(
    name: &str,
    executable: &str,
    bundle_path: &str,
    developer: &str,
    selected: &Selection,
) -> Plist {
    dict([
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
    ])
}

fn target(
    products: &Path,
    bundle: &Path,
    name: &str,
    scheme: &str,
    simulator: bool,
    selected: &Selection,
    ui_app: Option<&Path>,
) -> Result<Plist, String> {
    let executable = executable(bundle)?;
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
    let mut result = test_options(name, &executable, &bundle_path, developer, selected);
    if let Some(host) = host {
        let host_info = read_info(host)?;
        let identifier = plist::string(&host_info, "CFBundleIdentifier")
            .ok_or("test host has no bundle identifier")?;
        dependent.insert(0, text(test_path(products, host)?));
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
        if let Some(app) = ui_app {
            dependent.insert(0, text(test_path(products, app)?));
            if let Plist::Dict(fields) =
                ui_options(products, app, &base, scheme, simulator, selected)?
            {
                for (key, value) in fields {
                    plist::set(&mut result, &key, value);
                }
            }
            testing.remove("DYLD_INSERT_LIBRARIES");
        } else {
            plist::set(&mut result, "IsAppHostedTestBundle", Plist::Bool(true));
            testing.insert(
                "DYLD_INSERT_LIBRARIES".into(),
                "__TESTHOST__/Frameworks/libXCTestBundleInject.dylib:/usr/lib/libRPAC.dylib".into(),
            );
            testing.insert("XCInjectBundleInto".into(), "unused".into());
        }
    } else if ui_app.is_some() {
        return Err("UI test bundle has no XCTRunner app host".into());
    }
    if let Some(language) = &selected.language {
        plist::set(&mut result, "TestLanguage", text(language));
    }
    if let Some(region) = &selected.region {
        plist::set(&mut result, "TestRegion", text(region));
    }
    for (key, value) in selected
        .options
        .iter()
        .chain(selected.target_options.get(name).into_iter().flatten())
    {
        plist::set(&mut result, key, value.clone());
    }
    if let Some((plan, _)) = &selected.plan {
        testing.insert("XCODE_TEST_PLAN_NAME".into(), plan.clone());
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

fn expand_selection(
    plan: &BuildPlan<'_>,
    selected: &Selection,
    name: &str,
    ui: bool,
    settings_cache: &mut BTreeMap<String, BTreeMap<String, String>>,
) -> Result<Selection, String> {
    let mut expanded = selected.clone();
    let expansion_name = selected.expansion_target.as_deref().unwrap_or(name);
    let needs_expansion = selected
        .environment
        .values()
        .chain(selected.arguments.iter())
        .any(|v| v.contains('$'));
    for target in [
        ui.then_some(name),
        needs_expansion.then_some(expansion_name),
    ]
    .into_iter()
    .flatten()
    {
        if !settings_cache.contains_key(target) {
            settings_cache.insert(
                target.to_string(),
                xcodebuild::test_target_settings(plan, target).map_err(|e| e.to_string())?,
            );
        }
    }
    if needs_expansion {
        let settings = &settings_cache[expansion_name];
        expanded.environment = selected
            .environment
            .iter()
            .map(|(k, v)| (k.clone(), sweetpad_lib::resolver::expand_one(v, settings)))
            .collect();
        expanded.arguments = selected
            .arguments
            .iter()
            .map(|v| sweetpad_lib::resolver::expand_one(v, settings))
            .collect();
    }
    expanded
        .arguments
        .extend(sweetpad_lib::scheme::locale_arguments(
            selected.language.as_deref(),
            selected.region.as_deref(),
            sweetpad_lib::scheme::host_language,
        ));
    Ok(expanded)
}

fn selected_targets(
    plan: &BuildPlan<'_>,
    pif: &[Value],
    selected: &Selection,
    products: &Path,
    configuration: &str,
    simulator: bool,
    settings_cache: &mut BTreeMap<String, BTreeMap<String, String>>,
) -> Result<Vec<Plist>, String> {
    let mut targets = Vec::new();
    for name in &selected.names {
        let pif_target = pif_target(pif, name)
            .ok_or_else(|| format!("test target '{name}' is absent from the built PIF"))?;
        let kind = pif_target["productTypeIdentifier"]
            .as_str()
            .unwrap_or_default();
        let ui = kind == "com.apple.product-type.bundle.ui-testing";
        if !ui && kind != "com.apple.product-type.bundle.unit-test" {
            return Err(format!("'{name}' is not a test product"));
        }
        let basename = pif_target["productReference"]["name"]
            .as_str()
            .ok_or("test product has no filename")?;
        let bundle = locate(products, configuration, basename)?;
        let expanded = expand_selection(plan, selected, name, ui, settings_cache)?;
        let ui_app = if ui {
            let app_name = settings_cache[name]
                .get("TEST_TARGET_NAME")
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("UI test target '{name}' has no TEST_TARGET_NAME"))?;
            let app_target = super::pif_target(pif, app_name).ok_or_else(|| {
                format!("UI target application '{app_name}' is absent from the built PIF")
            })?;
            let basename = app_target["productReference"]["name"]
                .as_str()
                .ok_or("UI target app has no product filename")?;
            if Path::new(basename).file_name().and_then(|s| s.to_str()) != Some(basename) {
                return Err("invalid UI app product filename".into());
            }
            Some(products.join(configuration).join(basename))
        } else {
            None
        };
        targets.push(target(
            products,
            &bundle,
            name,
            plan.scheme,
            simulator,
            &expanded,
            ui_app.as_deref(),
        )?);
    }
    Ok(targets)
}

fn manifest_path(products: &Path, scheme: &str, configuration: &str) -> PathBuf {
    // The generated filename must remain a filename even for unusual scheme names.
    let safe_name: String = scheme
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    products.join(format!("{safe_name}-{configuration}.xctestrun"))
}

pub(super) fn write(plan: &BuildPlan<'_>, invocation: &Invocation) -> Result<PathBuf, String> {
    let sdk = invocation.parameters["activeRunDestination"]["platform"]
        .as_str()
        .unwrap_or_default();
    let simulator = sdk == "iphonesimulator";
    if !simulator && sdk != "iphoneos" {
        return Err("automatic .xctestrun generation currently supports iOS tests".into());
    }
    let configuration = format!("{}-{sdk}", plan.configuration);
    let pif: Vec<Value> =
        serde_json::from_slice(&std::fs::read(&invocation.pif).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let selections = if let Some(path) = sweetpad_lib::scheme::locate_with_derived_data(
        &invocation.container,
        plan.scheme,
        Some(&invocation.locations.folder),
    ) {
        let scheme = sweetpad_lib::xcscheme::parse(
            &std::fs::read_to_string(&path).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        selection::configurations(&scheme, &path, super::test_plan_name(plan.passthrough))?
    } else {
        vec![Selection {
            names: vec![plan.scheme.into()],
            ..Selection::default()
        }]
    };
    let plan_metadata = selections.first().and_then(|s| s.plan.clone());
    let products = &invocation.locations.products;
    let mut configurations = Vec::new();
    let mut settings_cache = BTreeMap::new();
    for mut selected in selections {
        selected.names.sort();
        selected.names.dedup();
        let targets = selected_targets(
            plan,
            &pif,
            &selected,
            products,
            &configuration,
            simulator,
            &mut settings_cache,
        )?;
        if targets.is_empty() {
            return Err("scheme has no enabled test targets".into());
        }
        configurations.push(dict([
            (
                "Name",
                text(
                    selected
                        .configuration
                        .as_deref()
                        .unwrap_or(plan.configuration),
                ),
            ),
            ("IsEnabled", Plist::Bool(true)),
            ("TestTargets", array(targets)),
        ]));
    }
    if configurations.is_empty() {
        return Err("test plan has no configurations".into());
    }
    let mut manifest = dict([
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
        ("TestConfigurations", array(configurations)),
    ]);
    if let Some((name, default)) = plan_metadata {
        plist::set(
            &mut manifest,
            "TestPlan",
            dict([("Name", text(name)), ("IsDefault", Plist::Bool(default))]),
        );
    }
    let path = manifest_path(products, plan.scheme, &configuration);
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
        let selected = selection::scheme(&scheme).unwrap();
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
