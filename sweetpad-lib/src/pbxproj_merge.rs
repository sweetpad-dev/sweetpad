//! Three-way semantic merge of parsed `project.pbxproj` object graphs.
//!
//! Git's textual merge mangles `pbxproj` files: they are flat, UUID-keyed
//! plists, so line-based conflicts land in arbitrary spots and the result is
//! usually unparseable. This module merges at the *object graph* level instead
//! — diffing `ours` and `theirs` against the common `base` per UUID-keyed
//! object and per field — then hands a clean [`crate::pbxproj::Value`] back to
//! [`crate::pbxproj_writer::serialize`], which regenerates Xcode's exact
//! on-disk bytes.
//!
//! The merge is conservative: anything it can resolve unambiguously (disjoint
//! additions, one-sided deletions, identical edits) it resolves silently;
//! anything genuinely contradictory (both sides set the same scalar to
//! different values, or one side edits what the other deletes) is recorded as a
//! [`Conflict`] for a human to settle. The caller is expected to *not* write
//! the file when conflicts remain.
//!
//! The engine is pure (no I/O, no git, no Xcode), so it is unit-tested below
//! without a Mac. The `sweetpad pbxproj resolve` command supplies the three
//! inputs from git's merge state and persists the result.

use std::collections::HashSet;

use crate::pbxproj::{Dict, Value};

/// Why a node could not be merged automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Both sides changed the same scalar to different values.
    BothModified,
    /// One side modified a node the other side deleted.
    ModifyDelete,
    /// Both sides introduced the same key with different values (no base).
    AddAdd,
}

impl ConflictKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ConflictKind::BothModified => "both-modified",
            ConflictKind::ModifyDelete => "modify-delete",
            ConflictKind::AddAdd => "add-add",
        }
    }
}

/// A single unresolved contradiction, located by a human-readable path through
/// the object graph (e.g. `objects/ABC123 (XCBuildConfiguration)/buildSettings/PRODUCT_NAME`).
#[derive(Debug, Clone)]
pub struct Conflict {
    pub path: String,
    pub kind: ConflictKind,
    pub detail: String,
}

/// The outcome of a three-way merge: the merged graph plus any contradictions
/// that need human resolution. When `conflicts` is non-empty the `value` still
/// holds a best-effort result (ours wins each contested scalar), but callers
/// should treat the merge as failed and leave the file for manual fixing.
#[derive(Debug)]
pub struct Merge {
    pub value: Value,
    pub conflicts: Vec<Conflict>,
}

impl Merge {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// Three-way merge `ours` and `theirs` over their common `base`. `base` is
/// optional to tolerate an add/add file (no merge base in git's index).
#[must_use]
pub fn merge(base: Option<&Value>, ours: &Value, theirs: &Value) -> Merge {
    let mut merger = Merger {
        conflicts: Vec::new(),
    };
    let value = merger.merge_value("", base, ours, theirs);
    Merge {
        value,
        conflicts: merger.conflicts,
    }
}

struct Merger {
    conflicts: Vec<Conflict>,
}

impl Merger {
    /// The core three-way rule, recursing into dicts and arrays. Returns the
    /// merged value; records a [`Conflict`] (and falls back to `ours`) for an
    /// irreconcilable scalar.
    fn merge_value(
        &mut self,
        path: &str,
        base: Option<&Value>,
        ours: &Value,
        theirs: &Value,
    ) -> Value {
        // Identical sides: nothing to reconcile (covers "both made the same
        // edit" and "neither edited").
        if ours == theirs {
            return ours.clone();
        }
        // Only one side diverged from base — take the side that changed.
        if let Some(b) = base {
            if b == ours {
                return theirs.clone();
            }
            if b == theirs {
                return ours.clone();
            }
        }
        // Both sides changed, differently. Structured values merge field-wise;
        // scalars (or type mismatches) are a true conflict.
        match (ours, theirs) {
            (Value::Dict(o), Value::Dict(t)) => {
                Value::Dict(self.merge_dict(path, base.and_then(Value::as_dict), o, t))
            }
            (Value::Array(o), Value::Array(t)) => {
                Value::Array(merge_array(base.and_then(Value::as_array), o, t))
            }
            _ => {
                self.conflicts.push(Conflict {
                    path: path.to_string(),
                    kind: if base.is_none() {
                        ConflictKind::AddAdd
                    } else {
                        ConflictKind::BothModified
                    },
                    detail: format!("ours={}, theirs={}", summarize(ours), summarize(theirs)),
                });
                ours.clone()
            }
        }
    }

    /// Key-keyed three-way merge of a dict. Keys are visited in base order
    /// first (so surviving entries keep their original position for stable,
    /// low-churn output), then ours-only, then theirs-only additions.
    fn merge_dict(&mut self, path: &str, base: Option<&Dict>, ours: &Dict, theirs: &Dict) -> Dict {
        let mut result = Dict::new();
        // Preserve the parser's single-line layout hint so the writer replays
        // e.g. PBXBuildFile bodies on one line, byte-for-byte.
        result.set_single_line(ours.is_single_line());

        for key in ordered_keys(base, ours, theirs) {
            let b = base.and_then(|d| d.get(&key));
            let o = ours.get(&key);
            let t = theirs.get(&key);
            match (b, o, t) {
                // Present (or added) on both sides — recurse.
                (_, Some(o), Some(t)) => {
                    let child = child_path(path, &key, ours);
                    result.insert(key, self.merge_value(&child, b, o, t));
                }
                // Added by exactly one side.
                (None, Some(o), None) => {
                    result.insert(key, o.clone());
                }
                (None, None, Some(t)) => {
                    result.insert(key, t.clone());
                }
                // Deleted by ours.
                (Some(b), None, Some(t)) => {
                    if b == t {
                        // theirs untouched, ours deleted → honor the delete.
                    } else {
                        self.conflicts.push(Conflict {
                            path: child_path(path, &key, theirs),
                            kind: ConflictKind::ModifyDelete,
                            detail: format!("ours deleted; theirs modified to {}", summarize(t)),
                        });
                        result.insert(key, t.clone());
                    }
                }
                // Deleted by theirs.
                (Some(b), Some(o), None) => {
                    if b == o {
                        // ours untouched, theirs deleted → honor the delete.
                    } else {
                        self.conflicts.push(Conflict {
                            path: child_path(path, &key, ours),
                            kind: ConflictKind::ModifyDelete,
                            detail: format!("theirs deleted; ours modified to {}", summarize(o)),
                        });
                        result.insert(key, o.clone());
                    }
                }
                // Deleted by both (base present or not) — drop the key.
                (_, None, None) => {}
            }
        }
        result
    }
}

/// Three-way merge of an array, treated as an ordered set (the shape of
/// every pbxproj reference list: `children`, `files`, `buildPhases`, …).
/// Honors one-sided deletions and unions additions from both sides, biased
/// to ours' ordering. Reorder-only differences resolve to ours.
fn merge_array(base: Option<&[Value]>, ours: &[Value], theirs: &[Value]) -> Vec<Value> {
    let base = base.unwrap_or(&[]);
    let mut result: Vec<Value> = Vec::new();

    // Walk ours, dropping base elements that theirs deleted.
    for v in ours {
        let in_base = contains(base, v);
        if in_base && !contains(theirs, v) {
            continue; // theirs removed this base element
        }
        if !contains(&result, v) {
            result.push(v.clone());
        }
    }
    // Append theirs-only additions (anything not from base and not already
    // contributed by ours).
    for v in theirs {
        if contains(base, v) || contains(ours, v) || contains(&result, v) {
            continue;
        }
        result.push(v.clone());
    }
    result
}

/// Build the graph path for a child key. Inside `objects`, annotate the
/// UUID with its `isa` so conflict reports name the object kind.
fn child_path(path: &str, key: &str, side: &Dict) -> String {
    if path == "objects" {
        let isa = side
            .get(key)
            .and_then(|v| v.get("isa"))
            .and_then(Value::as_str);
        return match isa {
            Some(isa) => format!("objects/{key} ({isa})"),
            None => format!("objects/{key}"),
        };
    }
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}/{key}")
    }
}

/// Ordered union of keys across the three dicts: base order first, then
/// ours-only, then theirs-only — each key once.
fn ordered_keys(base: Option<&Dict>, ours: &Dict, theirs: &Dict) -> Vec<String> {
    let mut order = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for dict in base.into_iter().chain([ours, theirs]) {
        for k in dict.keys() {
            if seen.insert(k.clone()) {
                order.push(k.clone());
            }
        }
    }
    order
}

fn contains(slice: &[Value], v: &Value) -> bool {
    slice.iter().any(|x| x == v)
}

/// A short, single-line rendering of a value for conflict messages.
fn summarize(v: &Value) -> String {
    match v {
        Value::String(s) => format!("\"{s}\""),
        Value::Array(a) => format!("[{} items]", a.len()),
        Value::Dict(d) => format!("{{{} keys}}", d.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::String(v.to_string())
    }
    fn arr(items: Vec<Value>) -> Value {
        Value::Array(items)
    }
    fn dict(pairs: Vec<(&str, Value)>) -> Value {
        Value::Dict(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    /// A file reference object, as it would appear under `objects`.
    fn file_ref(name: &str) -> Value {
        dict(vec![("isa", s("PBXFileReference")), ("path", s(name))])
    }

    fn objects(pairs: Vec<(&str, Value)>) -> Value {
        dict(vec![("objects", dict(pairs))])
    }

    fn merged_objects(m: &Merge) -> &Dict {
        m.value.get("objects").unwrap().as_dict().unwrap()
    }

    #[test]
    fn disjoint_object_additions_both_survive() {
        // Each side adds a different file reference to a shared base.
        let base = objects(vec![("A", file_ref("Shared.swift"))]);
        let ours = objects(vec![
            ("A", file_ref("Shared.swift")),
            ("O", file_ref("Ours.swift")),
        ]);
        let theirs = objects(vec![
            ("A", file_ref("Shared.swift")),
            ("T", file_ref("Theirs.swift")),
        ]);

        let m = merge(Some(&base), &ours, &theirs);

        assert!(
            m.is_clean(),
            "disjoint adds should merge cleanly: {:?}",
            m.conflicts
        );
        let objs = merged_objects(&m);
        assert!(objs.contains_key("A"));
        assert!(objs.contains_key("O"));
        assert!(objs.contains_key("T"));
    }

    #[test]
    fn one_sided_deletion_is_honored() {
        // ours deletes B; theirs leaves it untouched → B is gone.
        let base = objects(vec![("A", file_ref("A.swift")), ("B", file_ref("B.swift"))]);
        let ours = objects(vec![("A", file_ref("A.swift"))]);
        let theirs = base.clone();

        let m = merge(Some(&base), &ours, &theirs);

        assert!(m.is_clean(), "{:?}", m.conflicts);
        let objs = merged_objects(&m);
        assert!(objs.contains_key("A"));
        assert!(
            !objs.contains_key("B"),
            "ours deleted B; it must not survive"
        );
    }

    #[test]
    fn modify_delete_is_a_conflict() {
        // theirs edits B's path; ours deletes B.
        let base = objects(vec![("B", file_ref("B.swift"))]);
        let ours = objects(vec![]);
        let theirs = objects(vec![("B", file_ref("Renamed.swift"))]);

        let m = merge(Some(&base), &ours, &theirs);

        assert_eq!(m.conflicts.len(), 1);
        assert_eq!(m.conflicts[0].kind, ConflictKind::ModifyDelete);
        assert!(m.conflicts[0].path.contains("PBXFileReference"));
    }

    #[test]
    fn same_field_edit_is_a_conflict() {
        // Both sides set PRODUCT_NAME differently on the same build config.
        let cfg = |name: &str| {
            dict(vec![
                ("isa", s("XCBuildConfiguration")),
                ("buildSettings", dict(vec![("PRODUCT_NAME", s(name))])),
            ])
        };
        let base = objects(vec![("C", cfg("Base"))]);
        let ours = objects(vec![("C", cfg("Ours"))]);
        let theirs = objects(vec![("C", cfg("Theirs"))]);

        let m = merge(Some(&base), &ours, &theirs);

        assert_eq!(m.conflicts.len(), 1, "{:?}", m.conflicts);
        assert_eq!(m.conflicts[0].kind, ConflictKind::BothModified);
        assert!(
            m.conflicts[0].path.ends_with("buildSettings/PRODUCT_NAME"),
            "path was {}",
            m.conflicts[0].path
        );
    }

    #[test]
    fn non_conflicting_field_edits_both_apply() {
        // ours edits one setting, theirs edits another, on the same object.
        let cfg = |product: &str, bundle: &str| {
            dict(vec![
                ("isa", s("XCBuildConfiguration")),
                (
                    "buildSettings",
                    dict(vec![("PRODUCT_NAME", s(product)), ("BUNDLE_ID", s(bundle))]),
                ),
            ])
        };
        let base = objects(vec![("C", cfg("App", "com.base"))]);
        let ours = objects(vec![("C", cfg("Renamed", "com.base"))]);
        let theirs = objects(vec![("C", cfg("App", "com.theirs"))]);

        let m = merge(Some(&base), &ours, &theirs);

        assert!(m.is_clean(), "{:?}", m.conflicts);
        let settings = merged_objects(&m)
            .get("C")
            .unwrap()
            .get("buildSettings")
            .unwrap();
        assert_eq!(
            settings.get("PRODUCT_NAME").unwrap().as_str(),
            Some("Renamed")
        );
        assert_eq!(
            settings.get("BUNDLE_ID").unwrap().as_str(),
            Some("com.theirs")
        );
    }

    #[test]
    fn array_additions_union_and_deletions_apply() {
        let group = |children: Vec<&str>| {
            dict(vec![
                ("isa", s("PBXGroup")),
                ("children", arr(children.into_iter().map(s).collect())),
            ])
        };
        // base [F1,F2]; ours adds F3 and drops F2; theirs adds F4.
        let base = objects(vec![("G", group(vec!["F1", "F2"]))]);
        let ours = objects(vec![("G", group(vec!["F1", "F3"]))]);
        let theirs = objects(vec![("G", group(vec!["F1", "F2", "F4"]))]);

        let m = merge(Some(&base), &ours, &theirs);

        assert!(m.is_clean(), "{:?}", m.conflicts);
        let children = merged_objects(&m)
            .get("G")
            .unwrap()
            .get("children")
            .unwrap()
            .as_array()
            .unwrap();
        let got: Vec<&str> = children.iter().filter_map(Value::as_str).collect();
        // F2 removed by ours; F3 from ours, F4 from theirs unioned in.
        assert_eq!(got, vec!["F1", "F3", "F4"]);
    }

    #[test]
    fn identical_edits_on_both_sides_are_clean() {
        let base = objects(vec![("A", file_ref("Old.swift"))]);
        let ours = objects(vec![("A", file_ref("New.swift"))]);
        let theirs = objects(vec![("A", file_ref("New.swift"))]);

        let m = merge(Some(&base), &ours, &theirs);

        assert!(m.is_clean(), "{:?}", m.conflicts);
        assert_eq!(
            merged_objects(&m)
                .get("A")
                .unwrap()
                .get("path")
                .unwrap()
                .as_str(),
            Some("New.swift")
        );
    }

    #[test]
    fn single_line_layout_hint_is_preserved_through_a_merge() {
        // A PBXBuildFile that both sides edit (different fields) must keep its
        // single-line layout so the writer reproduces Xcode's bytes.
        let build_file = |file_ref: &str, settings_key: &str| {
            let mut d = Dict::new();
            d.insert("isa".into(), s("PBXBuildFile"));
            d.insert("fileRef".into(), s(file_ref));
            d.insert(settings_key.into(), s("x"));
            d.set_single_line(true);
            Value::Dict(d)
        };
        let base = objects(vec![("BF", build_file("R1", "base"))]);
        let ours = objects(vec![("BF", build_file("R2", "base"))]); // edits fileRef
        let theirs = objects(vec![("BF", build_file("R1", "theirs"))]); // adds key

        let m = merge(Some(&base), &ours, &theirs);

        assert!(m.is_clean(), "{:?}", m.conflicts);
        let bf = merged_objects(&m).get("BF").unwrap().as_dict().unwrap();
        assert!(
            bf.is_single_line(),
            "merged object lost its single-line layout hint"
        );
    }
}
