//! The Darwin Swift SDK bundle that builds for Apple platforms use on Linux,
//! written from an unpacked Xcode.app: Xcode's Swift and Clang resources,
//! each platform's SDKs and test frameworks, and the metadata Swift Build and
//! SwiftPM read. Nothing in it runs on the host. The linker, `dsymutil` and
//! the macro plugin ship with sweetpad, and the Clang headers come from the
//! host's Swift, whose compiler reads them.
//!
//! - The toolchain's `usr/lib/swift`, `swift_static` and `clang`, without
//!   `prebuilt-modules`, its back-deployment runtimes from `usr/lib/swift-*`,
//!   and the Swift/C++ interoperability headers in `usr/include/swift`.
//! - For iOS, the iOS Simulator and macOS, the platform's manifest with the
//!   settings a build without Xcode needs (see [`patch_manifest`]), its
//!   SDKs, test frameworks, test runner and extension stubs, with XCTest and
//!   Swift Testing linked into each SDK's frameworks, as SwiftPM looks for
//!   them there.
//! - For tvOS, watchOS and visionOS and their simulators, when Xcode has
//!   them, the same parts as they are, with Swift Build's specifications.
//! - `swift-sdk.json`, with a triple for each architecture each SDK
//!   supports, `info.json`, which makes the folder an artifact bundle, and
//!   `toolset.json`, which `swift-sdk.json` names.
//! - Xcode's `version.plist`, each platform's, the macOS build the SDK came
//!   from (`build-machine.plist`), and the App Intents phrase resources
//!   exported from the Mac, when given.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::property_list::{self, Plist};

/// The platforms every bundle has, as Xcode names their folders.
const BASE_PLATFORMS: [&str; 3] = ["iPhoneOS", "MacOSX", "iPhoneSimulator"];
/// The platforms a bundle has when the Xcode it's written from does.
const EXTRA_PLATFORMS: [&str; 6] = [
    "AppleTVOS",
    "AppleTVSimulator",
    "WatchOS",
    "WatchSimulator",
    "XROS",
    "XRSimulator",
];
/// What a base platform keeps of its folder.
const BASE_PARTS: [&str; 7] = [
    "version.plist",
    "Developer/SDKs",
    "Developer/Library/Frameworks",
    "Developer/Library/PrivateFrameworks",
    "Developer/Library/Xcode/Agents/XCTRunner.app",
    "Developer/usr/lib",
    "Library/Application Support",
];
/// Xcode's macro plugins for the platform's SDK, Mach-O dylibs the host
/// can't load. `sweetpad-macros` expands their macros instead.
const HOST_PLUGINS: &str = "Developer/usr/lib/swift/host";
/// What another platform keeps of its folder.
const EXTRA_PARTS: [&str; 10] = [
    "Info.plist",
    "version.plist",
    "PlatformBasicInfo.plist",
    "Developer/SDKs",
    "Developer/Library/Xcode/Specifications",
    "Developer/Library/Xcode/Agents/XCTRunner.app",
    "Developer/Library/Frameworks",
    "Developer/Library/PrivateFrameworks",
    "Developer/usr/lib",
    "Library/Application Support",
];
/// The test frameworks linked into each base SDK's frameworks, from the
/// platform's `Developer/Library`.
const TEST_FRAMEWORKS: [(&str, &str); 4] = [
    ("Testing", "Frameworks"),
    ("XCTest", "Frameworks"),
    ("XCUIAutomation", "Frameworks"),
    ("XCTestCore", "PrivateFrameworks"),
];
const TOOLCHAIN: &str = "Developer/Toolchains/XcodeDefault.xctoolchain";

/// What a bundle is written from.
pub(crate) struct Inputs<'a> {
    /// The unpacked Xcode.app.
    pub xcode: &'a Path,
    /// The host Swift's Clang resource headers (`clang/include`).
    pub clang_headers: &'a Path,
    /// The App Intents phrase resources `sweetpad sdk export` writes.
    pub phrases: Option<&'a Path>,
    /// The macOS build the SDK came from, for `BuildMachineOSBuild`.
    pub build_machine_os_build: Option<&'a str>,
}

/// Write the bundle into `out`, a folder that doesn't exist yet.
pub(crate) fn write(inputs: &Inputs<'_>, out: &Path) -> Result<Value, String> {
    let contents = inputs.xcode.join("Contents");
    if !contents.join("Developer").is_dir() {
        return Err(format!(
            "{} isn't an Xcode.app: it has no Contents/Developer",
            inputs.xcode.display()
        ));
    }
    if out.symlink_metadata().is_ok() {
        return Err(format!("{} already exists", out.display()));
    }
    std::fs::create_dir_all(out).map_err(|e| format!("couldn't create {}: {e}", out.display()))?;
    let developer = contents.join("Developer");

    let lib = format!("{TOOLCHAIN}/usr/lib");
    for name in ["swift", "swift_static", "clang"] {
        copy(&contents.join(&lib).join(name), &out.join(&lib).join(name))?;
    }
    let runtimes = copy_swift_runtimes(&contents, out)?;
    copy(
        &contents.join(TOOLCHAIN).join("usr/include/swift"),
        &out.join(TOOLCHAIN).join("usr/include/swift"),
    )?;
    replace_clang_headers(out, inputs.clang_headers)?;

    let mut platforms = Vec::new();
    for platform in BASE_PLATFORMS {
        let source = developer
            .join("Platforms")
            .join(format!("{platform}.platform"));
        if !source.join("Developer/SDKs").is_dir() {
            return Err(format!(
                "Xcode has no {platform} SDK in {}",
                source.display()
            ));
        }
        let target = out
            .join("Developer/Platforms")
            .join(format!("{platform}.platform"));
        patch_manifest(&source.join("Info.plist"), &target.join("Info.plist"))?;
        copy_parts(&source, &target, &BASE_PARTS)?;
        link_test_frameworks(&target, platform)?;
        platforms.push(platform);
    }
    for platform in EXTRA_PLATFORMS {
        let source = developer
            .join("Platforms")
            .join(format!("{platform}.platform"));
        if !source.join("Developer/SDKs").is_dir() {
            continue;
        }
        let target = out
            .join("Developer/Platforms")
            .join(format!("{platform}.platform"));
        copy_parts(&source, &target, &EXTRA_PARTS)?;
        platforms.push(platform);
    }

    let triples = triples(out)?;
    write_file(
        &out.join("swift-sdk.json"),
        &pretty(&json!({
            "schemaVersion": "4.0",
            "targetTriples": triples,
        })),
    )?;
    write_file(&out.join("info.json"), INFO.as_bytes())?;
    write_file(
        &out.join("toolset.json"),
        &pretty(&json!({
            "schemaVersion": "1.0",
            "swiftCompiler": {
                "extraCLIOptions": ["-Xfrontend", "-enable-cross-import-overlays", "-use-ld=lld"],
            },
        })),
    )?;
    copy(&contents.join("version.plist"), &out.join("version.plist"))?;
    if let Some(build) = inputs.build_machine_os_build.filter(|b| !b.is_empty()) {
        let plist = Plist::Dict(vec![(
            "BuildMachineOSBuild".to_string(),
            Plist::String(build.to_string()),
        )]);
        write_file(
            &out.join("build-machine.plist"),
            property_list::to_xml(&plist).as_bytes(),
        )?;
    }
    if let Some(phrases) = inputs.phrases.filter(|p| p.is_file()) {
        copy(phrases, &out.join("appintents-phrases.json"))?;
    }
    Ok(json!({
        "bundlePath": out.display().to_string(),
        "platforms": platforms,
        "triples": triples.as_object().map_or(0, serde_json::Map::len),
        "swiftRuntimes": runtimes,
    }))
}

/// The artifact bundle manifest that lets SwiftPM list the folder as a
/// Swift SDK, for either Linux host.
const INFO: &str = r#"{
    "schemaVersion": "1.0",
    "artifacts": {
        "darwin": {
            "type": "swiftSDK",
            "version": "0.0.1",
            "variants": [
                {
                    "path": ".",
                    "supportedTriples": ["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"]
                }
            ]
        }
    }
}"#;

/// The settings a base platform's manifest gives every target built for it
/// without Xcode: the Swift and Clang resources in the bundle, LLD as the
/// linker, and no signing by Swift Build itself, which `sweetpad build`
/// turns back on for a signed build. Written as XML, with the source's
/// permissions.
fn patch_manifest(source: &Path, target: &Path) -> Result<(), String> {
    let data = read(source)?;
    let mut manifest = property_list::read_info(&data)
        .map_err(|e| format!("couldn't read {}: {e}", source.display()))?;
    if property_list::get(&manifest, "DefaultProperties").is_none() {
        property_list::set(&mut manifest, "DefaultProperties", Plist::Dict(Vec::new()));
    }
    let defaults = property_list::get_mut(&mut manifest, "DefaultProperties")
        .ok_or_else(|| format!("{} isn't a dictionary", source.display()))?;
    for (key, value) in [
        (
            "SWIFT_RESOURCE_DIR",
            "$(PLATFORM_DIR)/../../Toolchains/XcodeDefault.xctoolchain/usr/lib/swift",
        ),
        ("CLANG_RESOURCE_DIR", "$(SWIFT_RESOURCE_DIR)/clang"),
        ("ALTERNATE_LINKER", "lld"),
        ("CODESIGN", "true"),
    ] {
        property_list::set(defaults, key, Plist::String(value.to_string()));
    }
    write_file(target, property_list::to_xml(&manifest).as_bytes())?;
    copy_mode(source, target)
}

/// Copy a platform's `parts`, leaving out its [`HOST_PLUGINS`].
fn copy_parts(source: &Path, target: &Path, parts: &[&str]) -> Result<(), String> {
    let plugins = source.join(HOST_PLUGINS);
    for part in parts {
        if source.join(part).symlink_metadata().is_ok() {
            copy_except(&source.join(part), &target.join(part), Some(&plugins))?;
        }
    }
    Ok(())
}

/// Link XCTest and Swift Testing into the platform's SDK, which SwiftPM
/// has no framework search path option for. Xcode keeps them in the
/// platform's `Developer/Library`, apart from what an app links.
fn link_test_frameworks(platform: &Path, name: &str) -> Result<(), String> {
    let frameworks = platform.join(format!(
        "Developer/SDKs/{name}.sdk/System/Library/Frameworks"
    ));
    for (framework, folder) in TEST_FRAMEWORKS {
        let link = frameworks.join(format!("{framework}.framework"));
        let target = format!("../../../../../Library/{folder}/{framework}.framework");
        std::os::unix::fs::symlink(&target, &link)
            .map_err(|e| format!("couldn't link {}: {e}", link.display()))?;
    }
    Ok(())
}

/// The back-deployment runtimes in the toolchain's `usr/lib/swift-*/<os>`,
/// each as a file of its own rather than a link to another version's.
fn copy_swift_runtimes(contents: &Path, out: &Path) -> Result<usize, String> {
    let lib = format!("{TOOLCHAIN}/usr/lib");
    let mut copied = 0;
    for folder in sorted_entries(&contents.join(&lib))? {
        let name = folder.file_name().unwrap_or_default().to_string_lossy();
        if !name.starts_with("swift-") || !folder.is_dir() {
            continue;
        }
        for os in sorted_entries(&folder)? {
            if !os.is_dir() {
                continue;
            }
            for library in sorted_entries(&os)? {
                if library.extension().is_none_or(|e| e != "dylib") {
                    continue;
                }
                let resolved = library
                    .canonicalize()
                    .map_err(|e| format!("couldn't resolve {}: {e}", library.display()))?;
                let relative = library.strip_prefix(contents).unwrap_or(&library);
                copy(&resolved, &out.join(relative))?;
                copied += 1;
            }
        }
    }
    Ok(copied)
}

/// Swap Xcode's Clang headers for the host compiler's: the intrinsics
/// headers follow the Clang version that reads them. `swift/clang` links to
/// `clang/<version>`, so the headers land there.
fn replace_clang_headers(out: &Path, headers: &Path) -> Result<(), String> {
    if !headers.join("stddef.h").is_file() {
        return Err(format!(
            "{} has no Clang resource headers",
            headers.display()
        ));
    }
    let include = out.join(TOOLCHAIN).join("usr/lib/swift/clang/include");
    if include.symlink_metadata().is_ok() {
        std::fs::remove_dir_all(&include)
            .map_err(|e| format!("couldn't remove {}: {e}", include.display()))?;
    }
    copy(headers, &include)
}

/// A triple for each architecture of each SDK, as Swift Build matches a
/// destination to its SDK: without one it takes the host's Swift resources
/// even with the platform's SDK in place.
fn triples(out: &Path) -> Result<Value, String> {
    let mut triples = serde_json::Map::new();
    let entry = |platform: &str, sdk: &str| {
        let libraries = format!("Developer/Platforms/{platform}.platform/Developer/usr/lib");
        json!({
            "sdkRootPath": format!("Developer/Platforms/{platform}.platform/Developer/SDKs/{sdk}"),
            "includeSearchPaths": [libraries],
            "librarySearchPaths": [libraries],
            "swiftResourcesPath": format!("{TOOLCHAIN}/usr/lib/swift"),
            "swiftStaticResourcesPath": format!("{TOOLCHAIN}/usr/lib/swift_static"),
            "toolsetPaths": ["toolset.json"],
        })
    };
    for (triple, platform) in [
        ("arm64-apple-ios", "iPhoneOS"),
        ("arm64-apple-ios-simulator", "iPhoneSimulator"),
        ("x86_64-apple-ios-simulator", "iPhoneSimulator"),
        ("arm64-apple-macosx", "MacOSX"),
        ("x86_64-apple-macosx", "MacOSX"),
    ] {
        let sdk = versioned_sdk(out, platform)?;
        triples.insert(triple.to_string(), entry(platform, &sdk));
    }
    for platform in EXTRA_PLATFORMS {
        let sdks = out.join(format!(
            "Developer/Platforms/{platform}.platform/Developer/SDKs"
        ));
        if !sdks.is_dir() {
            continue;
        }
        let mut seen = std::collections::BTreeSet::new();
        for sdk in sorted_entries(&sdks)? {
            let name = sdk
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if Path::new(&name).extension().is_none_or(|e| e != "sdk") {
                continue;
            }
            let settings: Value = serde_json::from_slice(&read(&sdk.join("SDKSettings.json"))?)
                .map_err(|e| format!("couldn't read {}/SDKSettings.json: {e}", sdk.display()))?;
            if !seen.insert(
                settings["CanonicalName"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            ) {
                continue;
            }
            for target in settings["SupportedTargets"]
                .as_object()
                .into_iter()
                .flat_map(|t| t.values())
            {
                let vendor = target["LLVMTargetTripleVendor"]
                    .as_str()
                    .unwrap_or_default();
                let system = target["LLVMTargetTripleSys"].as_str().unwrap_or_default();
                let environment = target["LLVMTargetTripleEnvironment"]
                    .as_str()
                    .unwrap_or_default();
                let suffix = if environment.is_empty() {
                    String::new()
                } else {
                    format!("-{environment}")
                };
                for arch in target["Archs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    triples.insert(
                        format!("{arch}-{vendor}-{system}{suffix}"),
                        entry(platform, &name),
                    );
                }
            }
        }
    }
    Ok(Value::Object(triples))
}

/// The platform's SDK with its version in the name, `iPhoneOS27.2.sdk`.
fn versioned_sdk(out: &Path, platform: &str) -> Result<String, String> {
    let sdks = out.join(format!(
        "Developer/Platforms/{platform}.platform/Developer/SDKs"
    ));
    sorted_entries(&sdks)?
        .iter()
        .filter_map(|p| p.file_name()?.to_str().map(str::to_string))
        .find(|name| {
            name.strip_prefix(platform)
                .and_then(|rest| rest.strip_suffix(".sdk"))
                .and_then(|version| version.split_once('.'))
                .is_some_and(|(major, minor)| {
                    [major, minor]
                        .iter()
                        .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                })
        })
        .ok_or_else(|| format!("no versioned {platform} SDK in {}", sdks.display()))
}

/// Copy `source`, a file, folder or link, to `target`: links as links,
/// files with their permissions. Compiler caches (`prebuilt-modules`) stay
/// behind. Files are read and written in full rather than with the
/// kernel's copy calls, which can leave zeroes in a file copied from a
/// shared Mac folder.
fn copy(source: &Path, target: &Path) -> Result<(), String> {
    copy_except(source, target, None)
}

/// [`copy`], leaving out `except` wherever it is in `source`.
fn copy_except(source: &Path, target: &Path, except: Option<&Path>) -> Result<(), String> {
    let meta = source
        .symlink_metadata()
        .map_err(|e| format!("couldn't read {}: {e}", source.display()))?;
    let fail = |e: std::io::Error| {
        format!(
            "couldn't copy {} to {}: {e}",
            source.display(),
            target.display()
        )
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(fail)?;
    }
    if meta.file_type().is_symlink() {
        let link = std::fs::read_link(source).map_err(fail)?;
        return std::os::unix::fs::symlink(link, target).map_err(fail);
    }
    if meta.is_dir() {
        std::fs::create_dir_all(target).map_err(fail)?;
        for child in sorted_entries(source)? {
            let name = child.file_name().unwrap_or_default();
            if name == "prebuilt-modules" || except == Some(child.as_path()) {
                continue;
            }
            copy_except(&child, &target.join(name), except)?;
        }
        return Ok(());
    }
    let mut input = std::fs::File::open(source).map_err(fail)?;
    let mut output = std::fs::File::create(target).map_err(fail)?;
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = input.read(&mut buffer).map_err(fail)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read]).map_err(fail)?;
    }
    copy_mode(source, target)
}

fn copy_mode(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = source
        .metadata()
        .map_err(|e| format!("couldn't read {}: {e}", source.display()))?
        .permissions()
        .mode()
        & 0o7777;
    std::fs::set_permissions(target, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("couldn't set the permissions of {}: {e}", target.display()))
}

fn sorted_entries(folder: &Path) -> Result<Vec<PathBuf>, String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(folder)
        .map_err(|e| format!("couldn't list {}: {e}", folder.display()))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("couldn't list {}: {e}", folder.display()))?;
    entries.sort();
    Ok(entries)
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("couldn't read {}: {e}", path.display()))
}

fn write_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("couldn't write {}: {e}", path.display());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(fail)?;
    }
    std::fs::write(path, contents).map_err(fail)
}

fn pretty(value: &Value) -> Vec<u8> {
    let mut text = serde_json::to_vec_pretty(value).unwrap_or_default();
    text.push(b'\n');
    text
}

/// The host Swift's Clang resource headers, from the compiler of the
/// pinned Swift release on `PATH`.
pub(crate) fn host_clang_headers(swift: &str) -> Result<PathBuf, String> {
    let output = std::process::Command::new("swiftc")
        .arg("-print-target-info")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("couldn't run swiftc for its Clang headers: {e}"))?;
    let info: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("swiftc -print-target-info printed no target info: {e}"))?;
    let version = info["compilerVersion"].as_str().unwrap_or_default();
    if !version.contains(&format!("Swift version {swift}")) {
        return Err(format!(
            "the SDK needs Swift {swift} on PATH, not '{version}'"
        ));
    }
    let resources = info["paths"]["runtimeResourcePath"]
        .as_str()
        .ok_or("swiftc -print-target-info named no resource folder")?;
    Ok(Path::new(resources).join("clang/include"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    fn file(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn link(path: &Path, target: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, path).unwrap();
    }

    fn platform(developer: &Path, name: &str, settings: &str) {
        let root = developer.join(format!("Platforms/{name}.platform"));
        file(
            &root.join("Info.plist"),
            &property_list::to_xml(&Plist::Dict(vec![
                ("CFBundleName".into(), Plist::String(name.into())),
                (
                    "DefaultProperties".into(),
                    Plist::Dict(vec![(
                        "ARCHS".into(),
                        Plist::String("$(ARCHS_STANDARD)".into()),
                    )]),
                ),
            ])),
        );
        file(&root.join("version.plist"), "platform version");
        let sdk = root.join(format!("Developer/SDKs/{name}.sdk"));
        file(&sdk.join("SDKSettings.json"), settings);
        file(
            &sdk.join("System/Library/Frameworks/UIKit.framework/UIKit.tbd"),
            "tbd",
        );
        link(
            &root.join(format!("Developer/SDKs/{name}27.2.sdk")),
            &format!("{name}.sdk"),
        );
        file(
            &root.join("Developer/Library/Frameworks/XCTest.framework/XCTest"),
            "xctest",
        );
        file(
            &root.join("Developer/usr/lib/libXCTestSwiftSupport.dylib"),
            "dylib",
        );
        file(
            &root.join("Developer/usr/lib/swift/host/plugins/libSwiftUIMacros.dylib"),
            "plugin",
        );
        file(
            &root.join("Developer/Library/Xcode/Specifications/Platform.xcspec"),
            "spec",
        );
        file(
            &root.join("Library/Application Support/MessagesApplicationExtensionStub/Stub"),
            "stub",
        );
        let runner = root.join("Developer/Library/Xcode/Agents/XCTRunner.app");
        file(&runner.join("XCTRunner"), "runner");
        link(&runner.join("runner-alias"), "XCTRunner");
        file(
            &root.join("Developer/Library/Xcode/Unwanted/file"),
            "left out",
        );
    }

    fn xcode(root: &Path) -> PathBuf {
        let app = root.join("Xcode.app");
        let developer = app.join("Contents/Developer");
        file(&app.join("Contents/version.plist"), "xcode version");
        let lib = app.join(format!("Contents/{TOOLCHAIN}/usr/lib"));
        file(&lib.join("swift/iphoneos/libswiftCore.tbd"), "core");
        file(
            &lib.join("swift/iphoneos/prebuilt-modules/27.2/Swift.swiftmodule"),
            "cache",
        );
        link(&lib.join("swift/clang"), "../clang/21");
        file(&lib.join("clang/21/include/arm_neon.h"), "xcode header");
        file(&lib.join("clang/21/lib/darwin/libclang_rt.ios.a"), "rt");
        file(&lib.join("swift_static/macosx/libswiftCore.a"), "static");
        file(
            &lib.join("swift-5.0/iphoneos/libswiftCompatibility50.dylib.real"),
            "runtime",
        );
        link(
            &lib.join("swift-5.0/iphoneos/libswiftCompatibility50.dylib"),
            "libswiftCompatibility50.dylib.real",
        );
        file(
            &lib.join("swift-5.0/iphoneos/swift-tool"),
            "macOS executable",
        );
        file(
            &app.join(format!("Contents/{TOOLCHAIN}/usr/include/swift/bridging")),
            "bridging",
        );
        let settings = |vendor: &str, sys: &str, env: &str, archs: &str| {
            format!(
                r#"{{"CanonicalName":"{sys}27.2","SupportedTargets":{{"{sys}":{{"Archs":[{archs}],"LLVMTargetTripleVendor":"{vendor}","LLVMTargetTripleSys":"{sys}","LLVMTargetTripleEnvironment":"{env}"}}}}}}"#
            )
        };
        platform(
            &developer,
            "iPhoneOS",
            &settings("apple", "ios", "", r#""arm64""#),
        );
        platform(
            &developer,
            "iPhoneSimulator",
            &settings("apple", "ios", "simulator", r#""arm64","x86_64""#),
        );
        platform(
            &developer,
            "MacOSX",
            &settings("apple", "macosx", "", r#""arm64","x86_64""#),
        );
        platform(
            &developer,
            "WatchOS",
            &settings("apple", "watchos", "", r#""arm64_32","arm64""#),
        );
        app
    }

    /// A bundle written from the fake Xcode, with the fake Xcode and the
    /// report.
    fn written(dir: &TempDir) -> (PathBuf, PathBuf, Value) {
        let app = xcode(&dir.join("in"));
        let headers = dir.join("host/clang/include");
        file(&headers.join("stddef.h"), "host header");
        let phrases = dir.join("phrases.json");
        file(&phrases, "{}");
        let out = dir.join("darwin.artifactbundle");
        let report = write(
            &Inputs {
                xcode: &app,
                clang_headers: &headers,
                phrases: Some(&phrases),
                build_machine_os_build: Some("25A100"),
            },
            &out,
        )
        .unwrap();
        (app, out, report)
    }

    #[test]
    fn a_bundle_has_the_toolchain_parts_builds_read() {
        let dir = TempDir::new("sweetpad-sdk-bundle");
        let (_, out, report) = written(&dir);
        assert_eq!(
            report["platforms"],
            json!(["iPhoneOS", "MacOSX", "iPhoneSimulator", "WatchOS"])
        );
        assert_eq!(report["swiftRuntimes"], 1);

        let lib = out.join(format!("{TOOLCHAIN}/usr/lib"));
        assert!(lib.join("swift/iphoneos/libswiftCore.tbd").is_file());
        assert!(!lib.join("swift/iphoneos/prebuilt-modules").exists());
        // The host's headers replace Xcode's, through the link to clang/21.
        assert_eq!(
            std::fs::read_link(lib.join("swift/clang")).unwrap(),
            Path::new("../clang/21")
        );
        assert!(lib.join("clang/21/include/stddef.h").is_file());
        assert!(!lib.join("clang/21/include/arm_neon.h").exists());
        assert!(lib.join("clang/21/lib/darwin/libclang_rt.ios.a").is_file());
        // A runtime is its own file, not a link to another version's.
        let runtime = lib.join("swift-5.0/iphoneos/libswiftCompatibility50.dylib");
        assert!(runtime.symlink_metadata().unwrap().is_file());
        assert!(!runtime.with_file_name("swift-tool").exists());
        assert!(
            out.join(format!("{TOOLCHAIN}/usr/include/swift/bridging"))
                .is_file()
        );
    }

    #[test]
    fn a_bundle_has_each_platform_and_nothing_that_runs() {
        let dir = TempDir::new("sweetpad-sdk-bundle-platforms");
        let (app, out, _) = written(&dir);
        let ios = out.join("Developer/Platforms/iPhoneOS.platform");
        let manifest =
            property_list::parse(&std::fs::read(ios.join("Info.plist")).unwrap()).unwrap();
        let defaults = property_list::get(&manifest, "DefaultProperties").unwrap();
        assert_eq!(
            property_list::string(defaults, "ARCHS"),
            Some("$(ARCHS_STANDARD)")
        );
        assert_eq!(
            property_list::string(defaults, "ALTERNATE_LINKER"),
            Some("lld")
        );
        assert_eq!(property_list::string(defaults, "CODESIGN"), Some("true"));
        assert_eq!(
            property_list::string(defaults, "CLANG_RESOURCE_DIR"),
            Some("$(SWIFT_RESOURCE_DIR)/clang")
        );
        let xctest =
            ios.join("Developer/SDKs/iPhoneOS.sdk/System/Library/Frameworks/XCTest.framework");
        assert_eq!(
            std::fs::read_link(&xctest).unwrap(),
            Path::new("../../../../../Library/Frameworks/XCTest.framework")
        );
        assert!(xctest.join("XCTest").is_file());
        assert!(
            ios.join("Library/Application Support/MessagesApplicationExtensionStub/Stub")
                .is_file()
        );
        let runner = ios.join("Developer/Library/Xcode/Agents/XCTRunner.app");
        assert!(runner.join("XCTRunner").is_file());
        assert_eq!(
            std::fs::read_link(runner.join("runner-alias")).unwrap(),
            Path::new("XCTRunner")
        );
        assert!(!ios.join("Developer/Library/Xcode/Specifications").exists());
        assert!(!ios.join("Developer/Library/Xcode/Unwanted").exists());

        // Another platform keeps its parts as Xcode has them.
        let watch = out.join("Developer/Platforms/WatchOS.platform");
        assert!(
            watch
                .join("Developer/Library/Xcode/Specifications/Platform.xcspec")
                .is_file()
        );
        assert!(
            !watch
                .join("Developer/SDKs/WatchOS.sdk/System/Library/Frameworks/XCTest.framework")
                .exists()
        );
        let manifest = std::fs::read(watch.join("Info.plist")).unwrap();
        assert_eq!(
            manifest,
            std::fs::read(app.join("Contents/Developer/Platforms/WatchOS.platform/Info.plist"))
                .unwrap()
        );
        for platform in ["iPhoneOS", "WatchOS"] {
            let lib = out.join(format!(
                "Developer/Platforms/{platform}.platform/Developer/usr/lib"
            ));
            assert!(lib.join("libXCTestSwiftSupport.dylib").is_file());
            assert!(!lib.join("swift/host").exists(), "{platform}");
        }
    }

    #[test]
    fn a_bundle_describes_itself_to_swiftpm_and_sweetpad() {
        let dir = TempDir::new("sweetpad-sdk-bundle-metadata");
        let (_, out, _) = written(&dir);
        let metadata: Value =
            serde_json::from_slice(&std::fs::read(out.join("swift-sdk.json")).unwrap()).unwrap();
        let triples = metadata["targetTriples"].as_object().unwrap();
        assert_eq!(
            triples.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "arm64-apple-ios",
                "arm64-apple-ios-simulator",
                "arm64-apple-macosx",
                "arm64-apple-watchos",
                "arm64_32-apple-watchos",
                "x86_64-apple-ios-simulator",
                "x86_64-apple-macosx",
            ]
        );
        assert_eq!(
            triples["arm64-apple-ios"]["sdkRootPath"],
            "Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS27.2.sdk"
        );
        assert_eq!(
            triples["arm64-apple-watchos"]["sdkRootPath"],
            "Developer/Platforms/WatchOS.platform/Developer/SDKs/WatchOS.sdk"
        );
        assert!(out.join("info.json").is_file() && out.join("toolset.json").is_file());
        assert_eq!(
            std::fs::read_to_string(out.join("version.plist")).unwrap(),
            "xcode version"
        );
        assert!(
            std::fs::read_to_string(out.join("build-machine.plist"))
                .unwrap()
                .contains("<string>25A100</string>")
        );
        assert!(out.join("appintents-phrases.json").is_file());
        for runs in ["toolset", "OpenAppleMacrosServer", "darwin-sdk-version.txt"] {
            assert!(!out.join(runs).exists(), "{runs}");
        }
    }

    #[test]
    fn a_bundle_needs_the_base_platforms_and_a_new_folder() {
        let dir = TempDir::new("sweetpad-sdk-bundle-missing");
        let app = xcode(&dir.join("in"));
        let headers = dir.join("headers");
        file(&headers.join("stddef.h"), "");
        let inputs = Inputs {
            xcode: &app,
            clang_headers: &headers,
            phrases: None,
            build_machine_os_build: None,
        };
        std::fs::create_dir_all(dir.join("exists")).unwrap();
        assert!(
            write(&inputs, &dir.join("exists"))
                .unwrap_err()
                .contains("already exists")
        );
        std::fs::remove_dir_all(app.join("Contents/Developer/Platforms/MacOSX.platform")).unwrap();
        assert!(
            write(&inputs, &dir.join("out"))
                .unwrap_err()
                .contains("no MacOSX SDK")
        );
    }
}
