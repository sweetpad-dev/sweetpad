//! The PIF emitter against `xcodebuild -dumpPIF`.
//!
//! Each case pairs a workspace or project with the PIF Xcode 27 dumped for it,
//! the absolute directory it sat in rewritten to a placeholder. GUIDs are
//! compared as they are: the emitter derives them the way Xcode does. The
//! comparison leaves out what doesn't come from the project:
//!
//! - SwiftPM package projects, which come from SwiftPM itself (its
//!   `dump-pif`), not from the emitter.
//! - Signatures, Xcode's cache keys, compared as the object each one names.
//! - The order of build files inside a phase, which has no build meaning and
//!   where Xcode's placement of a synchronized folder's files is unresolved.
//! - `provisioningStyle`, which Xcode's signing model decides when
//!   `CODE_SIGN_STYLE` is unset; Swift Build reads the setting itself
//!   whenever it is set.
//!
//! What remains is counted, and each case may not exceed its baseline: a
//! change that lowers a count should lower the baseline with it. The
//! leftovers are Xcode's random GUIDs and the build-rule names it takes from
//! file-type display names.
//!
//! The probe and Linux fixtures live in `ci/`. The corpus cases run only where
//! `corpus/` is checked out; their dumps are in `fixtures/_pif/xcode-27.0.0/`.
//! Refresh a dump with `xcodebuild -dumpPIF <out.json> -project <xcodeproj>`
//! (or `-workspace`), then drop the package objects and rewrite the directory.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sweetpad_lib::pif::pif;
use sweetpad_lib::project::standardize;

fn lib_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every difference between the emitted PIF for `container` and the dump,
/// once `placeholder` in the dump is `root` again.
fn differences(container: &Path, dump: &Path, placeholder: &str, root: &Path) -> Vec<String> {
    let root = standardize(root);
    let text = fs::read_to_string(dump)
        .unwrap()
        .replace(placeholder, &root.to_string_lossy());
    let expected: Vec<Value> = serde_json::from_str(&text).unwrap();
    let emitted = pif(container).unwrap();
    let expected = Objects::of(&expected);
    let emitted = Objects::of(&emitted);
    let mut out = Vec::new();
    for (key, contents) in &expected.objects {
        match emitted.objects.get(key) {
            Some(ours) => Diff {
                ours: &emitted,
                theirs: &expected,
                out: &mut out,
            }
            .compare(ours, contents, key),
            None => out.push(format!("{key}: missing")),
        }
    }
    for key in emitted.objects.keys() {
        if !expected.objects.contains_key(key) {
            out.push(format!("{key}: not in the dump"));
        }
    }
    out.sort();
    out
}

/// A PIF's objects by type and GUID, without SwiftPM packages, and the
/// object each signature names.
struct Objects {
    objects: HashMap<String, Value>,
    signatures: HashMap<String, String>,
}

impl Objects {
    fn of(pif: &[Value]) -> Self {
        let mut objects = HashMap::new();
        let mut signatures = HashMap::new();
        let mut packages: Vec<String> = Vec::new();
        for object in pif {
            let kind = object["type"].as_str().unwrap_or_default();
            let guid = object["contents"]["guid"].as_str().unwrap_or_default();
            let signature = object["signature"].as_str().unwrap_or_default();
            if kind == "project" && guid.starts_with("PACKAGE") {
                packages.push(signature.to_string());
                packages.extend(
                    object["contents"]["targets"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|s| s.as_str().map(str::to_string)),
                );
                continue;
            }
            let key = if kind == "workspace" {
                "workspace".to_string()
            } else {
                format!("{kind}:{guid}")
            };
            signatures.insert(signature.to_string(), key.clone());
            objects.insert(key, object["contents"].clone());
        }
        for signature in &packages {
            if let Some(key) = signatures.remove(signature) {
                objects.remove(&key);
            }
        }
        Self {
            objects,
            signatures,
        }
    }

    fn named(&self, signatures: &[Value]) -> Vec<String> {
        signatures
            .iter()
            .filter_map(|s| self.signatures.get(s.as_str()?).cloned())
            .collect()
    }
}

struct Diff<'a> {
    ours: &'a Objects,
    theirs: &'a Objects,
    out: &'a mut Vec<String>,
}

impl Diff<'_> {
    fn compare(&mut self, ours: &Value, theirs: &Value, at: &str) {
        match (ours, theirs) {
            (Value::Object(a), Value::Object(b)) => {
                for (key, value) in b {
                    let path = format!("{at}.{key}");
                    match a.get(key) {
                        Some(mine) => self.compare(mine, value, &path),
                        None => self.out.push(format!("{path}: missing")),
                    }
                }
                for key in a.keys().filter(|k| !b.contains_key(*k)) {
                    self.out.push(format!("{at}.{key}: not in the dump"));
                }
            }
            (Value::Array(a), Value::Array(b)) => self.compare_lists(a, b, at),
            _ if at.ends_with(".provisioningStyle") => {}
            _ if ours != theirs => self.out.push(format!("{at}: {ours} != {theirs}")),
            _ => {}
        }
    }

    /// Lists of objects with GUIDs pair up by GUID; `projects` and `targets`
    /// list signatures, compared as the objects they name.
    fn compare_lists(&mut self, a: &[Value], b: &[Value], at: &str) {
        let field = at.rsplit('.').next().unwrap_or_default();
        if field == "projects" || field == "targets" {
            let (mine, dumped) = (self.ours.named(a), self.theirs.named(b));
            if mine != dumped {
                self.out.push(format!("{at}: {mine:?} != {dumped:?}"));
            }
            return;
        }
        let (Some(mine), Some(dumped)) = (by_guid(a), by_guid(b)) else {
            if a.len() == b.len() {
                for (i, (mine, dumped)) in a.iter().zip(b).enumerate() {
                    self.compare(mine, dumped, &format!("{at}[{i}]"));
                }
            } else {
                self.out.push(format!("{at}: {a:?} != {b:?}"));
            }
            return;
        };
        let mine_map: HashMap<&str, &Value> = mine.iter().copied().collect();
        let dumped_map: HashMap<&str, &Value> = dumped.iter().copied().collect();
        for (guid, value) in &dumped {
            match mine_map.get(guid) {
                Some(m) => self.compare(m, value, &format!("{at}[{guid}]")),
                None => self.out.push(format!("{at}[{guid}]: missing")),
            }
        }
        for (guid, _) in mine.iter().filter(|(g, _)| !dumped_map.contains_key(g)) {
            self.out.push(format!("{at}[{guid}]: not in the dump"));
        }
        let shared = |list: &[(&str, &Value)], other: &HashMap<&str, &Value>| -> Vec<String> {
            list.iter()
                .filter(|(g, _)| other.contains_key(g))
                .map(|(g, _)| (*g).to_string())
                .collect()
        };
        if field != "buildFiles" && shared(&mine, &dumped_map) != shared(&dumped, &mine_map) {
            self.out.push(format!("{at}: order"));
        }
    }
}

fn by_guid(items: &[Value]) -> Option<Vec<(&str, &Value)>> {
    items
        .iter()
        .map(|v| Some((v.get("guid")?.as_str()?, v)))
        .collect()
}

/// Checks each case's differences against its baseline, reporting every
/// case over it before failing.
fn assert_within(cases: &[(String, Vec<String>, usize)]) {
    let mut over = Vec::new();
    for (name, differences, baseline) in cases {
        if differences.len() > *baseline {
            over.push(format!(
                "{name}: {} differences from xcodebuild -dumpPIF, baseline {baseline}:\n{}",
                differences.len(),
                differences.join("\n")
            ));
        } else if differences.len() < *baseline {
            eprintln!(
                "{name}: {} differences, below the baseline of {baseline}",
                differences.len()
            );
        }
    }
    assert!(over.is_empty(), "{}", over.join("\n\n"));
}

#[test]
fn linux_fixture_matches_xcodebuild_dump() {
    let dir = lib_dir().join("ci/fixture-linux");
    let found = differences(
        &dir.join("LinuxApp.xcodeproj"),
        &dir.join("LinuxApp.dumpPIF.json"),
        "$(FIXTURE_DIR)",
        &dir,
    );
    assert_within(&[("fixture-linux".into(), found, 0)]);
}

/// A synthetic project built to touch every mapping rule: copy destinations,
/// script and rule options, legacy and aggregate targets, platform filters,
/// text encodings, synchronized folders and their exceptions, a project
/// reference and a workspace in a sibling folder.
#[test]
fn probe_fixture_matches_xcodebuild_dump() {
    let dir = lib_dir().join("ci/fixture-pif-probe");
    // Xcode's random GUID for a variant group with neither a Base nor an
    // English translation, in the tree and in two targets' build files, and
    // an unnamed build rule's name.
    let cases: Vec<_> = [
        ("Probe/Probe.xcodeproj", "Probe.dumpPIF.json", 7),
        ("WS/Probe.xcworkspace", "ProbeWorkspace.dumpPIF.json", 7),
    ]
    .into_iter()
    .map(|(container, dump, baseline)| {
        let found = differences(
            &dir.join(container),
            &dir.join(dump),
            "$(FIXTURE_DIR)",
            &dir,
        );
        (container.to_string(), found, baseline)
    })
    .collect();
    assert_within(&cases);
}

#[test]
fn corpus_matches_xcodebuild_dumps() {
    // NetNewsWire's "Recovered References" group has a random GUID.
    const BASELINES: &[(&str, usize)] = &[("netnewswire", 2)];
    let corpus = lib_dir().join("corpus");
    let dumps = lib_dir().join("fixtures/_pif/xcode-27.0.0");
    let index: HashMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dumps.join("index.json")).unwrap()).unwrap();
    let mut cases = Vec::new();
    for (name, container) in &index {
        let container = corpus.join(container);
        if !container.exists() {
            continue;
        }
        let baseline = BASELINES
            .iter()
            .find(|(n, _)| n == name)
            .map_or(0, |(_, b)| *b);
        let found = differences(
            &container,
            &dumps.join(format!("{name}.json")),
            "$(CORPUS)",
            &corpus,
        );
        cases.push((name.clone(), found, baseline));
    }
    if cases.is_empty() {
        eprintln!("no corpus checkout; the corpus PIF cases were skipped");
    }
    assert_within(&cases);
}

/// The root manifest's dependencies for a project: the package folders it
/// shows and the packages it declares, as Xcode lists them when it resolves.
#[test]
fn corpus_package_dependencies_match_xcode() {
    let corpus = lib_dir().join("corpus");
    let tuist = "tuist-fixtures/examples/xcode/xcode_project_with_packages_and_tests";
    let ice_cubes: Vec<String> = [
        "AppAccount",
        "Account",
        "Conversations",
        "DesignSystem",
        "Explore",
        "Env",
        "Models",
        "StatusKit",
        "MediaUI",
        "NetworkClient",
        "Lists",
        "Notifications",
        "Timeline",
    ]
    .iter()
    .map(|p| format!(".package(path: \"$(CORPUS)/ice-cubes/Packages/{p}\")"))
    .chain(
        [
            ".package(url: \"https://github.com/evgenyneu/keychain-swift\", branch: \"master\")",
            ".package(url: \"https://github.com/SFSafeSymbols/SFSafeSymbols\", from: \"4.1.1\")",
            ".package(url: \"https://github.com/wishkit/wishkit-ios.git\", from: \"4.1.1\")",
            ".package(url: \"https://github.com/RevenueCat/purchases-ios-spm\", from: \"5.28.0\")",
        ]
        .map(String::from),
    )
    .collect();
    let cases = [
        (
            format!("{tuist}/App.xcodeproj"),
            ["LibraryA", "LibraryB"]
                .map(|p| format!(".package(path: \"$(CORPUS)/{tuist}/App/{p}\")"))
                .to_vec(),
        ),
        ("ice-cubes/IceCubesApp.xcodeproj".to_string(), ice_cubes),
    ];
    let root = standardize(&corpus).to_string_lossy().into_owned();
    for (container, expected) in cases {
        let path = corpus.join(&container);
        if !path.exists() {
            eprintln!("no {container}; skipped");
            continue;
        }
        let packages = sweetpad_lib::pif::packages(&path).unwrap();
        let manifest = sweetpad_lib::pif::manifest(&packages).replace(&root, "$(CORPUS)");
        let dependencies: Vec<&str> = manifest
            .lines()
            .filter_map(|l| l.trim().strip_suffix(','))
            .filter(|l| l.starts_with(".package("))
            .collect();
        assert_eq!(dependencies, expected, "{container}");
    }
}
