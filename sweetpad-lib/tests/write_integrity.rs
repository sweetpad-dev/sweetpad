//! Every verb that writes a project document, run against the committed
//! fixtures in both formats, with the document checked after each edit the
//! way Xcode reads it.
//!
//! A `project.pbxproj` edit may not leave a name pointing at no object, lose
//! an object the project reached unless the verb says it orphans one, delete
//! an object of a kind the verb does not own, or list a node in a second group
//! (Xcode 27.2 refuses to open that). A `project.xcproj` edit may not leave a
//! configuration's xcconfig, a target's product or the products group naming
//! nothing (Xcode 27.0 and 27.2 refuse that as an invalid reference), and a
//! move may not change where any node resolves. An edit that refuses may not
//! change the document at all, and an edit followed by its inverse restores
//! it. The checks read the documents directly rather than through the crate's
//! own readers, so they judge the writers independently.
//!
//! `WRITE_LIVE_ORACLE=1` adds the Xcode half: each edited project is written
//! to a copy of its bundle, and `xcodebuild -list` has to open it and list the
//! targets and configurations the unedited copy lists. A project that names
//! Swift packages is skipped: listing it means resolving them, which needs
//! the network and leaves a DerivedData folder behind. Then one small app is
//! moved around in both formats and built. It runs the selected Xcode, which
//! reads both formats from 27.0 on.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::TempDir;
use sweetpad_lib::membership::Phase;
use sweetpad_lib::pbxproj::{self, Dict};
use sweetpad_lib::spm::RequirementSpec;
use sweetpad_lib::stored_settings::{Assignment, Op, Scope};
use sweetpad_lib::tree::{AddGroupOutcome, AddRefOutcome, MoveOutcome};
use sweetpad_lib::{
    membership_pbxproj, membership_xcproj, settings_pbxproj, settings_xcproj, spm_pbxproj,
    spm_xcproj, sync_pbxproj, sync_xcproj, tree_pbxproj, tree_xcproj, xcproj,
};

const LIVE: &str = "WRITE_LIVE_ORACLE";

const PROBE_URL: &str = "https://github.com/sweetpad-dev/sweetpad-probe";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Every file called `name` under the fixtures, one per distinct content, so
/// a project captured under several Xcode versions is edited once.
fn documents(name: &str) -> Vec<PathBuf> {
    fn walk(dir: &Path, name: &str, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, name, out);
            } else if path.file_name().is_some_and(|n| n == name) {
                out.push(path);
            }
        }
    }
    let mut all = Vec::new();
    walk(&fixtures(), name, &mut all);
    let mut seen = BTreeSet::new();
    all.into_iter()
        .filter(|p| seen.insert(std::fs::read(p).unwrap_or_default()))
        .collect()
}

/// Up to `n` items spread evenly over `items`, first and last included.
fn spread<T: Clone>(items: &[T], n: usize) -> Vec<T> {
    if items.len() <= n {
        return items.to_vec();
    }
    (0..n)
        .map(|i| items[i * (items.len() - 1) / (n - 1).max(1)].clone())
        .collect()
}

fn label_of(path: &Path) -> String {
    path.strip_prefix(fixtures())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Failures across a whole run, reported together.
#[derive(Default)]
struct Report {
    failures: Vec<String>,
    edits: usize,
}

impl Report {
    fn finish(self, what: &str) {
        assert!(self.edits > 0, "no {what} edits ran");
        assert!(
            self.failures.is_empty(),
            "{} of {} {what} edits broke the document:\n{}",
            self.failures.len(),
            self.edits,
            self.failures.join("\n")
        );
    }
}

/// Edited documents worth opening in Xcode: what made each, and its text.
type Edits = Vec<(String, String)>;

/// A bundle, the name of its document, and the edits to open it with.
type Edited = (PathBuf, &'static str, Edits);

/// Run `exercise` on every fixture document called `name`, a few at a time,
/// and gather what each found.
fn exercise_all(
    name: &'static str,
    exercise: fn(&Path, &mut Report) -> Edits,
) -> (Report, Vec<Edited>) {
    let paths = documents(name);
    let next = AtomicUsize::new(0);
    let done = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                while let Some(path) = paths.get(next.fetch_add(1, Ordering::SeqCst)) {
                    let mut report = Report::default();
                    let edits = exercise(path, &mut report);
                    let bundle = path.parent().unwrap().to_path_buf();
                    done.lock().unwrap().push((bundle, report, edits));
                }
            });
        }
    });
    let mut done = done.into_inner().unwrap();
    done.sort_by(|a, b| a.0.cmp(&b.0));
    let mut report = Report::default();
    let mut live = Vec::new();
    for (bundle, found, edits) in done {
        report.edits += found.edits;
        report.failures.extend(found.failures);
        live.push((bundle, name, edits));
    }
    (report, live)
}

/// The keys under which a pbxproj object always holds object ids.
const ID_KEYS: [&str; 27] = [
    "baseConfigurationReference",
    "baseConfigurationReferenceAnchor",
    "buildConfigurationList",
    "buildConfigurations",
    "buildPhases",
    "buildRules",
    "children",
    "containerPortal",
    "dependencies",
    "exceptions",
    "fileRef",
    "files",
    "fileSystemSynchronizedGroups",
    "mainGroup",
    "package",
    "packageProductDependencies",
    "packageReferences",
    "productRef",
    "productRefGroup",
    "productReference",
    "ProductGroup",
    "ProjectRef",
    "projectReferences",
    "remoteRef",
    "target",
    "targetProxy",
    "targets",
];

/// A pbxproj document and the name its writer titles it with.
#[derive(Clone)]
struct Pbx {
    root: pbxproj::Value,
    name: String,
}

impl Pbx {
    fn objects(&self) -> &Dict {
        objects(&self.root)
    }

    fn text(&self) -> String {
        sweetpad_lib::pbxproj_writer::serialize(&self.root, &self.name)
    }
}

fn objects(root: &pbxproj::Value) -> &Dict {
    root.get("objects")
        .and_then(pbxproj::Value::as_dict)
        .expect("a pbxproj has objects")
}

fn objects_mut(root: &mut pbxproj::Value) -> &mut Dict {
    root.get_mut("objects")
        .and_then(pbxproj::Value::as_dict_mut)
        .expect("a pbxproj has objects")
}

fn isa(object: &pbxproj::Value) -> &str {
    object
        .get("isa")
        .and_then(pbxproj::Value::as_str)
        .unwrap_or_default()
}

/// Every string an object holds, with the key it sits under at any depth.
/// Build settings hold values, not names, and `remoteGlobalIDString` names an
/// object in another project as often as in this one.
fn strings(value: &pbxproj::Value, key: &str, out: &mut Vec<(String, String)>) {
    match value {
        pbxproj::Value::String(s) => out.push((key.to_string(), s.clone())),
        pbxproj::Value::Array(items) => {
            for item in items {
                strings(item, key, out);
            }
        }
        pbxproj::Value::Dict(dict) => {
            for (k, v) in dict.iter() {
                if !matches!(k.as_str(), "buildSettings" | "remoteGlobalIDString" | "isa") {
                    strings(v, k, out);
                }
            }
        }
    }
}

fn object_strings(object: &pbxproj::Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    strings(object, "", &mut out);
    out
}

/// The names that name no object: a string under a key that always holds ids,
/// or one that named an object the edit started with.
fn dangling(root: &pbxproj::Value, known: &BTreeMap<String, String>) -> BTreeSet<String> {
    let objects = objects(root);
    let mut out = BTreeSet::new();
    for (owner, object) in objects.iter() {
        for (key, value) in object_strings(object) {
            let is_id = ID_KEYS.contains(&key.as_str()) || known.contains_key(&value);
            if is_id && !objects.contains_key(&value) {
                out.insert(format!("{owner}.{key} = {value}"));
            }
        }
    }
    out
}

/// Every object the project reaches from its root object.
fn reachable(root: &pbxproj::Value) -> BTreeSet<String> {
    let objects = objects(root);
    let mut seen = BTreeSet::new();
    let mut stack: Vec<String> = root
        .get("rootObject")
        .and_then(pbxproj::Value::as_str)
        .map(str::to_string)
        .into_iter()
        .collect();
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(object) = objects.get(&id) {
            for (_, value) in object_strings(object) {
                if objects.contains_key(&value) && !seen.contains(&value) {
                    stack.push(value);
                }
            }
        }
    }
    seen
}

/// Every node that more than one group lists.
fn listed_twice(root: &pbxproj::Value) -> BTreeSet<String> {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for object in objects(root).values() {
        if !matches!(
            isa(object),
            "PBXGroup" | "PBXVariantGroup" | "XCVersionGroup"
        ) {
            continue;
        }
        let children = object
            .get("children")
            .and_then(pbxproj::Value::as_array)
            .unwrap_or_default();
        for child in children.iter().filter_map(pbxproj::Value::as_str) {
            *count.entry(child.to_string()).or_default() += 1;
        }
    }
    count
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(id, _)| id)
        .collect()
}

/// What a pbxproj edit may take out of the project.
#[derive(Clone, Copy)]
struct Allowed {
    /// The kinds of object the verb deletes.
    delete: &'static [&'static str],
    /// Whether the verb leaves objects nothing reaches, as `--orphan-children`
    /// and `group detach` say they do.
    orphan: bool,
    /// Whether the verb leaves build files naming the file it deleted, as
    /// `fileref remove --dangling` says it does.
    dangling_build_files: bool,
}

impl Allowed {
    const fn deleting(delete: &'static [&'static str]) -> Self {
        Allowed {
            delete,
            orphan: false,
            dangling_build_files: false,
        }
    }
}

const NOTHING: Allowed = Allowed::deleting(&[]);

const TREE_DELETES: &[&str] = &[
    "PBXFileReference",
    "PBXGroup",
    "PBXVariantGroup",
    "XCVersionGroup",
];

const MEMBERSHIP_DELETES: &[&str] = &[
    "PBXBuildFile",
    "PBXFileReference",
    "PBXGroup",
    "PBXVariantGroup",
    "XCVersionGroup",
];

/// What the checks read off the document an edit starts from.
struct Facts {
    /// Every object's kind, by id.
    known: BTreeMap<String, String>,
    dangling: BTreeSet<String>,
    reachable: BTreeSet<String>,
    twice: BTreeSet<String>,
}

impl Facts {
    fn of(doc: &Pbx) -> Self {
        let known: BTreeMap<String, String> = doc
            .objects()
            .iter()
            .map(|(id, o)| (id.clone(), isa(o).to_string()))
            .collect();
        Facts {
            dangling: dangling(&doc.root, &known),
            reachable: reachable(&doc.root),
            twice: listed_twice(&doc.root),
            known,
        }
    }
}

/// Check a pbxproj edit's result against the facts of the document it
/// started from.
fn check_pbx(facts: &Facts, after: &Pbx, allowed: Allowed) -> Result<(), String> {
    let mut problems = Vec::new();
    let new_dangling: Vec<String> = dangling(&after.root, &facts.known)
        .difference(&facts.dangling)
        .filter(|name| {
            let owner = name.split('.').next().unwrap_or_default();
            let build_file = after.objects().get(owner).map(isa) == Some("PBXBuildFile");
            !(allowed.dangling_build_files && build_file && name.contains(".fileRef = "))
        })
        .cloned()
        .collect();
    if !new_dangling.is_empty() {
        problems.push(format!("names nothing: {}", new_dangling.join(", ")));
    }
    for (id, kind) in &facts.known {
        if !after.objects().contains_key(id) && !allowed.delete.contains(&kind.as_str()) {
            problems.push(format!("deleted {kind} {id}"));
        }
    }
    if !allowed.orphan {
        let still = reachable(&after.root);
        let lost: Vec<&String> = facts
            .reachable
            .iter()
            .filter(|id| after.objects().contains_key(id.as_str()) && !still.contains(*id))
            .collect();
        if !lost.is_empty() {
            problems.push(format!("no longer reached: {lost:?}"));
        }
    }
    let twice: Vec<String> = listed_twice(&after.root)
        .difference(&facts.twice)
        .cloned()
        .collect();
    if !twice.is_empty() {
        problems.push(format!("listed in two groups: {}", twice.join(", ")));
    }
    let text = after.text();
    match pbxproj::parse(&text) {
        Ok(reparsed) => {
            if sweetpad_lib::pbxproj_writer::serialize(&reparsed, &after.name) != text {
                problems.push("does not read back as written".to_string());
            }
        }
        Err(e) => problems.push(format!("does not parse back: {e}")),
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// The path every file reference resolves to, by id.
fn pbx_resolved(root: &pbxproj::Value) -> BTreeMap<String, String> {
    tree_pbxproj::list_filerefs(root)
        .unwrap_or_default()
        .into_iter()
        .map(|r| (r.address, r.resolved))
        .collect()
}

/// The document with every group's children in one order, for comparing two
/// that list the same nodes in a different order.
fn sorted_children(root: &pbxproj::Value) -> pbxproj::Value {
    let mut root = root.clone();
    let ids: Vec<String> = objects(&root).keys().cloned().collect();
    let objects = objects_mut(&mut root);
    for id in ids {
        if let Some(children) = objects
            .get_mut(&id)
            .and_then(|o| o.get_mut("children"))
            .and_then(pbxproj::Value::as_array_mut)
        {
            children.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        }
    }
    root
}

/// The document's text with every empty `packageReferences`,
/// `packageProductDependencies` and `dependencies` list left out.
fn without_empty_package_lists(doc: &Pbx) -> String {
    let mut root = doc.root.clone();
    let objects = objects_mut(&mut root);
    let ids: Vec<String> = objects.keys().cloned().collect();
    for id in ids {
        let Some(object) = objects.get_mut(&id).and_then(pbxproj::Value::as_dict_mut) else {
            continue;
        };
        for key in [
            "packageReferences",
            "packageProductDependencies",
            "dependencies",
        ] {
            if object
                .get(key)
                .and_then(pbxproj::Value::as_array)
                .is_some_and(<[pbxproj::Value]>::is_empty)
            {
                object.remove(key);
            }
        }
    }
    sweetpad_lib::pbxproj_writer::serialize(&root, &doc.name)
}

/// One pbxproj fixture's run: a fresh copy of the document for every edit.
struct PbxRun<'a> {
    base: Pbx,
    facts: Facts,
    label: String,
    report: &'a mut Report,
    edits: Edits,
}

impl PbxRun<'_> {
    /// Run `edit` on a copy of `start` (the fixture without one), check the
    /// result, and hand it back when it applied. A refusal must leave the
    /// copy as it was.
    fn trial<T>(
        &mut self,
        what: &str,
        allowed: Allowed,
        start: Option<&Pbx>,
        edit: impl FnOnce(&mut pbxproj::Value) -> Result<T, String>,
    ) -> Option<(Pbx, T)> {
        self.report.edits += 1;
        let before = start.unwrap_or(&self.base);
        let mut after = before.clone();
        let Ok(outcome) = edit(&mut after.root) else {
            if after.root != before.root {
                self.fail(what, "a refusal changed the document");
            }
            return None;
        };
        let computed;
        let facts = if start.is_some() {
            computed = Facts::of(before);
            &computed
        } else {
            &self.facts
        };
        if let Err(problem) = check_pbx(facts, &after, allowed) {
            self.fail(what, &problem);
        }
        Some((after, outcome))
    }

    fn fail(&mut self, what: &str, problem: &str) {
        self.report
            .failures
            .push(format!("{}: {what}: {problem}", self.label));
    }

    /// Require `doc` to read as the fixture does.
    fn restored(&mut self, what: &str, doc: &Pbx) {
        if doc.text() != self.base.text() {
            self.fail(what, "the document is not restored");
        }
    }

    fn keep(&mut self, what: &str, doc: &Pbx) {
        if self.edits.len() < 6 {
            self.edits.push((what.to_string(), doc.text()));
        }
    }

    /// Merge two edits of the fixture the way `pbxproj resolve` does. A merge
    /// that reports no conflict has to leave the project as whole as each
    /// edit left it: nothing either side names may go missing.
    fn merged(&mut self, what: &str, ours: &Pbx, theirs: &Pbx) {
        self.report.edits += 1;
        let merge =
            sweetpad_lib::pbxproj_merge::merge(Some(&self.base.root), &ours.root, &theirs.root);
        if !merge.is_clean() {
            return;
        }
        let merged = Pbx {
            root: merge.value,
            name: self.base.name.clone(),
        };
        if let Err(problem) = check_pbx(&self.facts, &merged, ANYTHING) {
            self.fail(what, &format!("a clean merge {problem}"));
        }
    }
}

/// What any edit may take out, for a merge of two of them.
const ANYTHING: Allowed = Allowed {
    delete: &[
        "PBXBuildFile",
        "PBXFileReference",
        "PBXGroup",
        "PBXVariantGroup",
        "XCVersionGroup",
        "PBXFileSystemSynchronizedRootGroup",
        "PBXFileSystemSynchronizedBuildFileExceptionSet",
        "PBXFileSystemSynchronizedGroupBuildPhaseMembershipExceptionSet",
    ],
    orphan: true,
    dangling_build_files: false,
};

/// The kept edits merged in pairs.
fn pbx_merges(run: &mut PbxRun<'_>) {
    let edits: Vec<(String, Pbx)> = run
        .edits
        .iter()
        .filter_map(|(what, text)| {
            let root = pbxproj::parse(text).ok()?;
            let name = run.base.name.clone();
            Some((what.clone(), Pbx { root, name }))
        })
        .collect();
    for pair in edits.windows(2) {
        let [(first, ours), (second, theirs)] = pair else {
            continue;
        };
        run.merged(&format!("{first}, merged with {second}"), ours, theirs);
    }
}

/// The nodes a pbxproj run edits, and what they are before any edit.
struct PbxSample {
    files: Vec<String>,
    groups: Vec<String>,
    main_group: Option<String>,
    /// The group Xcode keeps listing each node.
    home: BTreeMap<String, String>,
    resolved: BTreeMap<String, String>,
}

fn pbx_sample(base: &Pbx) -> PbxSample {
    let objects = base.objects();
    let of_kind = |kind: &str| -> Vec<String> {
        objects
            .iter()
            .filter(|(_, o)| isa(o) == kind)
            .map(|(id, _)| id.clone())
            .collect()
    };
    // The references the rest of the project names come first, since those
    // are the ones a careless delete or move breaks.
    let named: BTreeSet<String> = objects
        .values()
        .flat_map(|o| {
            ["baseConfigurationReference", "productReference"]
                .into_iter()
                .filter_map(|k| o.get(k).and_then(pbxproj::Value::as_str))
                .map(str::to_string)
        })
        .collect();
    let mut files: Vec<String> = named.into_iter().take(4).collect();
    files.extend(spread(&of_kind("PBXFileReference"), 5));
    let home = tree_pbxproj::list_filerefs(&base.root)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|r| Some((r.address, r.parent?)))
        .chain(
            tree_pbxproj::list_groups(&base.root)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|g| Some((g.address, g.parent?))),
        )
        .collect();
    PbxSample {
        files,
        groups: spread(&of_kind("PBXGroup"), 5),
        main_group: objects
            .values()
            .find(|o| isa(o) == "PBXProject")
            .and_then(|p| p.get("mainGroup"))
            .and_then(pbxproj::Value::as_str)
            .map(str::to_string),
        home,
        resolved: pbx_resolved(&base.root),
    }
}

/// Every verb against one pbxproj document.
fn exercise_pbx(path: &Path, report: &mut Report) -> Edits {
    let text = std::fs::read_to_string(path).unwrap();
    let Ok(root) = pbxproj::parse(&text) else {
        return Vec::new();
    };
    let name = path
        .parent()
        .and_then(Path::file_stem)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = Pbx { root, name };
    let mut run = PbxRun {
        facts: Facts::of(&base),
        base,
        label: label_of(path),
        report,
        edits: Vec::new(),
    };
    let sample = pbx_sample(&run.base);
    for node in sample.files.iter().chain(&sample.groups) {
        pbx_remove(&mut run, node, sample.groups.contains(node));
        pbx_moves(&mut run, node, &sample);
        pbx_links(&mut run, node, &sample);
    }
    pbx_adds(&mut run, &sample);
    pbx_membership(&mut run);
    pbx_folders(&mut run);
    pbx_settings_and_packages(&mut run);
    pbx_merges(&mut run);
    run.edits
}

fn pbx_remove(run: &mut PbxRun<'_>, node: &str, is_group: bool) {
    for force in [false, true] {
        // `--orphan-children` leaves a group's children unreached, and
        // `--dangling` leaves the build files of a file.
        let allowed = Allowed {
            delete: TREE_DELETES,
            orphan: is_group && force,
            dangling_build_files: !is_group && force,
        };
        let what = format!("remove {node} (force {force})");
        let applied = run.trial(&what, allowed, None, |root| {
            if is_group {
                tree_pbxproj::remove_group(root, node, force)
            } else {
                tree_pbxproj::remove_fileref(root, node, force)
            }
        });
        if let Some((doc, _)) = applied
            && !force
        {
            run.keep(&what, &doc);
        }
    }
}

/// Move a node to the navigator root and to two groups, and each time back.
fn pbx_moves(run: &mut PbxRun<'_>, node: &str, sample: &PbxSample) {
    let destinations = std::iter::once(None).chain(
        sample
            .groups
            .iter()
            .filter(|g| *g != node)
            .take(2)
            .map(|g| Some(g.as_str())),
    );
    let anchor = |doc: &Pbx| {
        doc.objects()
            .get(node)
            .and_then(|o| o.get("sourceTree"))
            .and_then(pbxproj::Value::as_str)
            .map(str::to_string)
    };
    for to in destinations {
        let what = format!("move {node} to {to:?}");
        let Some((moved, outcome)) = run.trial(&what, NOTHING, None, |root| {
            tree_pbxproj::move_node(root, node, to)
        }) else {
            continue;
        };
        if pbx_resolved(&moved.root) != sample.resolved {
            run.fail(&what, "a file resolves somewhere else after the move");
        }
        let (MoveOutcome::Moved { .. }, Some(back)) = (outcome, sample.home.get(node)) else {
            continue;
        };
        run.keep(&what, &moved);
        let what = format!("{what} and back to {back}");
        let Some((returned, _)) = run.trial(&what, NOTHING, Some(&moved), |root| {
            tree_pbxproj::move_node(root, node, Some(back))
        }) else {
            continue;
        };
        if pbx_resolved(&returned.root) != sample.resolved {
            run.fail(&what, "a file resolves somewhere else after the move back");
        }
        // A path a move could only anchor at the project stays anchored
        // there, so only a node that kept its anchor comes back as it was.
        let kept = anchor(&moved) == anchor(&run.base) && anchor(&returned) == anchor(&run.base);
        if kept && sorted_children(&returned.root) != sorted_children(&run.base.root) {
            run.fail(&what, "the move back does not restore the document");
        }
    }
}

/// A node another group lists stays listed once, and a detach followed by an
/// attach puts it back.
fn pbx_links(run: &mut PbxRun<'_>, node: &str, sample: &PbxSample) {
    let home = sample.home.get(node);
    if let Some(other) = sample
        .groups
        .iter()
        .find(|g| Some(*g) != home && *g != node)
    {
        let what = format!("attach {node} to {other}");
        run.trial(&what, NOTHING, None, |root| {
            tree_pbxproj::attach(root, node, other)
        });
    }
    let Some(parent) = home else {
        return;
    };
    let unlisting = Allowed {
        orphan: true,
        ..NOTHING
    };
    let what = format!("detach {node} from {parent} and attach it back");
    if let Some((detached, _)) = run.trial(&what, unlisting, None, |root| {
        tree_pbxproj::detach(root, node, parent)
    }) && let Some((back, _)) = run.trial(&what, NOTHING, Some(&detached), |root| {
        tree_pbxproj::attach(root, node, parent)
    }) && sorted_children(&back.root) != sorted_children(&run.base.root)
    {
        run.fail(&what, "the document is not restored");
    }
}

/// An add and the remove of what it added leave the document as it was.
fn pbx_adds(run: &mut PbxRun<'_>, sample: &PbxSample) {
    let removing = Allowed::deleting(TREE_DELETES);
    for group in sample.groups.iter().chain(&sample.main_group) {
        let what = format!("add a file under {group} and remove it");
        if let Some((added, AddRefOutcome::Created { address, .. })) =
            run.trial(&what, NOTHING, None, |root| {
                tree_pbxproj::add_fileref(
                    root,
                    "SweetpadProbe.swift",
                    Some("sourcecode.swift"),
                    "<group>",
                    Some(group),
                )
            })
            && let Some((removed, _)) = run.trial(&what, removing, Some(&added), |root| {
                tree_pbxproj::remove_fileref(root, &address, false)
            })
        {
            run.restored(&what, &removed);
        }
        let what = format!("add a group under {group} and remove it");
        if let Some((added, AddGroupOutcome::Created { address, .. })) =
            run.trial(&what, NOTHING, None, |root| {
                tree_pbxproj::add_group(
                    root,
                    "SweetpadProbe",
                    Some(group),
                    Some("SweetpadProbe"),
                    "<group>",
                )
            })
            && let Some((removed, _)) = run.trial(&what, removing, Some(&added), |root| {
                tree_pbxproj::remove_group(root, &address, false)
            })
        {
            run.restored(&what, &removed);
        }
    }
}

/// Take files out of targets, and give one a file another target builds and
/// take it back.
fn pbx_membership(run: &mut PbxRun<'_>) {
    let removing = Allowed::deleting(MEMBERSHIP_DELETES);
    let targets = settings_pbxproj::target_names(&run.base.root);
    let members_of =
        |t: &str| membership_pbxproj::classic_members(&run.base.root, t).unwrap_or_default();
    let all: Vec<_> = targets.iter().map(|t| (t.clone(), members_of(t))).collect();
    for (target, members) in spread(&all, 3) {
        for entry in spread(&members, 3) {
            let what = format!("membership remove {} from {target}", entry.path);
            let paths = std::slice::from_ref(&entry.path);
            if let Some((doc, _)) = run.trial(&what, removing, None, |root| {
                membership_pbxproj::remove_membership(root, &target, paths)
            }) {
                run.keep(&what, &doc);
            }
        }
        let theirs = all
            .iter()
            .filter(|(t, _)| *t != target)
            .flat_map(|(_, m)| m)
            .find(|e| e.phase == Phase::Sources && !members.iter().any(|m| m.path == e.path));
        let Some(path) = theirs.map(|e| e.path.clone()) else {
            continue;
        };
        let Ok(Some(id)) = tree_pbxproj::fileref_for_path(&run.base.root, &path) else {
            continue;
        };
        let what = format!("membership add {path} to {target} and remove it");
        if let Some((added, additions)) = run.trial(&what, NOTHING, None, |root| {
            membership_pbxproj::add_membership_by_ids(
                root,
                &target,
                std::slice::from_ref(&id),
                &Phase::Sources,
            )
        }) && additions.iter().all(|a| !a.already_member)
        {
            if let Some((removed, _)) = run.trial(&what, removing, Some(&added), |root| {
                membership_pbxproj::remove_membership(root, &target, std::slice::from_ref(&path))
            }) {
                run.restored(&what, &removed);
            }
            // One branch stops building the file where the other starts, and
            // the first may take the reference with it.
            let builders = all
                .iter()
                .filter(|(t, m)| *t != target && m.iter().any(|e| e.path == path));
            for (builder, _) in builders.take(1) {
                let what = format!(
                    "membership remove {path} from {builder}, merged with its add to {target}"
                );
                if let Some((removed, _)) = run.trial(&what, removing, None, |root| {
                    membership_pbxproj::remove_membership(
                        root,
                        builder,
                        std::slice::from_ref(&path),
                    )
                }) {
                    run.merged(&what, &removed, &added);
                }
            }
        }
    }
}

/// Detach folders, and except a file in one and include it back.
fn pbx_folders(run: &mut PbxRun<'_>) {
    let detaching = Allowed::deleting(&[
        "PBXFileSystemSynchronizedRootGroup",
        "PBXFileSystemSynchronizedBuildFileExceptionSet",
        "PBXFileSystemSynchronizedGroupBuildPhaseMembershipExceptionSet",
    ]);
    let including = Allowed::deleting(&["PBXFileSystemSynchronizedBuildFileExceptionSet"]);
    for folders in sync_pbxproj::list(&run.base.root).unwrap_or_default() {
        let target = folders.target;
        for folder in folders.roots.iter().take(2) {
            let what = format!("folder remove {} from {target}", folder.dir);
            if let Some((doc, _)) = run.trial(&what, detaching, None, |root| {
                sync_pbxproj::remove_root(root, &target, &folder.dir)
            }) {
                run.keep(&what, &doc);
            }
            let probe = format!("{}/SweetpadProbe.swift", folder.dir);
            let what = format!("exclude {probe} from {target} and include it back");
            if let Some((excluded, _)) = run.trial(&what, NOTHING, None, |root| {
                sync_pbxproj::exclude(root, &target, &probe)
            }) && let Some((included, _)) =
                run.trial(&what, including, Some(&excluded), |root| {
                    sync_pbxproj::include(root, &target, &probe)
                })
            {
                run.restored(&what, &included);
            }
        }
    }
    let targets = settings_pbxproj::target_names(&run.base.root);
    let Some(target) = targets.first() else {
        return;
    };
    let what = format!("folder add SweetpadProbe to {target} and remove it");
    let removing = Allowed::deleting(&["PBXFileSystemSynchronizedRootGroup"]);
    if let Some((added, _)) = run.trial(&what, NOTHING, None, |root| {
        sync_pbxproj::add_root(root, target, "SweetpadProbe")
    }) && let Some((removed, _)) = run.trial(&what, removing, Some(&added), |root| {
        sync_pbxproj::remove_root(root, target, "SweetpadProbe")
    }) {
        run.restored(&what, &removed);
    }
}

/// A stored setting set and unset, and a package added, linked and removed.
fn pbx_settings_and_packages(run: &mut PbxRun<'_>) {
    let targets = settings_pbxproj::target_names(&run.base.root);
    let probe = Assignment {
        key: "SWEETPAD_PROBE".to_string(),
        op: Op::Assign(vec!["1".to_string()]),
    };
    let keys = ["SWEETPAD_PROBE".to_string()];
    let scopes = std::iter::once(Scope::Project).chain(targets.first().cloned().map(Scope::Target));
    for scope in scopes {
        let what = format!("settings set and unset on {scope:?}");
        if let Some((set, _)) = run.trial(&what, NOTHING, None, |root| {
            settings_pbxproj::set(root, &scope, &[], std::slice::from_ref(&probe))
        }) && let Some((unset, _)) = run.trial(&what, NOTHING, Some(&set), |root| {
            settings_pbxproj::unset(root, &scope, &[], &keys)
        }) {
            run.restored(&what, &unset);
        }
    }
    let Some(target) = targets.first() else {
        return;
    };
    let what = format!("dependency add and remove on {target}");
    let requirement = RequirementSpec::UpToNextMajor("1.0.0".to_string());
    let Some((added, reference)) = run.trial(&what, NOTHING, None, |root| {
        let reference = spm_pbxproj::add_remote_dependency(root, PROBE_URL, &requirement)?;
        spm_pbxproj::link_product(root, &reference, "SweetpadProbe", target)?;
        Ok(reference)
    }) else {
        return;
    };
    let removing = Allowed::deleting(&[
        "XCRemoteSwiftPackageReference",
        "XCSwiftPackageProductDependency",
        "PBXBuildFile",
        "PBXTargetDependency",
    ]);
    // The add gives the project and the target a package list, and the
    // remove leaves it empty, as Xcode leaves it.
    if let Some((removed, ())) = run.trial(&what, removing, Some(&added), |root| {
        spm_pbxproj::remove_package(root, &reference, &[])
    }) && without_empty_package_lists(&removed) != without_empty_package_lists(&run.base)
    {
        run.fail(&what, "the document is not restored");
    }
}

#[test]
fn every_pbxproj_write_keeps_the_project_whole() {
    let (report, live) = exercise_all("project.pbxproj", exercise_pbx);
    report.finish("pbxproj");
    if std::env::var(LIVE).is_ok() {
        xcode_opens(&live);
    }
}

/// One node of a `project.xcproj` navigator, read straight off the document.
struct XcNode {
    address: String,
    id: Option<String>,
    /// Where it resolves, relative to the project directory, or in its own
    /// anchored spelling when it is not under it.
    resolved: String,
    kind: String,
}

fn xc_join(base: &str, path: &str) -> String {
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn xc_nodes(root: &xcproj::Value) -> Vec<XcNode> {
    fn walk(children: &[xcproj::Value], parent: Option<&str>, base: &str, out: &mut Vec<XcNode>) {
        for node in children {
            let text = |key: &str| node.get(key).and_then(xcproj::Value::as_str);
            let (name, path) = (text("name"), text("path"));
            let shown = name.unwrap_or_else(|| {
                path.map_or("", |p| {
                    let p = p.trim_end_matches('/');
                    p.rsplit_once('/').map_or(p, |(_, last)| last)
                })
            });
            let address = parent.map_or_else(|| shown.to_string(), |p| format!("{p}/{shown}"));
            let resolved = match path {
                None => base.to_string(),
                Some(p) if p.starts_with("<PROJECT>/") => xc_join("", &p["<PROJECT>/".len()..]),
                Some(p) if p.starts_with('<') || p.starts_with('/') => p.to_string(),
                Some(p) => xc_join(base, p),
            };
            let children = node
                .get("children")
                .and_then(xcproj::Value::as_array)
                .unwrap_or(&[]);
            walk(children, Some(&address), &resolved, out);
            out.push(XcNode {
                id: text("id").map(str::to_string),
                kind: text("kind").unwrap_or("file").to_string(),
                address,
                resolved,
            });
        }
    }
    let files = root
        .get("files")
        .and_then(xcproj::Value::as_array)
        .unwrap_or(&[]);
    let mut out = Vec::new();
    walk(files, None, "", &mut out);
    out
}

/// Every reference the document makes to a navigator node, keyed by what it
/// is, with its spelling.
fn xc_references(root: &xcproj::Value) -> BTreeMap<String, String> {
    fn configurations(
        owner: &xcproj::Value,
        list: &str,
        scope: &str,
        out: &mut BTreeMap<String, String>,
    ) {
        let entries = owner
            .get(list)
            .and_then(xcproj::Value::as_array)
            .unwrap_or(&[]);
        for entry in entries {
            let name = entry
                .get("name")
                .and_then(xcproj::Value::as_str)
                .unwrap_or_default();
            let spelling = entry.get("file").and_then(|file| {
                file.as_str()
                    .or_else(|| file.get("anchor").and_then(xcproj::Value::as_str))
            });
            if let Some(spelling) = spelling {
                out.insert(format!("{scope} {name} xcconfig"), spelling.to_string());
            }
        }
    }
    let mut out = BTreeMap::new();
    configurations(root, "configurations", "project", &mut out);
    let targets = root
        .get("targets")
        .and_then(xcproj::Value::as_array)
        .unwrap_or(&[]);
    for target in targets {
        let name = target
            .get("name")
            .and_then(xcproj::Value::as_str)
            .unwrap_or_default();
        configurations(target, "specialized-configurations", name, &mut out);
        if let Some(product) = target.get("product").and_then(xcproj::Value::as_str) {
            out.insert(format!("{name} product"), product.to_string());
        }
    }
    let products = root
        .get("products-group")
        .and_then(xcproj::Value::as_str)
        .unwrap_or("Products");
    out.insert("products group".to_string(), products.to_string());
    out
}

/// The node a reference names: `id:` and its id, or its navigator path.
fn xc_resolve<'a>(nodes: &'a [XcNode], spelling: &str) -> Option<&'a XcNode> {
    spelling
        .strip_prefix("id:")
        .and_then(|id| nodes.iter().find(|n| n.id.as_deref() == Some(id)))
        .or_else(|| nodes.iter().find(|n| n.address == spelling))
}

fn check_xc(before: &xcproj::Value, after: &xcproj::Value, moved: bool) -> Result<(), String> {
    let mut problems = Vec::new();
    let (was, now) = (xc_nodes(before), xc_nodes(after));
    let now_refs = xc_references(after);
    for (what, spelling) in xc_references(before) {
        let Some(named) = xc_resolve(&was, &spelling) else {
            continue;
        };
        match now_refs.get(&what).and_then(|s| xc_resolve(&now, s)) {
            None => problems.push(format!(
                "the {what} names nothing ({:?})",
                now_refs.get(&what)
            )),
            Some(node) if node.resolved != named.resolved || node.kind != named.kind => {
                problems.push(format!(
                    "the {what} names {} where it named {}",
                    node.resolved, named.resolved
                ));
            }
            Some(_) => {}
        }
    }
    let spots = |nodes: &[XcNode]| {
        let mut spots: Vec<(String, String)> = nodes
            .iter()
            .map(|n| (n.kind.clone(), n.resolved.clone()))
            .collect();
        spots.sort();
        spots
    };
    if moved && spots(&was) != spots(&now) {
        problems.push("a node resolves somewhere else after the move".to_string());
    }
    let text = xcproj::serialize(after);
    match xcproj::parse(&text) {
        Ok(reparsed) if xcproj::serialize(&reparsed) == text => {}
        Ok(_) => problems.push("does not read back as written".to_string()),
        Err(e) => problems.push(format!("does not parse back: {e}")),
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// One xcproj fixture's run.
struct XcRun<'a> {
    base: xcproj::Value,
    label: String,
    report: &'a mut Report,
    edits: Edits,
}

impl XcRun<'_> {
    /// [`PbxRun::trial`] for a project.xcproj. `moved` says the edit is a
    /// move, which may change no node's place on disk.
    fn trial<T>(
        &mut self,
        what: &str,
        moved: bool,
        start: Option<&xcproj::Value>,
        edit: impl FnOnce(&mut xcproj::Value) -> Result<T, String>,
    ) -> Option<(xcproj::Value, T)> {
        self.report.edits += 1;
        let before = start.unwrap_or(&self.base);
        let mut after = before.clone();
        let Ok(outcome) = edit(&mut after) else {
            if xcproj::serialize(&after) != xcproj::serialize(before) {
                self.fail(what, "a refusal changed the document");
            }
            return None;
        };
        if let Err(problem) = check_xc(before, &after, moved) {
            self.fail(what, &problem);
        }
        Some((after, outcome))
    }

    fn fail(&mut self, what: &str, problem: &str) {
        self.report
            .failures
            .push(format!("{}: {what}: {problem}", self.label));
    }

    fn restored(&mut self, what: &str, doc: &xcproj::Value) {
        if xcproj::serialize(doc) != xcproj::serialize(&self.base) {
            self.fail(what, "the document is not restored");
        }
    }

    fn keep(&mut self, what: &str, doc: &xcproj::Value) {
        if self.edits.len() < 6 {
            self.edits.push((what.to_string(), xcproj::serialize(doc)));
        }
    }
}

/// The nodes an xcproj run edits: each by a spelling that names it alone,
/// with whether it is a group and the group holding it.
struct XcSample {
    nodes: Vec<(String, bool, Option<String>)>,
    groups: Vec<String>,
}

fn xc_sample(base: &xcproj::Value) -> XcSample {
    let nodes = xc_nodes(base);
    let files = tree_xcproj::list_filerefs(base).unwrap_or_default();
    let groups = tree_xcproj::list_groups(base).unwrap_or_default();
    // The id where the navigator path names more than one node.
    let unique = |address: &str| {
        let hits: Vec<&XcNode> = nodes.iter().filter(|n| n.address == address).collect();
        match hits.as_slice() {
            [one] => Some(one.address.clone()),
            many => many
                .iter()
                .find_map(|n| n.id.as_ref().map(|id| format!("id:{id}"))),
        }
    };
    let parents: BTreeMap<String, Option<String>> = files
        .iter()
        .map(|f| (f.address.clone(), f.parent.clone()))
        .chain(groups.iter().map(|g| (g.address.clone(), g.parent.clone())))
        .collect();
    let group_addresses: Vec<String> = groups.iter().map(|g| g.address.clone()).collect();
    // The nodes the rest of the document names come first.
    let mut sample: Vec<String> = xc_references(base)
        .values()
        .filter_map(|s| xc_resolve(&nodes, s).map(|n| n.address.clone()))
        .take(4)
        .collect();
    let file_addresses: Vec<String> = files.iter().map(|f| f.address.clone()).collect();
    sample.extend(spread(&file_addresses, 4));
    sample.extend(spread(&group_addresses, 4));
    sample.extend(
        nodes
            .iter()
            .filter(|n| n.kind == "folder")
            .map(|n| n.address.clone())
            .take(2),
    );
    let mut seen = BTreeSet::new();
    sample.retain(|a| seen.insert(a.clone()));
    XcSample {
        nodes: sample
            .iter()
            .filter_map(|address| {
                let spelling = unique(address)?;
                let parent = parents.get(address).cloned().flatten();
                Some((spelling, group_addresses.contains(address), parent))
            })
            .collect(),
        groups: group_addresses.iter().filter_map(|g| unique(g)).collect(),
    }
}

/// Every verb against one project.xcproj document.
fn exercise_xc(path: &Path, report: &mut Report) -> Edits {
    let base = xcproj::parse_file(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let sample = xc_sample(&base);
    let mut run = XcRun {
        base,
        label: label_of(path),
        report,
        edits: Vec::new(),
    };
    for (spelling, is_group, parent) in &sample.nodes {
        xc_remove_and_move(&mut run, spelling, *is_group, parent.as_deref(), &sample);
    }
    xc_adds(&mut run, &sample);
    xc_membership_and_folders(&mut run);
    xc_settings_and_packages(&mut run);
    run.edits
}

fn xc_remove_and_move(
    run: &mut XcRun<'_>,
    spelling: &str,
    is_group: bool,
    parent: Option<&str>,
    sample: &XcSample,
) {
    for force in [false, true] {
        let what = format!("remove {spelling} (force {force})");
        if let Some((doc, _)) = run.trial(&what, false, None, |root| {
            if is_group {
                tree_xcproj::remove_group(root, spelling, force)
            } else {
                tree_xcproj::remove_fileref(root, spelling, force)
            }
        }) {
            run.keep(&what, &doc);
        }
    }
    let below = format!("{spelling}/");
    let destinations = std::iter::once(None).chain(
        spread(&sample.groups, 3)
            .into_iter()
            .filter(|g| g != spelling && !g.starts_with(&below))
            .take(2)
            .map(Some),
    );
    for to in destinations {
        let what = format!("move {spelling} to {to:?}");
        let Some((moved, outcome)) = run.trial(&what, true, None, |root| {
            tree_xcproj::move_node(root, spelling, to.as_deref())
        }) else {
            continue;
        };
        let MoveOutcome::Moved { address: now, .. } = outcome else {
            continue;
        };
        run.keep(&what, &moved);
        let now = if spelling.starts_with("id:") {
            spelling
        } else {
            &now
        };
        let what = format!("{what} and back");
        run.trial(&what, true, Some(&moved), |root| {
            tree_xcproj::move_node(root, now, parent.or(Some("/")))
        });
    }
}

/// An add and the remove of what it added leave the document as it was.
fn xc_adds(run: &mut XcRun<'_>, sample: &XcSample) {
    let groups = sample.groups.iter().take(3).map(Some).chain([None]);
    for group in groups {
        let what = format!("add a file under {group:?} and remove it");
        if let Some((added, AddRefOutcome::Created { address, .. })) =
            run.trial(&what, false, None, |root| {
                tree_xcproj::add_fileref(
                    root,
                    "SweetpadProbe.swift",
                    Some("sourcecode.swift"),
                    "<group>",
                    group.map(String::as_str),
                )
            })
            && let Some((removed, _)) = run.trial(&what, false, Some(&added), |root| {
                tree_xcproj::remove_fileref(root, &address, false)
            })
        {
            run.restored(&what, &removed);
        }
        let what = format!("add a group under {group:?} and remove it");
        if let Some((added, AddGroupOutcome::Created { address, .. })) =
            run.trial(&what, false, None, |root| {
                tree_xcproj::add_group(
                    root,
                    "SweetpadProbe",
                    group.map(String::as_str),
                    Some("SweetpadProbe"),
                    "<group>",
                )
            })
            && let Some((removed, _)) = run.trial(&what, false, Some(&added), |root| {
                tree_xcproj::remove_group(root, &address, false)
            })
        {
            run.restored(&what, &removed);
        }
    }
}

fn xc_membership_and_folders(run: &mut XcRun<'_>) {
    for target in spread(&settings_xcproj::target_names(&run.base), 3) {
        let members = membership_xcproj::classic_members(&run.base, &target).unwrap_or_default();
        for entry in spread(&members, 2) {
            let what = format!(
                "membership remove {} from {target} and add it back",
                entry.path
            );
            let paths = std::slice::from_ref(&entry.path);
            if let Some((removed, _)) = run.trial(&what, false, None, |root| {
                membership_xcproj::remove_membership(root, &target, paths)
            }) {
                run.keep(&what, &removed);
                run.trial(&what, false, Some(&removed), |root| {
                    membership_xcproj::add_membership(root, &target, paths, &entry.phase)
                });
            }
        }
    }
    for folders in sync_xcproj::list(&run.base).unwrap_or_default() {
        let target = folders.target;
        for folder in folders.roots.iter().take(2) {
            let what = format!("folder remove {} from {target}", folder.dir);
            if let Some((doc, _)) = run.trial(&what, false, None, |root| {
                sync_xcproj::remove_root(root, &target, &folder.dir)
            }) {
                run.keep(&what, &doc);
            }
            let probe = format!("{}/SweetpadProbe.swift", folder.dir);
            let what = format!("exclude {probe} from {target} and include it back");
            if let Some((excluded, _)) = run.trial(&what, false, None, |root| {
                sync_xcproj::exclude(root, &target, &probe)
            }) && let Some((included, _)) = run.trial(&what, false, Some(&excluded), |root| {
                sync_xcproj::include(root, &target, &probe)
            }) {
                run.restored(&what, &included);
            }
        }
    }
}

fn xc_settings_and_packages(run: &mut XcRun<'_>) {
    let targets = settings_xcproj::target_names(&run.base);
    let probe = Assignment {
        key: "SWEETPAD_PROBE".to_string(),
        op: Op::Assign(vec!["1".to_string()]),
    };
    let keys = ["SWEETPAD_PROBE".to_string()];
    let scopes = std::iter::once(Scope::Project).chain(targets.first().cloned().map(Scope::Target));
    for scope in scopes {
        let what = format!("settings set and unset on {scope:?}");
        if let Some((set, _)) = run.trial(&what, false, None, |root| {
            settings_xcproj::set(root, &scope, &[], std::slice::from_ref(&probe))
        }) && let Some((unset, _)) = run.trial(&what, false, Some(&set), |root| {
            settings_xcproj::unset(root, &scope, &[], &keys)
        }) {
            run.restored(&what, &unset);
        }
    }
    let Some(target) = targets.first() else {
        return;
    };
    let what = format!("dependency add and remove on {target}");
    let requirement = RequirementSpec::UpToNextMajor("1.0.0".to_string());
    if let Some((added, name)) = run.trial(&what, false, None, |root| {
        let name = spm_xcproj::add_remote_dependency(root, PROBE_URL, &requirement)?;
        spm_xcproj::link_product(root, &name, "SweetpadProbe", target)?;
        Ok(name)
    }) && let Some((removed, ())) = run.trial(&what, false, Some(&added), |root| {
        spm_xcproj::remove_package(root, &name, &[])
    }) {
        run.restored(&what, &removed);
    }
}

#[test]
fn every_xcproj_write_keeps_the_project_whole() {
    let (report, live) = exercise_all("project.xcproj", exercise_xc);
    report.finish("xcproj");
    if std::env::var(LIVE).is_ok() {
        xcode_opens(&live);
    }
}

/// The targets and configurations `xcodebuild -list` reports for a bundle,
/// or why it could not. xcodebuild writes a result bundle for a failure, and
/// puts it in the user's temp directory whatever `TMPDIR` says unless it is
/// told where.
fn xcode_list(bundle: &Path, results: &Path) -> Result<String, String> {
    let list = |attempt: usize| {
        Command::new("xcodebuild")
            .arg("-list")
            .arg("-project")
            .arg(bundle)
            .arg("-disableAutomaticPackageResolution")
            .arg("-resultBundlePath")
            .arg(results.with_extension(format!("{attempt}.xcresult")))
            .output()
            .map_err(|e| format!("xcodebuild: {e}"))
    };
    // Under load xcodebuild sometimes exits with nothing said, so a failure
    // is asked a second time before it counts.
    let mut out = list(0)?;
    if !out.status.success() {
        out = list(1)?;
    }
    if !out.status.success() {
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        let reason: Vec<&str> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .take(6)
            .collect();
        return Err(format!("{} | {}", out.status, reason.join(" | ")));
    }
    // The schemes Xcode would make up differ with nothing in the edit, so
    // only the targets and configurations are compared.
    let mut kept = Vec::new();
    let mut section = "";
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let trimmed = line.trim();
        if trimmed.ends_with(':') {
            section = if matches!(trimmed, "Targets:" | "Build Configurations:") {
                trimmed
            } else {
                ""
            };
        } else if !section.is_empty() && !trimmed.is_empty() {
            kept.push(format!("{section} {trimmed}"));
        }
    }
    Ok(kept.join("\n"))
}

/// Write each edited document into a copy of its bundle and have Xcode list
/// it, next to the unedited copy.
fn xcode_opens(projects: &[Edited]) {
    let scratch = TempDir::new("write-integrity");
    let failures = Mutex::new(Vec::new());
    let skipped = Mutex::new(Vec::new());
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some((bundle, document, edits)) = projects.get(index) else {
                        break;
                    };
                    let copy = scratch
                        .join(index.to_string())
                        .join(bundle.file_name().unwrap());
                    copy_tree(bundle, &copy);
                    let text = std::fs::read_to_string(copy.join(document)).unwrap_or_default();
                    if names_packages(&text) {
                        let why = "it names Swift packages";
                        skipped
                            .lock()
                            .unwrap()
                            .push(format!("{}: {why}", label_of(bundle)));
                        continue;
                    }
                    let results = |n: usize| copy.with_extension(format!("{n}.xcresult"));
                    let baseline = match xcode_list(&copy, &results(0)) {
                        Ok(listed) => listed,
                        Err(why) => {
                            skipped
                                .lock()
                                .unwrap()
                                .push(format!("{}: {why}", label_of(bundle)));
                            continue;
                        }
                    };
                    for (n, (what, text)) in edits.iter().enumerate() {
                        std::fs::write(copy.join(document), text).unwrap();
                        let problem = match xcode_list(&copy, &results(n + 1)) {
                            Ok(listed) if listed == baseline => continue,
                            Ok(listed) => format!(
                                "Xcode lists\n{listed}\nwhere the unedited copy lists\n{baseline}"
                            ),
                            Err(why) => why,
                        };
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("{}: {what}: {problem}", label_of(bundle)));
                    }
                }
            });
        }
    });
    let skipped = skipped.into_inner().unwrap();
    eprintln!(
        "{} of {} projects skipped, with why:\n{}",
        skipped.len(),
        projects.len(),
        skipped.join("\n")
    );
    let failures = failures.into_inner().unwrap();
    assert!(
        failures.is_empty(),
        "Xcode cannot open {} edited projects:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        skipped.len() < projects.len(),
        "Xcode listed no project at all"
    );
}

/// Whether a project document, in either format, names a Swift package.
fn names_packages(text: &str) -> bool {
    [
        "XCRemoteSwiftPackageReference",
        "XCLocalSwiftPackageReference",
        "\"packages\": [",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// A small app, moved around and built in both formats: the group holding its
/// sources leaves for the navigator root and comes back, and its tests and
/// products groups move under its sources, so a build has to find every file
/// and every product through the rewritten document.
#[test]
fn a_moved_project_still_builds() {
    if std::env::var(LIVE).is_err() {
        eprintln!("skipping: set {LIVE}=1 to build an edited project with xcodebuild");
        return;
    }
    let scratch = TempDir::new("write-integrity-build");
    for format in ["project.pbxproj", "project.xcproj"] {
        let dir = scratch.join(format);
        copy_tree(
            &fixtures().join("_synthetic-objectversion-110/project"),
            &dir,
        );
        let bundle = dir.join("SweetpadCIApp.xcodeproj");
        let document = bundle.join(format);
        if format == "project.xcproj" {
            std::fs::remove_file(bundle.join("project.pbxproj")).unwrap();
            std::fs::copy(
                fixtures().join("_xcproj/SweetpadCIApp.xcodeproj/project.xcproj"),
                &document,
            )
            .unwrap();
        }
        let text = std::fs::read_to_string(&document).unwrap();
        let edited = if format == "project.pbxproj" {
            moved_pbxproj(&text)
        } else {
            let mut root = xcproj::parse(&text).unwrap();
            tree_xcproj::move_node(&mut root, "Sources/App", None).unwrap();
            tree_xcproj::move_node(&mut root, "Tests/AppTests", Some("Sources")).unwrap();
            tree_xcproj::move_node(&mut root, "Products", Some("Sources")).unwrap();
            tree_xcproj::move_node(&mut root, "App", Some("Sources")).unwrap();
            xcproj::serialize(&root)
        };
        std::fs::write(&document, edited).unwrap();
        let out = Command::new("xcodebuild")
            .arg("build")
            .arg("-project")
            .arg(&bundle)
            .args(["-scheme", "SweetpadCIMac", "-derivedDataPath"])
            .arg(scratch.join(format!("{format}-dd")))
            .args(["CODE_SIGNING_ALLOWED=NO", "COMPILER_INDEX_STORE_ENABLE=NO"])
            .arg("PRODUCT_BUNDLE_IDENTIFIER=dev.sweetpad.writeintegrity")
            .arg("-resultBundlePath")
            .arg(scratch.join(format!("{format}.xcresult")))
            .output()
            .expect("run xcodebuild");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let errors: Vec<&str> = stdout.lines().filter(|l| l.contains("error")).collect();
        assert!(
            out.status.success(),
            "{format}: the edited project does not build:\n{}\n{}",
            errors.join("\n"),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// The pbxproj half of [`a_moved_project_still_builds`]'s moves.
fn moved_pbxproj(text: &str) -> String {
    let mut root = pbxproj::parse(text).unwrap();
    let groups = tree_pbxproj::list_groups(&root).unwrap();
    let id = |path: &str| {
        groups
            .iter()
            .find(|g| g.navigator_path.as_deref() == Some(path))
            .map_or_else(|| panic!("no group {path}"), |g| g.address.clone())
    };
    let (app, tests, sources, products) = (
        id("Sources/App"),
        id("Tests/AppTests"),
        id("Sources"),
        id("Products"),
    );
    tree_pbxproj::move_node(&mut root, &app, None).unwrap();
    tree_pbxproj::move_node(&mut root, &tests, Some(&sources)).unwrap();
    tree_pbxproj::move_node(&mut root, &products, Some(&sources)).unwrap();
    tree_pbxproj::move_node(&mut root, &app, Some(&sources)).unwrap();
    sweetpad_lib::pbxproj_writer::serialize(&root, "SweetpadCIApp")
}
