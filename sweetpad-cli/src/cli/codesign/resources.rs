//! `_CodeSignature/CodeResources`: the hash of each file of an iOS-style
//! bundle besides its executable, under the rules `codesign` writes for one.
//! The version 1 list hashes with SHA-1; the version 2 one with SHA-256,
//! and leaves out the bundle's own `Info.plist` and `PkgInfo`, which the
//! code directory hashes itself. A nested bundle's files are listed one by
//! one, as `codesign` lists them for an iOS app.

use std::path::Path;

use super::crypto::{sha1, sha256};
use super::plist::{self, Plist};

/// How a rule treats the files it matches.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Treatment {
    Hash,
    Optional,
    Omit,
}

/// A rule of `codesign`'s: its pattern as the file writes it, which files
/// it matches, its weight and what it does with them.
struct Rule {
    pattern: &'static str,
    matches: fn(&str) -> bool,
    weight: Option<f64>,
    treatment: Treatment,
}

fn any(_: &str) -> bool {
    true
}

fn in_lproj(path: &str) -> bool {
    path.contains(".lproj/")
}

fn locversion(path: &str) -> bool {
    path.contains(".lproj/") && path.ends_with("/locversion.plist")
}

fn in_base_lproj(path: &str) -> bool {
    path.starts_with("Base.lproj/")
}

const RULES: &[Rule] = &[
    Rule {
        pattern: "^.*",
        matches: any,
        weight: None,
        treatment: Treatment::Hash,
    },
    Rule {
        pattern: "^.*\\.lproj/",
        matches: in_lproj,
        weight: Some(1000.0),
        treatment: Treatment::Optional,
    },
    Rule {
        pattern: "^.*\\.lproj/locversion.plist$",
        matches: locversion,
        weight: Some(1100.0),
        treatment: Treatment::Omit,
    },
    Rule {
        pattern: "^Base\\.lproj/",
        matches: in_base_lproj,
        weight: Some(1010.0),
        treatment: Treatment::Hash,
    },
    Rule {
        pattern: "^version.plist$",
        matches: |p| p == "version.plist",
        weight: None,
        treatment: Treatment::Hash,
    },
];

const RULES2: &[Rule] = &[
    Rule {
        pattern: ".*\\.dSYM($|/)",
        matches: |p| p.ends_with(".dSYM") || p.contains(".dSYM/"),
        weight: Some(11.0),
        treatment: Treatment::Hash,
    },
    Rule {
        pattern: "^(.*/)?\\.DS_Store$",
        matches: |p| p.rsplit('/').next() == Some(".DS_Store"),
        weight: Some(2000.0),
        treatment: Treatment::Omit,
    },
    Rule {
        pattern: "^.*",
        matches: any,
        weight: None,
        treatment: Treatment::Hash,
    },
    Rule {
        pattern: "^.*\\.lproj/",
        matches: in_lproj,
        weight: Some(1000.0),
        treatment: Treatment::Optional,
    },
    Rule {
        pattern: "^.*\\.lproj/locversion.plist$",
        matches: locversion,
        weight: Some(1100.0),
        treatment: Treatment::Omit,
    },
    Rule {
        pattern: "^Base\\.lproj/",
        matches: in_base_lproj,
        weight: Some(1010.0),
        treatment: Treatment::Hash,
    },
    Rule {
        pattern: "^Info\\.plist$",
        matches: |p| p == "Info.plist",
        weight: Some(20.0),
        treatment: Treatment::Omit,
    },
    Rule {
        pattern: "^PkgInfo$",
        matches: |p| p == "PkgInfo",
        weight: Some(20.0),
        treatment: Treatment::Omit,
    },
    Rule {
        pattern: "^embedded\\.provisionprofile$",
        matches: |p| p == "embedded.provisionprofile",
        weight: Some(20.0),
        treatment: Treatment::Hash,
    },
    Rule {
        pattern: "^version\\.plist$",
        matches: |p| p == "version.plist",
        weight: Some(20.0),
        treatment: Treatment::Hash,
    },
];

/// What the heaviest rule matching `path` does with it.
fn treatment(rules: &[Rule], path: &str) -> Treatment {
    rules
        .iter()
        .filter(|rule| (rule.matches)(path))
        .max_by(|a, b| a.weight.unwrap_or(1.0).total_cmp(&b.weight.unwrap_or(1.0)))
        .map_or(Treatment::Hash, |rule| rule.treatment)
}

fn rules_plist(rules: &[Rule]) -> Plist {
    Plist::Dict(
        rules
            .iter()
            .map(|rule| {
                let value = match (rule.treatment, rule.weight) {
                    (Treatment::Hash, None) => Plist::Bool(true),
                    (treatment, weight) => {
                        let mut fields = Vec::new();
                        match treatment {
                            Treatment::Omit => fields.push(("omit".to_string(), Plist::Bool(true))),
                            Treatment::Optional => {
                                fields.push(("optional".to_string(), Plist::Bool(true)));
                            }
                            Treatment::Hash => {}
                        }
                        if let Some(weight) = weight {
                            fields.push(("weight".to_string(), Plist::Real(weight)));
                        }
                        Plist::Dict(fields)
                    }
                };
                (rule.pattern.to_string(), value)
            })
            .collect(),
    )
}

/// The `CodeResources` of the bundle at `bundle`, whose executable is
/// `executable`, relative to it.
pub(super) fn code_resources(bundle: &Path, executable: &str) -> Result<Vec<u8>, String> {
    let mut paths = Vec::new();
    collect(bundle, "", &mut paths)?;
    paths.sort();
    let mut files = Vec::new();
    let mut files2 = Vec::new();
    for path in paths {
        if path == executable || path.starts_with("_CodeSignature/") {
            continue;
        }
        let full = bundle.join(&path);
        let metadata = std::fs::symlink_metadata(&full)
            .map_err(|e| format!("couldn't read {}: {e}", full.display()))?;
        if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&full)
                .map_err(|e| format!("couldn't read {}: {e}", full.display()))?;
            if treatment(RULES2, &path) != Treatment::Omit {
                files2.push((
                    path,
                    Plist::Dict(vec![(
                        "symlink".to_string(),
                        Plist::String(target.to_string_lossy().into_owned()),
                    )]),
                ));
            }
            continue;
        }
        let bytes =
            std::fs::read(&full).map_err(|e| format!("couldn't read {}: {e}", full.display()))?;
        match treatment(RULES, &path) {
            Treatment::Omit => {}
            Treatment::Hash => files.push((path.clone(), Plist::Data(sha1(&bytes).to_vec()))),
            Treatment::Optional => files.push((
                path.clone(),
                Plist::Dict(vec![
                    ("hash".to_string(), Plist::Data(sha1(&bytes).to_vec())),
                    ("optional".to_string(), Plist::Bool(true)),
                ]),
            )),
        }
        let hash2 = ("hash2".to_string(), Plist::Data(sha256(&bytes).to_vec()));
        match treatment(RULES2, &path) {
            Treatment::Omit => {}
            Treatment::Hash => files2.push((path, Plist::Dict(vec![hash2]))),
            Treatment::Optional => files2.push((
                path,
                Plist::Dict(vec![hash2, ("optional".to_string(), Plist::Bool(true))]),
            )),
        }
    }
    let resources = Plist::Dict(vec![
        ("files".to_string(), Plist::Dict(files)),
        ("files2".to_string(), Plist::Dict(files2)),
        ("rules".to_string(), rules_plist(RULES)),
        ("rules2".to_string(), rules_plist(RULES2)),
    ]);
    Ok(plist::to_xml(&resources).into_bytes())
}

/// Every file and symlink under `dir`, as paths relative to the bundle.
fn collect(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("couldn't list {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("couldn't list {}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = format!("{prefix}{name}");
        let kind = entry
            .file_type()
            .map_err(|e| format!("couldn't read {}: {e}", entry.path().display()))?;
        if kind.is_dir() {
            collect(&entry.path(), &format!("{path}/"), out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}
