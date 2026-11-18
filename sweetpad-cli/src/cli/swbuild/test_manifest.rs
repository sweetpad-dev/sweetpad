//! Relocatable XCTest manifests for Swift Build's iOS test products.
//!
//! Selection comes from the scheme, never from every bundle left in Products.
//! Unsupported testing policies produce a precise note after a successful build.

use std::collections::{BTreeMap, HashMap};
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

/// SwiftPM's scheme name can select a PIF target named `<name>-product`.
/// Resolve it in its owning package, using the exact target Swift Build used.
struct TargetSettings<'a> {
    pif: &'a [Value],
    package_manifests: HashMap<String, PathBuf>,
    cache: BTreeMap<String, BTreeMap<String, String>>,
}

impl TargetSettings<'_> {
    fn get(
        &mut self,
        plan: &BuildPlan<'_>,
        name: &str,
    ) -> Result<&BTreeMap<String, String>, String> {
        if !self.cache.contains_key(name) {
            let package = pif_target(self.pif, name).and_then(|target| {
                let owner = self.package_manifests.get(target["guid"].as_str()?)?;
                Some((target["name"].as_str()?, owner.as_path()))
            });
            let (target_name, manifest) = package.map_or((name, None), |(n, p)| (n, Some(p)));
            let values = xcodebuild::test_target_settings(plan, target_name, manifest)
                .map_err(|e| e.to_string())?;
            self.cache.insert(name.into(), values);
        }
        Ok(&self.cache[name])
    }

    fn product_name(
        &mut self,
        plan: &BuildPlan<'_>,
        name: &str,
        target: &Value,
    ) -> Result<String, String> {
        let reference = target["productReference"]["name"]
            .as_str()
            .ok_or("product has no filename")?;
        if reference.contains("$(") || reference.contains("${") {
            // A package test's reference names its executable. The manifest
            // points to the enclosing wrapper, including its extension.
            return self
                .get(plan, name)?
                .get("FULL_PRODUCT_NAME")
                .filter(|s| !s.is_empty())
                .cloned()
                .ok_or_else(|| format!("test product '{name}' has no resolved filename"));
        }
        Ok(reference.into())
    }
}

fn expand_selection(
    plan: &BuildPlan<'_>,
    selected: &Selection,
    name: &str,
    settings_cache: &mut TargetSettings<'_>,
) -> Result<Selection, String> {
    let mut expanded = selected.clone();
    let expansion_name = selected.expansion_target.as_deref().unwrap_or(name);
    let needs_expansion = selected
        .environment
        .values()
        .chain(selected.arguments.iter())
        .any(|v| v.contains('$'));
    if needs_expansion {
        let settings = settings_cache.get(plan, expansion_name)?;
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
    selected: &Selection,
    products: &Path,
    configuration: &str,
    simulator: bool,
    settings_cache: &mut TargetSettings<'_>,
) -> Result<Vec<Plist>, String> {
    let mut targets = Vec::new();
    for name in &selected.names {
        let pif_target = pif_target(settings_cache.pif, name)
            .ok_or_else(|| format!("test target '{name}' is absent from the built PIF"))?;
        let kind = pif_target["productTypeIdentifier"]
            .as_str()
            .unwrap_or_default();
        let ui = kind == "com.apple.product-type.bundle.ui-testing";
        if !ui && kind != "com.apple.product-type.bundle.unit-test" {
            return Err(format!("'{name}' is not a test product"));
        }
        let basename = settings_cache.product_name(plan, name, pif_target)?;
        let bundle = locate(products, configuration, &basename)?;
        let expanded = expand_selection(plan, selected, name, settings_cache)?;
        let ui_app = if ui {
            let app_name = settings_cache
                .get(plan, name)?
                .get("TEST_TARGET_NAME")
                .filter(|s| !s.is_empty())
                .cloned()
                .ok_or_else(|| format!("UI test target '{name}' has no TEST_TARGET_NAME"))?;
            let app_target = super::pif_target(settings_cache.pif, &app_name).ok_or_else(|| {
                format!("UI target application '{app_name}' is absent from the built PIF")
            })?;
            let basename = settings_cache.product_name(plan, &app_name, app_target)?;
            if Path::new(&basename).file_name().and_then(|s| s.to_str()) != Some(basename.as_str())
            {
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
    let mut settings_cache = TargetSettings {
        pif: &pif,
        package_manifests: sweetpad_lib::pif::register_test_packages(&invocation.container, &pif),
        cache: BTreeMap::new(),
    };
    for mut selected in selections {
        selected.names.sort();
        selected.names.dedup();
        let targets = selected_targets(
            plan,
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
    let container = sweetpad_lib::project::absolutize(&invocation.container);
    let source_root = container.parent().unwrap_or(Path::new("/"));
    let mut manifest = dict([
        (
            "__xctestrun_metadata__",
            dict([
                ("FormatVersion", Plist::Int(2)),
                (SOURCE_ROOT, text(source_root.display().to_string())),
                (PRODUCTS_ROOT, text(products.display().to_string())),
            ]),
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
    std::fs::write(&temporary, plist::to_xml(&manifest))
        .and_then(|()| std::fs::rename(&temporary, &path))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// The metadata keys naming the folder that held the project and the
/// products folder on the machine that built them. Values from the scheme or
/// test plan, like `$(SRCROOT)/Fixtures`, are expanded to that machine's
/// paths, which [`localize`] moves to the machine that runs the tests.
const SOURCE_ROOT: &str = "SweetPadSourceRoot";
const PRODUCTS_ROOT: &str = "SweetPadProductsRoot";

/// The keys of a test target whose values are expanded from build settings.
const EXPANDED: [&str; 4] = [
    "EnvironmentVariables",
    "CommandLineArguments",
    "UITargetAppEnvironmentVariables",
    "UITargetAppCommandLineArguments",
];

/// A copy of a manifest with this machine's paths, beside the original so
/// `__TESTROOT__` still names the products folder, removed when dropped.
pub(crate) struct LocalManifest {
    pub(crate) path: PathBuf,
    /// How many values had a path of the building machine.
    pub(crate) moved: usize,
}

impl Drop for LocalManifest {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The manifest at `manifest`, with the paths the building machine
/// expanded into environment variables and arguments moved here: its
/// source root to `source_root` and its products folder to the manifest's
/// folder. `None` when the manifest names no other machine's paths, or
/// wasn't written by a sweetpad that records them.
pub(crate) fn localize(
    manifest: &Path,
    source_root: &Path,
) -> Result<Option<LocalManifest>, String> {
    let data = std::fs::read(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    if data.starts_with(b"bplist00") {
        return Ok(None);
    }
    let mut value = plist::parse(&data).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let metadata = plist::get(&value, "__xctestrun_metadata__");
    let root = |key: &str| {
        metadata
            .and_then(|m| plist::string(m, key))
            .map(str::to_string)
    };
    let products = manifest.parent().unwrap_or(Path::new("."));
    let mut roots: Vec<(String, String)> = [
        (root(SOURCE_ROOT), source_root),
        (root(PRODUCTS_ROOT), products),
    ]
    .into_iter()
    .filter_map(|(from, to)| Some((from?, to.display().to_string())))
    .filter(|(from, to)| from != to && !from.is_empty())
    .collect();
    if roots.is_empty() {
        return Ok(None);
    }
    // The products folder may sit inside the source root.
    roots.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
    let mut changed = 0;
    if let Some(Plist::Array(configurations)) = plist::get_mut(&mut value, "TestConfigurations") {
        for configuration in configurations {
            let Some(Plist::Array(targets)) = plist::get_mut(configuration, "TestTargets") else {
                continue;
            };
            for target in targets {
                for key in EXPANDED {
                    if let Some(field) = plist::get_mut(target, key) {
                        changed += move_paths(field, &roots);
                    }
                }
            }
        }
    }
    if changed == 0 {
        return Ok(None);
    }
    let name = manifest
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let path = manifest.with_file_name(format!(".{name}.{}.local.xctestrun", std::process::id()));
    std::fs::write(&path, plist::to_xml(&value)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(LocalManifest {
        path,
        moved: changed,
    }))
}

/// Move the paths in the strings of `value` from each root in `roots` to
/// its new place, returning how many strings changed.
fn move_paths(value: &mut Plist, roots: &[(String, String)]) -> usize {
    match value {
        Plist::String(text) => {
            let relocated = roots
                .iter()
                .fold(text.clone(), |text, (from, to)| move_root(&text, from, to));
            usize::from(std::mem::replace(text, relocated) != *text)
        }
        Plist::Array(items) => items.iter_mut().map(|item| move_paths(item, roots)).sum(),
        Plist::Dict(entries) => entries
            .iter_mut()
            .map(|(_, item)| move_paths(item, roots))
            .sum(),
        _ => 0,
    }
}

/// `text` with each `from` that is a whole path, or a path's leading
/// folders, replaced by `to`: `from` followed by the end, `/`, `:` or a
/// quote or space, so `/src/app` doesn't move `/src/application`.
fn move_root(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        let after = &rest[at + from.len()..];
        let whole = after
            .chars()
            .next()
            .is_none_or(|c| matches!(c, '/' | ':' | ' ' | '"' | '\'' | ','));
        out.push_str(&rest[..at]);
        out.push_str(if whole { to } else { from });
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    #[test]
    fn a_manifest_built_elsewhere_gets_this_machines_paths() {
        let dir = TempDir::new("manifest-localize");
        let manifest = dir.join("App-Debug-iphonesimulator.xctestrun");
        let target = dict([
            (
                "EnvironmentVariables",
                dict([
                    ("FIXTURES", text("/home/me/app/Fixtures")),
                    (
                        "OUTPUT",
                        text("/home/me/dd/Build/Products/Debug-iphonesimulator"),
                    ),
                    ("OTHER", text("/home/me/application")),
                ]),
            ),
            ("CommandLineArguments", array([text("--root=/home/me/app")])),
            (
                "TestHostPath",
                text("__TESTROOT__/Debug-iphonesimulator/App.app"),
            ),
            ("IsAppHostedTestBundle", Plist::Bool(true)),
        ]);
        let value = dict([
            (
                "__xctestrun_metadata__",
                dict([
                    ("FormatVersion", Plist::Int(2)),
                    (SOURCE_ROOT, text("/home/me/app")),
                    (PRODUCTS_ROOT, text("/home/me/dd/Build/Products")),
                ]),
            ),
            (
                "TestConfigurations",
                array([dict([("TestTargets", array([target]))])]),
            ),
        ]);
        std::fs::write(&manifest, plist::to_xml(&value)).unwrap();
        let local = localize(&manifest, Path::new("/Users/me/app"))
            .unwrap()
            .unwrap();
        assert_eq!(local.moved, 3);
        let moved = plist::parse(&std::fs::read(&local.path).unwrap()).unwrap();
        let target = &plist::get(&moved, "TestConfigurations")
            .and_then(|c| {
                if let Plist::Array(c) = c {
                    c.first()
                } else {
                    None
                }
            })
            .and_then(|c| plist::get(c, "TestTargets"))
            .and_then(|t| {
                if let Plist::Array(t) = t {
                    t.first()
                } else {
                    None
                }
            })
            .unwrap()
            .clone();
        let environment = plist::get(target, "EnvironmentVariables").unwrap();
        assert_eq!(
            plist::string(environment, "FIXTURES"),
            Some("/Users/me/app/Fixtures")
        );
        assert_eq!(
            plist::string(environment, "OUTPUT").map(PathBuf::from),
            Some(dir.join("Debug-iphonesimulator"))
        );
        assert_eq!(
            plist::string(environment, "OTHER"),
            Some("/home/me/application")
        );
        assert_eq!(
            plist::get(target, "IsAppHostedTestBundle"),
            Some(&Plist::Bool(true))
        );
        let path = local.path.clone();
        drop(local);
        assert!(!path.exists());
        assert!(
            localize(&manifest, Path::new("/home/me/app"))
                .unwrap()
                .is_some_and(|l| l.moved == 1)
        );
    }

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
    fn package_test_manifest_locates_the_wrapper_even_when_the_executable_has_another_name() {
        use crate::cli::resolve::Container;
        use serde_json::json;

        let root = TempDir::new("package-test-wrapper");
        let container = Container::Project(root.join("App.xcodeproj"));
        let plan = BuildPlan {
            action: xcodebuild::BuildAction::BuildForTesting,
            container: &container,
            scheme: "AccountTests",
            configuration: "Debug",
            destination: Some("generic/platform=iOS Simulator"),
            sdk: None,
            clean: false,
            hot: false,
            hot_entitlements: None,
            result_bundle: None,
            passthrough: &[],
        };
        let package = root.join("Account");
        let pif = [
            json!({"type":"project","contents":{"guid":"PACKAGE:account","projectName":"Account",
                "projectDirectory": package, "targets":["test-signature"]}}),
            json!({"type":"target","signature":"test-signature","contents":{
            "guid":"PACKAGE-PRODUCT:account_AccountTests.AccountTests","name":"AccountTests-product",
            "productTypeIdentifier":"com.apple.product-type.bundle.unit-test",
            "productReference":{"name":"$(EXECUTABLE_NAME)"},
            "buildConfigurations":[{"name":"Debug","buildSettings":{
                "TARGET_NAME":"AccountTests","PRODUCT_NAME":"$(TARGET_NAME)Wrapper","EXECUTABLE_NAME":"TestBinary"
            }}]}}),
        ];
        let mut settings = TargetSettings {
            pif: &pif,
            package_manifests: sweetpad_lib::pif::register_test_packages(container.path(), &pif),
            cache: BTreeMap::new(),
        };
        let configuration = "Debug-iphonesimulator";
        let bundle = root.join(configuration).join("AccountTestsWrapper.xctest");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("TestBinary"), []).unwrap();
        let info = dict([("CFBundleExecutable", text("TestBinary"))]);
        std::fs::write(bundle.join("Info.plist"), plist::to_xml(&info)).unwrap();
        let selected = Selection {
            names: vec!["AccountTests".into()],
            ..Selection::default()
        };
        let targets =
            selected_targets(&plan, &selected, &root, configuration, true, &mut settings).unwrap();
        assert_eq!(
            plist::string(&targets[0], "TestBundlePath"),
            Some("__TESTROOT__/Debug-iphonesimulator/AccountTestsWrapper.xctest")
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
