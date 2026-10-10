//! The PIF emitter against `xcodebuild -dumpPIF`.
//!
//! Each case pairs a committed project with the dump Xcode printed for it, the
//! fixture's absolute directory rewritten to `$(FIXTURE_DIR)`. Every GUID
//! starts with the project's GUID, which Xcode derives from inputs the emitter
//! doesn't reproduce, so both sides swap it for a placeholder. Signatures are
//! cache keys, compared as the object each one names.
//!
//! Refresh a dump with `xcodebuild -dumpPIF <out.json> -project <xcodeproj>`
//! and the same directory rewrite.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sweetpad_lib::pbxproj;
use sweetpad_lib::pif::project_pif;

const FIXTURE_DIR: &str = "$(FIXTURE_DIR)";

fn ci_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ci")
}

fn normalize(pif: &Value, fixture_dir: &str) -> Value {
    let objects = pif.as_array().expect("PIF is an array of objects");
    let project_guid = objects
        .iter()
        .find(|o| o["type"] == "project")
        .and_then(|o| o["contents"]["guid"].as_str())
        .expect("PIF has a project")
        .to_string();
    let names: HashMap<String, String> = objects
        .iter()
        .map(|o| {
            let name = o["contents"]["name"].as_str().unwrap_or_default();
            (
                o["signature"].as_str().unwrap_or_default().to_string(),
                format!("<{}:{name}>", o["type"].as_str().unwrap_or_default()),
            )
        })
        .collect();
    let rewrite = |s: &str| {
        names.get(s).cloned().unwrap_or_else(|| {
            s.replace(&project_guid, "<project>:")
                .replace(fixture_dir, FIXTURE_DIR)
        })
    };
    let normalized = objects
        .iter()
        .map(|o| {
            let mut contents = walk(&o["contents"], &rewrite);
            if o["type"] == "workspace" {
                contents["guid"] = Value::from("<workspace>");
            }
            serde_json::json!({ "type": o["type"], "contents": contents })
        })
        .collect();
    Value::Array(normalized)
}

fn walk(value: &Value, rewrite: &impl Fn(&str) -> String) -> Value {
    match value {
        Value::String(s) => Value::String(rewrite(s)),
        Value::Array(items) => Value::Array(items.iter().map(|v| walk(v, rewrite)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), walk(v, rewrite)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn assert_matches_dump(xcodeproj: &Path, dump: &Path) {
    let fixture_dir = fs::canonicalize(xcodeproj.parent().unwrap()).unwrap();
    let fixture_dir = fixture_dir.to_string_lossy();
    let value = pbxproj::parse_file(&xcodeproj.join("project.pbxproj")).unwrap();
    let emitted = Value::Array(project_pif(&value, xcodeproj).unwrap());
    let expected: Value = serde_json::from_str(&fs::read_to_string(dump).unwrap()).unwrap();
    let emitted = serde_json::to_string_pretty(&normalize(&emitted, &fixture_dir)).unwrap();
    let expected = serde_json::to_string_pretty(&normalize(&expected, FIXTURE_DIR)).unwrap();
    assert_eq!(
        emitted,
        expected,
        "{} differs from its dump",
        xcodeproj.display()
    );
}

#[test]
fn linux_fixture_matches_xcodebuild_dump() {
    let dir = ci_dir().join("fixture-linux");
    assert_matches_dump(
        &dir.join("LinuxApp.xcodeproj"),
        &dir.join("LinuxApp.dumpPIF.json"),
    );
}
