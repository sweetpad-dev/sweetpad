//! Discovery oracle: validate the container listing API — `targets`,
//! `configurations`, and `schemes` for a `.xcodeproj`, merged `schemes` for an
//! `.xcworkspace` — against the captured `xcodebuild -list -json` output
//! (`metadata/**/list.json`, written by `scripts/02_capture_metadata.py`).
//!
//! This is the discovery counterpart to the build-settings oracles: those
//! prove the *resolution* pipeline, this proves the listing surface the
//! VS Code extension drives first (scheme/target/configuration pickers).
//!
//! Scoring is per-capture set comparison plus an ordering check. Schemes that
//! `xcodebuild` synthesizes from *Swift package* manifests (a local package's
//! product/target schemes, e.g. ice-cubes' `Account` … `Timeline`) are outside
//! the pbxproj/xcscheme surface this library models, so they're subtracted
//! from the oracle set and tallied separately rather than failed.

mod common;

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use common::{JsonValue, parse_json};
use sweetpad_lib::{project, workspace};

/// All `metadata/**/list.json` captures in the corpus.
fn find_list_captures() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let root = common::fixtures_root();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name() == Some(OsStr::new("list.json"))
                && path.iter().any(|c| c == OsStr::new("metadata"))
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Map a `…/metadata/<sub…>/list.json` capture to its `…/raw/<sub…>/` root.
fn raw_root_for(capture: &Path) -> PathBuf {
    let comps: Vec<&OsStr> = capture.iter().collect();
    let metadata_idx = comps
        .iter()
        .rposition(|c| *c == OsStr::new("metadata"))
        .expect("capture path contains metadata/");
    let mut root = PathBuf::new();
    for (i, c) in comps.iter().enumerate().take(comps.len() - 1) {
        if i == metadata_idx {
            root.push("raw");
        } else {
            root.push(c);
        }
    }
    root
}

/// Find the shallowest directory entry named `name` under `root` (the
/// container the capture was taken against — nested checkouts can embed
/// same-named projects deeper down).
fn find_container(root: &Path, name: &str) -> Option<PathBuf> {
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(root.to_path_buf());
    while let Some(dir) = queue.pop_front() {
        let candidate = dir.join(name);
        if candidate.is_dir() {
            return Some(candidate);
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut subdirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && p.extension() != Some(OsStr::new("xcodeproj"))
                    && p.extension() != Some(OsStr::new("xcworkspace"))
            })
            .collect();
        subdirs.sort();
        queue.extend(subdirs);
    }
    None
}

fn string_array(obj: &JsonValue, key: &str) -> Vec<String> {
    obj.as_object()
        .and_then(|o| o.get(key))
        .and_then(JsonValue::as_array)
        .map(|a| {
            a.iter()
                .filter_map(JsonValue::as_string)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Schemes `xcodebuild -list` synthesizes from Swift package manifests rather
/// than scheme files / pbxproj targets: anything not accounted for by the
/// container's targets or scheme files while the project references packages.
/// We can't *derive* these without parsing `Package.swift`, so the oracle
/// subtracts them; the count is printed so the gap stays visible.
struct Comparison {
    label: String,
    missing: Vec<String>,
    extra: Vec<String>,
    package_only: Vec<String>,
    order_mismatch: bool,
}

fn compare_lists(
    label: &str,
    ours: &[String],
    oracle: &[String],
    package_candidates: Option<&BTreeSet<String>>,
) -> Comparison {
    let ours_set: BTreeSet<&String> = ours.iter().collect();
    let oracle_set: BTreeSet<&String> = oracle.iter().collect();
    let mut missing: Vec<String> = Vec::new();
    let mut package_only: Vec<String> = Vec::new();
    for name in oracle_set.difference(&ours_set) {
        if package_candidates.is_some_and(|pkgs| pkgs.contains(name.as_str())) {
            package_only.push((*name).clone());
        } else {
            missing.push((*name).clone());
        }
    }
    let extra: Vec<String> = ours_set
        .difference(&oracle_set)
        .map(|s| (*s).clone())
        .collect();
    // Ordering: compare the shared subsequence in capture order vs ours.
    let shared_in_oracle: Vec<&String> = oracle.iter().filter(|s| ours_set.contains(s)).collect();
    let shared_in_ours: Vec<&String> = ours.iter().filter(|s| oracle_set.contains(s)).collect();
    Comparison {
        label: label.to_string(),
        missing,
        extra,
        package_only,
        order_mismatch: shared_in_oracle != shared_in_ours,
    }
}

#[test]
fn discovery_matches_xcodebuild_list_captures() {
    let captures = find_list_captures();
    assert!(
        !captures.is_empty(),
        "no metadata/**/list.json captures found"
    );

    let mut failures: Vec<String> = Vec::new();
    let mut package_scheme_total = 0usize;
    let mut compared = 0usize;

    for capture in &captures {
        let text = fs::read_to_string(capture).expect("read list.json");
        let json = parse_json(&text).expect("parse list.json");
        let obj = json.as_object().expect("list.json object");
        let raw = raw_root_for(capture);
        let rel = capture
            .strip_prefix(common::fixtures_root())
            .unwrap_or(capture)
            .display()
            .to_string();

        if let Some(ws_json) = obj.get("workspace") {
            let name = ws_json
                .as_object()
                .and_then(|o| o.get("name"))
                .and_then(JsonValue::as_string)
                .expect("workspace name");
            let container = find_container(&raw, &format!("{name}.xcworkspace"))
                .unwrap_or_else(|| panic!("no {name}.xcworkspace under {}", raw.display()));
            let ws = workspace::open(&container).expect("open workspace");
            let ours = ws.merged_schemes();
            let oracle = string_array(ws_json, "schemes");
            let cmp = compare_lists("workspace schemes", &ours, &oracle, None);
            report(&rel, &cmp, &mut failures, &mut package_scheme_total);
            compared += 1;
        }

        if let Some(proj_json) = obj.get("project") {
            let name = proj_json
                .as_object()
                .and_then(|o| o.get("name"))
                .and_then(JsonValue::as_string)
                .expect("project name");
            let container = find_container(&raw, &format!("{name}.xcodeproj"))
                .unwrap_or_else(|| panic!("no {name}.xcodeproj under {}", raw.display()));
            let proj = project::open(&container).expect("open project");

            let our_targets: Vec<String> = proj.targets.iter().map(|t| t.name.clone()).collect();
            let cmp = compare_lists(
                "project targets",
                &our_targets,
                &string_array(proj_json, "targets"),
                None,
            );
            report(&rel, &cmp, &mut failures, &mut package_scheme_total);

            let cmp = compare_lists(
                "project configurations",
                &proj.configurations,
                &string_array(proj_json, "configurations"),
                None,
            );
            report(&rel, &cmp, &mut failures, &mut package_scheme_total);

            // Any oracle scheme that is neither one of our discovered schemes
            // nor a pbxproj target can only come from a Swift package
            // manifest — tally those instead of failing on them.
            let target_set: BTreeSet<String> = our_targets.iter().cloned().collect();
            let scheme_set: BTreeSet<String> = proj.schemes.iter().cloned().collect();
            let package_candidates: BTreeSet<String> = string_array(proj_json, "schemes")
                .into_iter()
                .filter(|s| {
                    !scheme_set.contains(s)
                        && !target_set.contains(s)
                        && !target_set.contains(s.trim_end_matches("-Package"))
                })
                .collect();
            let cmp = compare_lists(
                "project schemes",
                &proj.schemes,
                &string_array(proj_json, "schemes"),
                Some(&package_candidates),
            );
            report(&rel, &cmp, &mut failures, &mut package_scheme_total);
            compared += 1;
        }
    }

    println!(
        "discovery oracle: {compared} containers compared across {} captures; \
         {package_scheme_total} package-manifest schemes outside the pbxproj surface (tallied, not failed)",
        captures.len()
    );
    assert!(
        failures.is_empty(),
        "discovery diverged from xcodebuild -list on {} comparison(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn report(
    capture: &str,
    cmp: &Comparison,
    failures: &mut Vec<String>,
    package_scheme_total: &mut usize,
) {
    *package_scheme_total += cmp.package_only.len();
    if !cmp.package_only.is_empty() {
        println!(
            "  [package-only] {capture} :: {} :: {:?}",
            cmp.label, cmp.package_only
        );
    }
    if !cmp.missing.is_empty() || !cmp.extra.is_empty() || cmp.order_mismatch {
        failures.push(format!(
            "  {capture} :: {} :: missing={:?} extra={:?} order_mismatch={}",
            cmp.label, cmp.missing, cmp.extra, cmp.order_mismatch
        ));
    }
}
