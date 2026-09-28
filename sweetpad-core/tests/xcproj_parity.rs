//! The two project formats resolve to the same build settings.
//!
//! `BuildContext` caches one parsed document and answers every
//! [`BuildContext::resolve`] from it, so the format is decided once, at open.
//! This suite pins the consequence: hand it the same project written both
//! ways and every setting it resolves agrees.
//!
//! The pair is built at run time rather than checked in twice. The fixture
//! `_xcproj/SweetpadCIApp.xcodeproj/project.xcproj` is Xcode 27.2's conversion
//! of `_synthetic-objectversion-110`'s `project.pbxproj`, so copying that
//! project and swapping the document in gives two trees that differ in nothing
//! else — which is what makes a difference in the output attributable to the
//! reader.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sweetpad_core::build_context::{BuildContext, ResolveQuery};
use sweetpad_core::scratch::ScratchDir;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("SWEETPAD_LIB_DIR")).join("fixtures")
}

fn pbxproj_project() -> PathBuf {
    fixtures().join("_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj")
}

/// A copy of the pbxproj project with its document replaced by the converted
/// one. Each test lays down a copy of its own, which goes when the returned
/// guard drops.
fn xcproj_project() -> (ScratchDir, PathBuf) {
    let root = ScratchDir::new("sweetpad-xcproj-parity").unwrap();
    copy_tree(
        &fixtures().join("_synthetic-objectversion-110/project"),
        &root.join("project"),
    );
    let project = root.join("project/SweetpadCIApp.xcodeproj");
    std::fs::remove_file(project.join("project.pbxproj")).unwrap();
    std::fs::copy(
        fixtures().join("_xcproj/SweetpadCIApp.xcodeproj/project.xcproj"),
        project.join("project.xcproj"),
    )
    .unwrap();
    // A resolved setting holds the project path the way xcodebuild spells it
    // (`/var/…`, symlinks resolved, `/private` dropped), so the copy is
    // compared in that spelling.
    let project = sweetpad_lib::project::standardize(&project);
    (root, project)
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// Everything a resolved setting can hold that the two copies differ in
/// legitimately: their own directory, and the DerivedData container name,
/// whose 28-character hash is derived from that directory.
fn normalize(settings: &BTreeMap<String, String>, project: &Path) -> BTreeMap<String, String> {
    let dir = sweetpad_lib::project::standardize(project.parent().unwrap())
        .display()
        .to_string();
    settings
        .iter()
        .map(|(k, v)| (k.clone(), mask_hash(&v.replace(&dir, "<DIR>"))))
        .collect()
}

fn mask_hash(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let run: String = chars.iter().skip(i + 1).take(28).collect();
        let ends_run = chars.get(i + 29).is_none_or(|c| !c.is_ascii_alphanumeric());
        if chars[i] == '-'
            && run.len() == 28
            && ends_run
            && run.chars().all(|c| c.is_ascii_lowercase())
        {
            out.push_str("-<HASH>");
            i += 29;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

#[test]
fn both_formats_describe_the_same_project() {
    let (_dir, xc) = xcproj_project();
    let a = BuildContext::open(&pbxproj_project()).unwrap();
    let b = BuildContext::open(&xc).unwrap();

    let names = |ctx: &BuildContext| -> Vec<String> {
        ctx.project.targets.iter().map(|t| t.name.clone()).collect()
    };
    assert_eq!(names(&a), names(&b));
    assert_eq!(a.project.configurations, b.project.configurations);
    assert_eq!(a.project.schemes, b.project.schemes);
    assert_eq!(a.project.name, b.project.name);
}

#[test]
fn both_formats_resolve_the_same_settings() {
    let (_dir, xc) = xcproj_project();
    let pbx = pbxproj_project();
    let a = BuildContext::open(&pbx).unwrap();
    let b = BuildContext::open(&xc).unwrap();

    for target in a.project.targets.iter().map(|t| &t.name) {
        for configuration in &a.project.configurations {
            let query = ResolveQuery::new(target, configuration, "macosx", "arm64");
            let ra = a.resolve(&query).unwrap();
            let rb = b.resolve(&query).unwrap();
            assert_eq!(
                ra.product_type, rb.product_type,
                "{target}/{configuration} product type"
            );
            let (na, nb) = (normalize(&ra.settings, &pbx), normalize(&rb.settings, &xc));
            let mut keys: Vec<&String> = na.keys().chain(nb.keys()).collect();
            keys.sort_unstable();
            keys.dedup();
            let differing: Vec<String> = keys
                .into_iter()
                .filter(|k| na.get(*k) != nb.get(*k))
                .map(|k| format!("{k}: {:?} vs {:?}", na.get(k), nb.get(k)))
                .collect();
            assert!(
                differing.is_empty(),
                "{target}/{configuration}:\n{}",
                differing.join("\n")
            );
        }
    }
}

/// The test bundle's host reaches the settings that depend on it. The pbxproj
/// records the host in the root object's `TargetAttributes` and the converted
/// document doesn't carry it at all, so this only holds because both readers
/// fall back to the application the bundle depends on.
#[test]
fn a_test_bundle_builds_into_its_host() {
    let (_dir, xc) = xcproj_project();
    for project in [pbxproj_project().as_path(), xc.as_path()] {
        let ctx = BuildContext::open(project).unwrap();
        let resolved = ctx
            .resolve(&ResolveQuery::new(
                "SweetpadCIAppTests",
                "Debug",
                "macosx",
                "arm64",
            ))
            .unwrap();
        assert_eq!(
            resolved
                .settings
                .get("TARGET_BUILD_SUBPATH")
                .map(String::as_str),
            Some("/SweetpadCIApp.app/Contents/PlugIns"),
            "{}",
            project.display()
        );
    }
}

/// Both formats carry the project's development region through to
/// `DEVELOPMENT_LANGUAGE`, as xcodebuild reports it for a project that isn't
/// in English.
#[test]
fn the_development_region_names_the_development_language() {
    let root = ScratchDir::new("sweetpad-region").unwrap();
    let (_xc_dir, xc) = xcproj_project();
    let mut projects = Vec::new();
    for (tag, from, document, before, after) in [
        (
            "pbxproj",
            pbxproj_project(),
            "project.pbxproj",
            "developmentRegion = en;",
            "developmentRegion = zh_CN;",
        ),
        (
            "xcproj",
            xc.clone(),
            "project.xcproj",
            r#""development": "en""#,
            r#""development": "zh_CN""#,
        ),
    ] {
        let copy = root.join(tag);
        copy_tree(from.parent().unwrap(), &copy);
        let project = copy.join("SweetpadCIApp.xcodeproj");
        let path = project.join(document);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(before), "{tag} fixture changed");
        std::fs::write(&path, text.replace(before, after)).unwrap();
        projects.push(project);
    }
    for project in &projects {
        let settings = BuildContext::open(project)
            .unwrap()
            .resolve(&ResolveQuery::new(
                "SweetpadCIApp",
                "Debug",
                "macosx",
                "arm64",
            ))
            .unwrap()
            .settings;
        assert_eq!(
            settings.get("DEVELOPMENT_LANGUAGE").map(String::as_str),
            Some("zh_CN"),
            "{}",
            project.display()
        );
    }
}

/// The BSP server answers the same way for both formats.
///
/// It is the one consumer that reaches past [`BuildContext`] on its own — it
/// lists targets from disk and stamps the project file to decide what is
/// stale — so a session driven end to end is what proves the format reaches
/// an editor, not just the resolver.
#[test]
fn the_bsp_server_answers_the_same_for_both_formats() {
    let (_dir, xc_project) = xcproj_project();
    let pbx = bsp_session(&pbxproj_project());
    let xc = bsp_session(&xc_project);
    assert_eq!(pbx.0, xc.0, "targets");
    assert!(!pbx.1.is_empty(), "no compiler arguments for the pbxproj");
    assert_eq!(pbx.1, xc.1, "compiler arguments");
}

/// A `bsp-server` command that keeps what a session writes in Cargo's scratch
/// space for integration tests. The server resolves against the active Xcode,
/// and the parsed catalog it caches would otherwise land in the user's
/// `~/.cache/sweetpad`. Its home is there too, as `CFFIXED_USER_HOME`: the
/// DerivedData locator and the `xcodebuild` a prepare runs both follow that,
/// not `HOME`, so what the warm-up after `build/initialized` builds stays out
/// of the user's DerivedData.
fn bsp_server() -> std::process::Command {
    let home = concat!(env!("CARGO_TARGET_TMPDIR"), "/home");
    std::fs::create_dir_all(home).unwrap();
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_bsp-server"));
    cmd.env("SWEETPAD_CACHE_DIR", env!("CARGO_TARGET_TMPDIR"))
        .env("HOME", home)
        .env("CFFIXED_USER_HOME", home);
    cmd
}

/// A scripted session asking for the project's targets and for the compiler
/// arguments of one source file: the target uris, and the arguments with the
/// two copies' own paths masked out.
fn bsp_session(project: &Path) -> (Vec<String>, Vec<String>) {
    use std::io::{Read, Write};

    let dir = project.parent().unwrap();
    let source = format!("file://{}/Sources/App/ContentView.swift", dir.display());
    let messages = [
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "build/initialize", "params": {
            "displayName": "parity", "version": "1", "bspVersion": "2.2.0",
            "rootUri": format!("file://{}", dir.display()),
            "capabilities": {"languageIds": ["swift"]}}}),
        serde_json::json!({"jsonrpc": "2.0", "method": "build/initialized", "params": {}}),
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "workspace/buildTargets", "params": {}}),
        serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "textDocument/sourceKitOptions", "params": {
            "textDocument": {"uri": source},
            "target": {"uri": "sweetpad://target/SweetpadCIMac"},
            "language": "swift"}}),
        serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "build/shutdown", "params": {}}),
        serde_json::json!({"jsonrpc": "2.0", "method": "build/exit", "params": {}}),
    ];
    let mut input = Vec::new();
    for message in &messages {
        let body = message.to_string();
        input.extend(format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes());
    }

    let mut child = bsp_server()
        .args(["bsp", "--project", &project.display().to_string()])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    let _ = child.wait();

    let frames = parse_frames(&out);
    let result = |id: i64| {
        frames
            .iter()
            .find(|f| f.get("id").and_then(serde_json::Value::as_i64) == Some(id))
            .and_then(|f| f.get("result"))
    };
    let strings = |value: Option<&serde_json::Value>| -> Vec<String> {
        value
            .and_then(serde_json::Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str()).map(mask_hash).collect())
            .unwrap_or_default()
    };
    let targets = result(2)
        .and_then(|r| r.get("targets"))
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|t| t.pointer("/id/uri").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    // The source list is built on the canonicalized project dir, and the
    // settings on the standardized one: `/private/var/…` and `/var/…` for the
    // copy under `$TMPDIR`. The longer spelling goes first, since it holds the
    // shorter.
    let real = std::fs::canonicalize(dir).unwrap().display().to_string();
    let dir = dir.display().to_string();
    let arguments = strings(result(3).and_then(|r| r.get("compilerArguments")))
        .iter()
        .map(|a| a.replace(&real, "<DIR>").replace(&dir, "<DIR>"))
        .collect();
    (targets, arguments)
}

fn parse_frames(out: &str) -> Vec<serde_json::Value> {
    let mut frames = Vec::new();
    let mut rest = out;
    while let Some(header) = rest.find("Content-Length:") {
        rest = &rest[header + "Content-Length:".len()..];
        let Some(separator) = rest.find("\r\n\r\n") else {
            break;
        };
        let length: usize = rest[..separator].trim().parse().unwrap_or(0);
        let start = separator + 4;
        let end = (start + length).min(rest.len());
        if let Ok(frame) = serde_json::from_str(&rest[start..end]) {
            frames.push(frame);
        }
        rest = &rest[end..];
    }
    frames
}
