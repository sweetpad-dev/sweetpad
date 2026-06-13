//! Three-way semantic merge of SwiftPM `Package.resolved` files.
//!
//! `Package.resolved` is JSON (so, per the crate's dependency policy, parsed
//! with `serde_json` rather than a hand-rolled reader), but it conflicts in git
//! almost as often as `pbxproj`: every branch that bumps a dependency rewrites
//! the same `pins` array. The pins are an array *keyed by `identity`*, so a
//! line-based merge collides needlessly. This module merges them by identity —
//! unioning disjoint pins, taking one-sided version bumps, and flagging only a
//! genuine both-sides-bumped-differently as a [`Conflict`].
//!
//! Output is re-rendered to Xcode's exact shape ([`serialize`]): 2-space
//! indent, `" : "` separators, alphabetically-sorted keys (serde_json's default
//! `Map` ordering), pins sorted by identity, trailing newline. `originHash` is
//! a digest Xcode derives from the pins and regenerates on the next resolve, so
//! the merge never treats it as a conflict — it keeps ours' value and lets
//! Xcode refresh it.
//!
//! Conflicts reuse [`crate::pbxproj_merge`]'s [`Conflict`]/[`ConflictKind`]
//! vocabulary so the `resolve` commands report both file kinds uniformly.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use serde_json::{Map, Value};

use crate::pbxproj_merge::{Conflict, ConflictKind};

/// The merged document plus any contradictions left for a human.
#[derive(Debug)]
pub struct SpmMerge {
    pub value: Value,
    pub conflicts: Vec<Conflict>,
}

impl SpmMerge {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// Three-way merge `ours` and `theirs` over their common `base` (optional, to
/// tolerate an add/add file with no merge base).
#[must_use]
pub fn merge(base: Option<&Value>, ours: &Value, theirs: &Value) -> SpmMerge {
    let mut merger = Merger {
        conflicts: Vec::new(),
    };
    let value = merger.merge_value("", base, ours, theirs);
    SpmMerge {
        value,
        conflicts: merger.conflicts,
    }
}

struct Merger {
    conflicts: Vec<Conflict>,
}

impl Merger {
    fn merge_value(
        &mut self,
        path: &str,
        base: Option<&Value>,
        ours: &Value,
        theirs: &Value,
    ) -> Value {
        if ours == theirs {
            return ours.clone();
        }
        if let Some(b) = base {
            if b == ours {
                return theirs.clone();
            }
            if b == theirs {
                return ours.clone();
            }
        }
        match (ours, theirs) {
            (Value::Object(o), Value::Object(t)) => {
                Value::Object(self.merge_object(path, base.and_then(Value::as_object), o, t))
            }
            (Value::Array(o), Value::Array(t)) => Value::Array(self.merge_array(
                path,
                base.and_then(Value::as_array).map(Vec::as_slice),
                o,
                t,
            )),
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

    fn merge_object(
        &mut self,
        path: &str,
        base: Option<&Map<String, Value>>,
        ours: &Map<String, Value>,
        theirs: &Map<String, Value>,
    ) -> Map<String, Value> {
        let mut result = Map::new();
        // Union of keys; the result Map sorts them, matching Xcode.
        let keys: BTreeSet<&String> = base
            .into_iter()
            .flat_map(|m| m.keys())
            .chain(ours.keys())
            .chain(theirs.keys())
            .collect();

        for key in keys {
            // originHash is a digest of the pins that Xcode regenerates; never
            // a real conflict. Keep ours (or theirs) and move on.
            if path.is_empty() && key == "originHash" {
                if let Some(v) = ours.get(key).or_else(|| theirs.get(key)) {
                    result.insert(key.clone(), v.clone());
                }
                continue;
            }

            let b = base.and_then(|m| m.get(key));
            let o = ours.get(key);
            let t = theirs.get(key);
            match (b, o, t) {
                (_, Some(o), Some(t)) => {
                    let child = join(path, key);
                    result.insert(key.clone(), self.merge_value(&child, b, o, t));
                }
                (None, Some(o), None) => {
                    result.insert(key.clone(), o.clone());
                }
                (None, None, Some(t)) => {
                    result.insert(key.clone(), t.clone());
                }
                (Some(b), None, Some(t)) => {
                    if b != t {
                        self.conflicts.push(Conflict {
                            path: join(path, key),
                            kind: ConflictKind::ModifyDelete,
                            detail: format!("ours deleted; theirs modified to {}", summarize(t)),
                        });
                        result.insert(key.clone(), t.clone());
                    }
                }
                (Some(b), Some(o), None) => {
                    if b != o {
                        self.conflicts.push(Conflict {
                            path: join(path, key),
                            kind: ConflictKind::ModifyDelete,
                            detail: format!("theirs deleted; ours modified to {}", summarize(o)),
                        });
                        result.insert(key.clone(), o.clone());
                    }
                }
                (_, None, None) => {}
            }
        }
        result
    }

    /// Merge an array. `pins`-shaped arrays (every element an object with a
    /// string `identity`) merge by that identity and re-sort; anything else is
    /// an ordered-set union.
    fn merge_array(
        &mut self,
        path: &str,
        base: Option<&[Value]>,
        ours: &[Value],
        theirs: &[Value],
    ) -> Vec<Value> {
        let base = base.unwrap_or(&[]);
        if keyed(ours) && keyed(theirs) {
            return self.merge_keyed(path, base, ours, theirs);
        }
        // Ordered-set union: ours (minus base elements theirs deleted), then
        // theirs-only additions.
        let mut result: Vec<Value> = Vec::new();
        for v in ours {
            if contains(base, v) && !contains(theirs, v) {
                continue;
            }
            if !contains(&result, v) {
                result.push(v.clone());
            }
        }
        for v in theirs {
            if !contains(base, v) && !contains(ours, v) && !contains(&result, v) {
                result.push(v.clone());
            }
        }
        result
    }

    /// Merge two `pins`-shaped arrays by `identity`, three-way per pin, then
    /// sort by identity (Xcode's canonical order).
    fn merge_keyed(
        &mut self,
        path: &str,
        base: &[Value],
        ours: &[Value],
        theirs: &[Value],
    ) -> Vec<Value> {
        let ids: BTreeSet<&str> = base
            .iter()
            .chain(ours)
            .chain(theirs)
            .filter_map(identity)
            .collect();

        let mut result = Vec::new();
        for id in ids {
            let b = find(base, id);
            let o = find(ours, id);
            let t = find(theirs, id);
            match (b, o, t) {
                (_, Some(o), Some(t)) => {
                    result.push(self.merge_value(&join(path, id), b, o, t));
                }
                (None, Some(o), None) => result.push(o.clone()),
                (None, None, Some(t)) => result.push(t.clone()),
                (Some(b), None, Some(t)) => {
                    if b != t {
                        self.conflicts.push(Conflict {
                            path: join(path, id),
                            kind: ConflictKind::ModifyDelete,
                            detail: "ours removed this pin; theirs changed it".into(),
                        });
                        result.push(t.clone());
                    }
                }
                (Some(b), Some(o), None) => {
                    if b != o {
                        self.conflicts.push(Conflict {
                            path: join(path, id),
                            kind: ConflictKind::ModifyDelete,
                            detail: "theirs removed this pin; ours changed it".into(),
                        });
                        result.push(o.clone());
                    }
                }
                (_, None, None) => {}
            }
        }
        result
    }
}

fn keyed(arr: &[Value]) -> bool {
    !arr.is_empty() && arr.iter().all(|v| identity(v).is_some())
}

fn identity(v: &Value) -> Option<&str> {
    v.get("identity").and_then(Value::as_str)
}

fn find<'a>(arr: &'a [Value], id: &str) -> Option<&'a Value> {
    arr.iter().find(|v| identity(v) == Some(id))
}

fn contains(slice: &[Value], v: &Value) -> bool {
    slice.iter().any(|x| x == v)
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}/{key}")
    }
}

fn summarize(v: &Value) -> String {
    match v {
        Value::String(s) => format!("\"{s}\""),
        Value::Array(a) => format!("[{} items]", a.len()),
        Value::Object(o) => format!("{{{} keys}}", o.len()),
        other => other.to_string(),
    }
}

/// Render to Xcode's `Package.resolved` style: 2-space indent, `" : "`
/// separators, sorted keys (serde_json `Map` default), trailing newline.
#[must_use]
pub fn serialize(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0);
    out.push('\n');
    out
}

fn write_value(out: &mut String, value: &Value, indent: usize) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            out.push_str("{\n");
            let last = map.len() - 1;
            for (i, (k, v)) in map.iter().enumerate() {
                push_indent(out, indent + 1);
                // serde_json renders the key with correct JSON string escaping.
                out.push_str(&Value::String(k.clone()).to_string());
                out.push_str(" : ");
                write_value(out, v, indent + 1);
                if i != last {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, indent);
            out.push('}');
        }
        Value::Array(items) if !items.is_empty() => {
            out.push_str("[\n");
            let last = items.len() - 1;
            for (i, v) in items.iter().enumerate() {
                push_indent(out, indent + 1);
                write_value(out, v, indent + 1);
                if i != last {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, indent);
            out.push(']');
        }
        // Empty object/array and all scalars render exactly as serde_json does.
        other => {
            let _ = write!(out, "{other}");
        }
    }
}

fn push_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    /// A v3 document with one pin, as Xcode writes it (byte-for-byte target).
    const FIXTURE: &str = r#"{
  "originHash" : "b1df43e027bbdf1507591168740d8722cc9decaf55c89a6641d41c7ec4da687a",
  "pins" : [
    {
      "identity" : "swift-syntax",
      "kind" : "remoteSourceControl",
      "location" : "https://github.com/swiftlang/swift-syntax.git",
      "state" : {
        "revision" : "0687f71944021d616d34d922343dcef086855920",
        "version" : "600.0.1"
      }
    }
  ],
  "version" : 3
}
"#;

    fn pin(identity: &str, version: &str) -> Value {
        json(&format!(
            r#"{{ "identity": "{identity}", "kind": "remoteSourceControl",
                 "location": "https://example.com/{identity}.git",
                 "state": {{ "revision": "rev-{version}", "version": "{version}" }} }}"#
        ))
    }
    fn doc(pins: Vec<Value>) -> Value {
        Value::Object(Map::from_iter([
            ("originHash".to_string(), Value::String("hash".into())),
            ("pins".to_string(), Value::Array(pins)),
            ("version".to_string(), Value::Number(3.into())),
        ]))
    }

    #[test]
    fn serialize_matches_xcode_byte_for_byte() {
        assert_eq!(serialize(&json(FIXTURE)), FIXTURE);
    }

    #[test]
    fn disjoint_pin_additions_union_and_sort() {
        let base = doc(vec![pin("alamofire", "5.9.0")]);
        let ours = doc(vec![pin("alamofire", "5.9.0"), pin("snapkit", "5.7.0")]);
        let theirs = doc(vec![pin("alamofire", "5.9.0"), pin("kingfisher", "7.0.0")]);

        let m = merge(Some(&base), &ours, &theirs);

        assert!(m.is_clean(), "{:?}", m.conflicts);
        let pins = m.value.get("pins").unwrap().as_array().unwrap();
        let ids: Vec<&str> = pins.iter().filter_map(identity).collect();
        // Sorted by identity.
        assert_eq!(ids, vec!["alamofire", "kingfisher", "snapkit"]);
    }

    #[test]
    fn one_sided_version_bump_is_taken() {
        let base = doc(vec![pin("alamofire", "5.9.0")]);
        let ours = doc(vec![pin("alamofire", "5.10.0")]); // ours bumps
        let theirs = base.clone(); // theirs unchanged

        let m = merge(Some(&base), &ours, &theirs);

        assert!(m.is_clean(), "{:?}", m.conflicts);
        let v = m.value.get("pins").unwrap()[0]
            .get("state")
            .unwrap()
            .get("version")
            .unwrap();
        assert_eq!(v.as_str(), Some("5.10.0"));
    }

    #[test]
    fn both_sides_bump_same_pin_differently_conflicts() {
        let base = doc(vec![pin("alamofire", "5.9.0")]);
        let ours = doc(vec![pin("alamofire", "5.10.0")]);
        let theirs = doc(vec![pin("alamofire", "5.9.1")]);

        let m = merge(Some(&base), &ours, &theirs);

        // revision and version both diverge → both reported, under the pin.
        assert!(!m.is_clean());
        assert!(
            m.conflicts.iter().all(|c| c.path.contains("alamofire")),
            "{:?}",
            m.conflicts
        );
    }

    #[test]
    fn one_side_adds_other_removes_a_different_pin() {
        let base = doc(vec![pin("alamofire", "5.9.0"), pin("old", "1.0.0")]);
        let ours = doc(vec![
            pin("alamofire", "5.9.0"),
            pin("old", "1.0.0"),
            pin("new", "2.0.0"),
        ]);
        let theirs = doc(vec![pin("alamofire", "5.9.0")]); // removed "old"

        let m = merge(Some(&base), &ours, &theirs);

        assert!(m.is_clean(), "{:?}", m.conflicts);
        let ids: Vec<&str> = m
            .value
            .get("pins")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(identity)
            .collect();
        assert_eq!(ids, vec!["alamofire", "new"]); // "old" deleted, "new" added
    }

    #[test]
    fn diverged_origin_hash_is_not_a_conflict() {
        // Both sides have different originHash (because pins changed) but the
        // pins themselves merge cleanly — must not conflict on the digest.
        let mut base = doc(vec![pin("a", "1.0.0")]);
        let mut ours = doc(vec![pin("a", "1.0.0"), pin("b", "1.0.0")]);
        let mut theirs = doc(vec![pin("a", "1.0.0"), pin("c", "1.0.0")]);
        base["originHash"] = Value::String("base-hash".into());
        ours["originHash"] = Value::String("ours-hash".into());
        theirs["originHash"] = Value::String("theirs-hash".into());

        let m = merge(Some(&base), &ours, &theirs);

        assert!(
            m.is_clean(),
            "originHash divergence must not conflict: {:?}",
            m.conflicts
        );
        assert_eq!(
            m.value.get("originHash").unwrap().as_str(),
            Some("ours-hash")
        );
    }
}
