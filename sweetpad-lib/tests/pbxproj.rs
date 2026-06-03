use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use sweetpad::pbxproj::{Value, parse};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn find_pbxproj_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            // Skip transitive package dependencies pulled in by SwiftPM during builds —
            // they are not part of the intended corpus.
            if matches!(
                p.file_name().and_then(|n| n.to_str()),
                Some(".derived" | ".cache" | "DerivedData")
            ) {
                continue;
            }
            walk(&p, out);
        } else if p.file_name() == Some(OsStr::new("project.pbxproj")) {
            out.push(p);
        }
    }
}

fn sanity_check(v: &Value) -> Result<(), String> {
    let d = v.as_dict().ok_or("top-level is not a dict")?;
    for key in [
        "archiveVersion",
        "classes",
        "objectVersion",
        "objects",
        "rootObject",
    ] {
        if !d.contains_key(key) {
            return Err(format!("missing top-level key {key}"));
        }
    }
    let objects = d["objects"].as_dict().ok_or("objects is not a dict")?;
    if objects.is_empty() {
        return Err("objects is empty".into());
    }
    let root_id = d["rootObject"]
        .as_str()
        .ok_or("rootObject is not a string")?;
    let root_obj = objects
        .get(root_id)
        .ok_or_else(|| format!("rootObject {root_id} not in objects"))?;
    let isa = root_obj
        .get("isa")
        .and_then(Value::as_str)
        .ok_or("rootObject has no isa")?;
    if isa != "PBXProject" {
        return Err(format!("rootObject isa is {isa}, expected PBXProject"));
    }
    Ok(())
}

#[test]
fn parses_every_pbxproj_in_corpus() {
    let files = find_pbxproj_files(&fixtures_root());
    assert!(
        files.len() >= 5,
        "expected at least 5 pbxproj files in fixtures/, found {}",
        files.len()
    );

    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let s = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{}: read error: {e}", path.display()));
                continue;
            }
        };
        match parse(&s) {
            Ok(v) => {
                if let Err(msg) = sanity_check(&v) {
                    failures.push(format!("{}: {msg}", path.display()));
                }
            }
            Err(e) => {
                failures.push(format!("{}: parse error: {e}", path.display()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} pbxproj fixtures failed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}
