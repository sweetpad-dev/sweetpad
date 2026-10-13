//! The stand-in against what Xcode's momc made of the models in
//! `fixtures/momc` (`regen.py` records it).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::archive::{Plist, read};
use super::*;
use crate::cli::testdir::TempDir;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/momc")
}

fn models() -> Vec<PathBuf> {
    let mut models: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e == "xcdatamodel" || e == "xcdatamodeld")
        })
        .collect();
    models.sort();
    models
}

fn expected(model: &Path) -> PathBuf {
    fixtures().join("expected").join(model.file_name().unwrap())
}

fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The files under `dir`, by path inside it.
fn files_in(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let name = path.strip_prefix(dir).unwrap().display().to_string();
                out.insert(name, std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn first_difference(actual: &str, expected: &str) -> String {
    for (i, (a, e)) in actual.lines().zip(expected.lines()).enumerate() {
        if a != e {
            return format!("line {}:\n  got:      {a:?}\n  expected: {e:?}", i + 1);
        }
    }
    format!(
        "one is a prefix of the other ({} vs {} lines)",
        actual.lines().count(),
        expected.lines().count()
    )
}

/// A keyed archive as a tree that doesn't depend on the order objects
/// are written in: each dictionary's entries and each set's members
/// sorted, every object numbered by when a walk in that order meets it.
fn canonical(data: &[u8]) -> String {
    let Plist::Dict(top) = read::parse(data) else {
        panic!("an archive is a dictionary");
    };
    let field = |name: &str| top.iter().find(|(k, _)| k == name).unwrap().1.clone();
    let Plist::Array(objects) = field("$objects") else {
        panic!("no $objects");
    };
    let Plist::Dict(roots) = field("$top") else {
        panic!("no $top");
    };
    let mut out = String::new();
    let mut seen = BTreeMap::new();
    walk(&objects, &roots[0].1, 0, "", &mut seen, &mut out);
    out
}

fn get<'a>(dict: &'a [(String, Plist)], key: &str) -> Option<&'a Plist> {
    dict.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn resolve<'a>(objects: &'a [Plist], value: &'a Plist) -> &'a Plist {
    match value {
        Plist::Uid(n) => &objects[usize::try_from(*n).unwrap()],
        other => other,
    }
}

/// What an object sorts by in a set or dictionary: an entity or
/// property's name, or a value's text.
fn sort_name(objects: &[Plist], value: &Plist) -> String {
    match resolve(objects, value) {
        Plist::Dict(d) => ["NSEntityName", "NSPropertyName"]
            .iter()
            .find_map(|k| get(d, k))
            .map(|name| sort_name(objects, name))
            .unwrap_or_default(),
        Plist::String(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

fn scalar(value: &Plist) -> String {
    match value {
        Plist::Data(bytes) => bytes.iter().fold("data ".to_string(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        }),
        other => format!("{other:?}"),
    }
}

fn walk(
    objects: &[Plist],
    value: &Plist,
    indent: usize,
    label: &str,
    seen: &mut BTreeMap<u64, usize>,
    out: &mut String,
) {
    let pad = "  ".repeat(indent);
    let Plist::Uid(uid) = value else {
        let _ = writeln!(out, "{pad}{label}{}", scalar(value));
        return;
    };
    if *uid == 0 {
        let _ = writeln!(out, "{pad}{label}nil");
        return;
    }
    let Plist::Dict(object) = resolve(objects, value) else {
        let _ = writeln!(out, "{pad}{label}{}", scalar(resolve(objects, value)));
        return;
    };
    let Some(Plist::Dict(class)) = get(object, "$class").map(|c| resolve(objects, c)) else {
        panic!("an object with no class");
    };
    let Some(Plist::Array(chain)) = get(class, "$classes") else {
        panic!("a class with no chain");
    };
    let chain: Vec<String> = chain.iter().map(scalar).collect();
    if let Some(n) = seen.get(uid) {
        let _ = writeln!(out, "{pad}{label}<{} @{n}>", chain[0]);
        return;
    }
    let n = seen.len();
    seen.insert(*uid, n);
    let _ = writeln!(out, "{pad}{label}{} @{n}", chain.join(" > "));
    if let (Some(Plist::Array(keys)), Some(Plist::Array(values))) =
        (get(object, "NS.keys"), get(object, "NS.objects"))
    {
        let mut pairs: Vec<(String, &Plist)> = keys
            .iter()
            .map(|k| sort_name(objects, k))
            .zip(values)
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        for (key, value) in pairs {
            walk(objects, value, indent + 1, &format!("[{key}] "), seen, out);
        }
        return;
    }
    if let Some(Plist::Array(items)) = get(object, "NS.objects") {
        let mut items: Vec<&Plist> = items.iter().collect();
        if chain[0].contains("Set") {
            items.sort_by_key(|i| sort_name(objects, i));
        }
        for (i, item) in items.into_iter().enumerate() {
            walk(objects, item, indent + 1, &format!("[{i}] "), seen, out);
        }
        return;
    }
    let mut fields: Vec<&(String, Plist)> = object.iter().filter(|(k, _)| k != "$class").collect();
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, value) in fields {
        walk(objects, value, indent + 1, &format!("{key}: "), seen, out);
    }
}

/// A plain property list with its dictionaries sorted.
fn sorted(value: Plist) -> Plist {
    match value {
        Plist::Dict(entries) => {
            let mut entries: Vec<(String, Plist)> =
                entries.into_iter().map(|(k, v)| (k, sorted(v))).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Plist::Dict(entries)
        }
        Plist::Array(items) => Plist::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

fn args(model: &Path, destination: &Path, generate: bool, dry_run: bool) -> Args {
    Args {
        generate,
        dry_run,
        quiet: Quiet::default(),
        source: model.to_path_buf(),
        destination: destination.to_path_buf(),
    }
}

#[test]
fn each_model_compiles_as_xcode_compiles_it() {
    for model in models() {
        let name = model.file_name().unwrap().to_string_lossy().into_owned();
        let out = TempDir::new("sweetpad-momc-compile");
        let mut destination = out.to_path_buf().into_os_string();
        destination.push("/");
        let (_, log) = compile_model(&args(&model, Path::new(&destination), false, false)).unwrap();
        let log: String = log
            .iter()
            .map(|l| l.replace(&fixtures().display().to_string(), "$(DIR)") + "\n")
            .collect();
        let want = read_text(&expected(&model).join("compile.log"));
        assert!(
            log == want,
            "{name} compile log, {}",
            first_difference(&log, &want)
        );

        let ours = files_in(&out);
        let xcode = files_in(&expected(&model).join("out"));
        assert_eq!(
            ours.keys().collect::<Vec<_>>(),
            xcode.keys().collect::<Vec<_>>(),
            "{name}'s compiled files"
        );
        for (file, data) in &ours {
            if Path::new(file).extension().is_some_and(|e| e == "mom") {
                let (got, want) = (canonical(data), canonical(&xcode[file]));
                assert!(
                    got == want,
                    "{name} {file}, {}",
                    first_difference(&got, &want)
                );
            } else {
                assert_eq!(
                    sorted(read::parse(data)),
                    sorted(read::parse(&xcode[file])),
                    "{name} {file}"
                );
            }
        }
    }
}

#[test]
fn each_model_generates_the_classes_xcode_generates() {
    let header = codegen::Header {
        author: "$(AUTHOR)".to_string(),
        date: "$(DATE)".to_string(),
    };
    for model in models() {
        let name = model.file_name().unwrap().to_string_lossy().into_owned();
        let out = TempDir::new("sweetpad-momc-generate");
        let listed = generate(&args(&model, &out, true, true), &header).unwrap();
        let listed: String = listed
            .iter()
            .map(|p| p.strip_prefix(&*out).unwrap().display().to_string() + "\n")
            .collect();
        assert_eq!(
            listed,
            read_text(&expected(&model).join("generate.dry-run")),
            "{name} dry run"
        );
        assert!(
            files_in(&out).is_empty(),
            "{name}: a dry run writes nothing"
        );

        generate(&args(&model, &out, true, false), &header).unwrap();
        for (file, text) in files_in(&out) {
            let got = String::from_utf8(text).unwrap();
            let want = read_text(&expected(&model).join("generated").join(&file));
            assert!(
                got == want,
                "{name} {file}, {}",
                first_difference(&got, &want)
            );
        }
    }
}

#[test]
fn the_arguments_swift_build_passes_parse() {
    let args = parse_args(&[
        "--action".to_string(),
        "generate".to_string(),
        "--swift-version".to_string(),
        "5.0".to_string(),
        "--sdkroot=/sdk".to_string(),
        "--iphonesimulator-deployment-target".to_string(),
        "17.0".to_string(),
        "--module".to_string(),
        "App".to_string(),
        "--dry-run".to_string(),
        "--no-inverse-relationship-warnings".to_string(),
        "Model.xcdatamodeld".to_string(),
        "/out".to_string(),
    ])
    .unwrap();
    assert!(args.generate && args.dry_run && args.quiet.inverse && !args.quiet.all);
    assert_eq!(args.source, Path::new("Model.xcdatamodeld"));
    assert_eq!(args.destination, Path::new("/out"));
    assert!(parse_args(&["--frobnicate".to_string()]).is_err());
}

#[test]
fn a_model_that_needs_predicates_is_refused_by_name() {
    let dir = TempDir::new("sweetpad-momc-refuse");
    let model = dir.join("Fetch.xcdatamodel");
    std::fs::create_dir_all(&model).unwrap();
    std::fs::write(
        model.join("contents"),
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<model type="com.apple.IDECoreDataModeler.DataModel" sourceLanguage="Swift">
    <entity name="A" syncable="YES"/>
    <fetchRequest name="All" entity="A" predicateString="TRUEPREDICATE"/>
</model>"#,
    )
    .unwrap();
    let error = compile_model(&args(&model, &dir, false, false)).unwrap_err();
    assert!(error.contains("fetch request templates"), "{error}");
}
