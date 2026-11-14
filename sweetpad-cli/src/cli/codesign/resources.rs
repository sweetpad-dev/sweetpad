//! `_CodeSignature/CodeResources`: the hash of each file of a bundle besides
//! its executable, under the rules `codesign` writes for the bundle's shape.
//! The version 1 list hashes with SHA-1; the version 2 one with SHA-256,
//! and leaves out the bundle's own `Info.plist` and `PkgInfo`, which the
//! code directory hashes itself. An iOS-style bundle's nested bundles are
//! listed file by file. A macOS bundle's code, where its rules put code, is
//! listed as nested code instead: its code directory hash and designated
//! requirement, a nested bundle as one entry.

use std::path::Path;

use super::bundle::Flavor;
use super::crypto::{sha1, sha256};
use super::inspect;
use super::plist::{self, Plist};

/// How a rule treats the files it matches.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Treatment {
    Hash,
    Optional,
    Omit,
    Nested,
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
    path.ends_with(".lproj/locversion.plist")
}

fn in_resources(path: &str) -> bool {
    path.starts_with("Resources/")
}

fn in_resources_lproj(path: &str) -> bool {
    path.strip_prefix("Resources/")
        .is_some_and(|rest| rest.contains(".lproj/"))
}

fn resources_locversion(path: &str) -> bool {
    path.strip_prefix("Resources/")
        .is_some_and(|rest| rest.ends_with(".lproj/locversion.plist"))
}

fn in_resources_base_lproj(path: &str) -> bool {
    path.starts_with("Resources/Base.lproj/")
}

/// Where a macOS bundle keeps code: `^(Frameworks|…|Library/(…))/`.
fn in_code_folder(path: &str) -> bool {
    [
        "Frameworks/",
        "SharedFrameworks/",
        "PlugIns/",
        "Plug-ins/",
        "XPCServices/",
        "Helpers/",
        "MacOS/",
        "Library/Automator/",
        "Library/Spotlight/",
        "Library/LoginItems/",
    ]
    .iter()
    .any(|folder| path.starts_with(folder))
}

fn at_top(path: &str) -> bool {
    !path.is_empty() && !path.contains('/')
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

const MAC_RULES: &[Rule] = &[
    Rule {
        pattern: "^Resources/",
        matches: in_resources,
        weight: None,
        treatment: Treatment::Hash,
    },
    Rule {
        pattern: "^Resources/.*\\.lproj/",
        matches: in_resources_lproj,
        weight: Some(1000.0),
        treatment: Treatment::Optional,
    },
    Rule {
        pattern: "^Resources/.*\\.lproj/locversion.plist$",
        matches: resources_locversion,
        weight: Some(1100.0),
        treatment: Treatment::Omit,
    },
    Rule {
        pattern: "^Resources/Base\\.lproj/",
        matches: in_resources_base_lproj,
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

const MAC_RULES2: &[Rule] = &[
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
        pattern: "^(Frameworks|SharedFrameworks|PlugIns|Plug-ins|XPCServices|Helpers|MacOS|Library/(Automator|Spotlight|LoginItems))/",
        matches: in_code_folder,
        weight: Some(10.0),
        treatment: Treatment::Nested,
    },
    Rule {
        pattern: "^.*",
        matches: any,
        weight: None,
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
        pattern: "^Resources/",
        matches: in_resources,
        weight: Some(20.0),
        treatment: Treatment::Hash,
    },
    Rule {
        pattern: "^Resources/.*\\.lproj/",
        matches: in_resources_lproj,
        weight: Some(1000.0),
        treatment: Treatment::Optional,
    },
    Rule {
        pattern: "^Resources/.*\\.lproj/locversion.plist$",
        matches: resources_locversion,
        weight: Some(1100.0),
        treatment: Treatment::Omit,
    },
    Rule {
        pattern: "^Resources/Base\\.lproj/",
        matches: in_resources_base_lproj,
        weight: Some(1010.0),
        treatment: Treatment::Hash,
    },
    Rule {
        pattern: "^[^/]+$",
        matches: at_top,
        weight: Some(10.0),
        treatment: Treatment::Nested,
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

/// What the heaviest rule matching `path` does with it; a path no rule
/// matches is left out.
fn treatment(rules: &[Rule], path: &str) -> Option<Treatment> {
    rules
        .iter()
        .filter(|rule| (rule.matches)(path))
        .max_by(|a, b| a.weight.unwrap_or(1.0).total_cmp(&b.weight.unwrap_or(1.0)))
        .map(|rule| rule.treatment)
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
                            Treatment::Nested => {
                                fields.push(("nested".to_string(), Plist::Bool(true)));
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

/// What the walk finds at a path.
enum Found {
    File,
    Symlink,
    /// A folder under a nested-code rule with a dot in its name: a bundle,
    /// sealed whole.
    Bundle,
}

/// The `CodeResources` of the bundle whose sealed files are under `root`,
/// sealed under `flavor`'s rules, leaving out `executable`, relative to it.
pub(super) fn code_resources(
    root: &Path,
    executable: &str,
    flavor: Flavor,
) -> Result<Vec<u8>, String> {
    let (rules, rules2) = match flavor {
        Flavor::Shallow => (RULES, RULES2),
        Flavor::Mac => (MAC_RULES, MAC_RULES2),
    };
    let mut paths = Vec::new();
    collect(root, "", rules2, &mut paths)?;
    paths.sort_by(|a, b| a.0.cmp(&b.0));
    let mut files = Vec::new();
    let mut files2 = Vec::new();
    for (path, found) in paths {
        if path == executable {
            continue;
        }
        let full = root.join(&path);
        let unreadable = |e: std::io::Error| format!("couldn't read {}: {e}", full.display());
        let sealed = treatment(rules2, &path).filter(|t| *t != Treatment::Omit);
        match found {
            Found::Symlink => {
                let target = std::fs::read_link(&full).map_err(unreadable)?;
                if sealed.is_some() {
                    files2.push((
                        path,
                        Plist::Dict(vec![(
                            "symlink".to_string(),
                            Plist::String(target.to_string_lossy().into_owned()),
                        )]),
                    ));
                }
            }
            Found::Bundle => files2.push((path, nested(&full)?)),
            Found::File => {
                let listed = treatment(rules, &path);
                let bytes = if matches!(listed, Some(Treatment::Hash | Treatment::Optional))
                    || matches!(sealed, Some(Treatment::Hash | Treatment::Optional))
                {
                    std::fs::read(&full).map_err(unreadable)?
                } else {
                    Vec::new()
                };
                match listed {
                    Some(Treatment::Hash) => {
                        files.push((path.clone(), Plist::Data(sha1(&bytes).to_vec())));
                    }
                    Some(Treatment::Optional) => files.push((
                        path.clone(),
                        Plist::Dict(vec![
                            ("hash".to_string(), Plist::Data(sha1(&bytes).to_vec())),
                            ("optional".to_string(), Plist::Bool(true)),
                        ]),
                    )),
                    _ => {}
                }
                let hash2 = || ("hash2".to_string(), Plist::Data(sha256(&bytes).to_vec()));
                match sealed {
                    Some(Treatment::Hash) => files2.push((path, Plist::Dict(vec![hash2()]))),
                    Some(Treatment::Optional) => files2.push((
                        path,
                        Plist::Dict(vec![hash2(), ("optional".to_string(), Plist::Bool(true))]),
                    )),
                    Some(Treatment::Nested) => files2.push((path, nested(&full)?)),
                    _ => {}
                }
            }
        }
    }
    let resources = Plist::Dict(vec![
        ("files".to_string(), Plist::Dict(files)),
        ("files2".to_string(), Plist::Dict(files2)),
        ("rules".to_string(), rules_plist(rules)),
        ("rules2".to_string(), rules_plist(rules2)),
    ]);
    Ok(plist::to_xml(&resources).into_bytes())
}

/// Nested code's seal: its code directory hash and designated requirement.
/// Code that isn't signed can't be sealed.
fn nested(path: &Path) -> Result<Plist, String> {
    let (cdhash, requirement) = inspect::designated(path)
        .map_err(|e| format!("{e}\nIn subcomponent: {}", path.display()))?;
    Ok(Plist::Dict(vec![
        ("cdhash".to_string(), Plist::Data(cdhash)),
        ("requirement".to_string(), Plist::String(requirement)),
    ]))
}

/// Every file and symlink under `dir`, as paths relative to the bundle,
/// with the bundles nested-code rules seal whole in place of their files.
/// The bundle's own `_CodeSignature` is left out.
fn collect(
    dir: &Path,
    prefix: &str,
    rules2: &[Rule],
    out: &mut Vec<(String, Found)>,
) -> Result<(), String> {
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
            if path == "_CodeSignature" {
                continue;
            }
            if name.contains('.') && treatment(rules2, &path) == Some(Treatment::Nested) {
                out.push((path, Found::Bundle));
            } else {
                collect(&entry.path(), &format!("{path}/"), rules2, out)?;
            }
        } else if kind.is_symlink() {
            out.push((path, Found::Symlink));
        } else {
            out.push((path, Found::File));
        }
    }
    Ok(())
}
