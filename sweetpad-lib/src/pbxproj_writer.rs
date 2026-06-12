//! Serialize a parsed [`crate::pbxproj::Value`] back to Xcode's on-disk
//! `project.pbxproj` format, byte-for-byte.
//!
//! Xcode's writer is deterministic: tab indentation, `isa`-grouped sections in
//! the `objects` dict, `/* … */` annotations after every object reference, a
//! fixed quoting rule, and single-line bodies for a few object kinds. None of
//! that survives parsing (comments are skipped; quoting is decoded), so this
//! module *regenerates* it from the object graph — the annotations are pure
//! functions of the objects they decorate. The one input that isn't in the
//! file at all is the project's own name (Xcode derives it from the
//! `.xcodeproj` directory name); it appears in `XCConfigurationList` comments,
//! so [`serialize`] takes it as a parameter.
//!
//! Round-trip fidelity is verified in `tests/serializer_roundtrip.rs`, which
//! re-serializes every fixture in the corpus and compares against the raw
//! bytes.

use std::collections::{BTreeMap, HashMap};

use crate::pbxproj::{Dict, Value};

/// Serialize a parsed pbxproj document. `project_name` is the `.xcodeproj`
/// bundle's basename without extension (e.g. `Alamofire`); Xcode embeds it in
/// `Build configuration list for PBXProject "<name>"` annotations.
#[must_use]
pub fn serialize(root: &Value, project_name: &str) -> String {
    let comments = build_comments(root, project_name);
    let mut out = String::with_capacity(1 << 16);
    out.push_str("// !$*UTF8*$!\n");
    let Some(top) = root.as_dict() else {
        // Not a pbxproj-shaped document; fall back to a bare value.
        write_value(&mut out, root, 0, false, &comments, None);
        out.push('\n');
        return out;
    };
    out.push_str("{\n");
    for (key, value) in top {
        out.push('\t');
        write_string(&mut out, key, false);
        out.push_str(" = ");
        if key == "objects"
            && let Some(objects) = value.as_dict()
        {
            write_objects(&mut out, objects, &comments);
        } else {
            write_value(&mut out, value, 1, false, &comments, Some(key));
        }
        out.push_str(";\n");
    }
    out.push_str("}\n");
    out
}

/// The `objects` dict: entries grouped into `/* Begin <isa> section */`
/// blocks (sections sorted by isa, entries in source order), each object's
/// key annotated, and single-line bodies replayed from the parser's layout
/// hint.
fn write_objects(out: &mut String, objects: &Dict, comments: &HashMap<String, String>) {
    let mut sections: BTreeMap<&str, Vec<(&String, &Value)>> = BTreeMap::new();
    for (guid, obj) in objects {
        let isa = obj.get("isa").and_then(Value::as_str).unwrap_or("");
        sections.entry(isa).or_default().push((guid, obj));
    }
    out.push_str("{\n");
    for (isa, entries) in &sections {
        out.push_str("\n/* Begin ");
        out.push_str(isa);
        out.push_str(" section */\n");
        for (guid, obj) in entries {
            out.push_str("\t\t");
            write_string(out, guid, false);
            if let Some(c) = comments.get(guid.as_str()) {
                out.push_str(" /* ");
                out.push_str(c);
                out.push_str(" */");
            }
            out.push_str(" = ");
            let inline = obj.as_dict().is_some_and(Dict::is_single_line);
            write_value(out, obj, 2, inline, comments, None);
            out.push_str(";\n");
        }
        out.push_str("/* End ");
        out.push_str(isa);
        out.push_str(" section */\n");
    }
    out.push_str("\t}");
}

/// Reference annotations are suppressed for these keys: their values are
/// object GUIDs, but Xcode writes them bare.
fn suppresses_annotation(key: Option<&str>) -> bool {
    matches!(key, Some("remoteGlobalIDString" | "TestTargetID"))
}

fn write_value(
    out: &mut String,
    value: &Value,
    indent: usize,
    inline: bool,
    comments: &HashMap<String, String>,
    key: Option<&str>,
) {
    match value {
        Value::String(s) => {
            // Xcode quotes `explicitFolders` elements that contain a path
            // separator even though `/` is otherwise quote-free.
            let force_quotes = key == Some("explicitFolders") && s.contains('/');
            write_string(out, s, force_quotes);
            if !suppresses_annotation(key)
                && let Some(c) = comments.get(s.as_str())
            {
                out.push_str(" /* ");
                out.push_str(c);
                out.push_str(" */");
            }
        }
        Value::Array(items) => {
            if inline {
                out.push('(');
                for item in items {
                    write_value(out, item, indent, true, comments, key);
                    out.push_str(", ");
                }
                out.push(')');
            } else {
                out.push_str("(\n");
                for item in items {
                    push_tabs(out, indent + 1);
                    write_value(out, item, indent + 1, false, comments, key);
                    out.push_str(",\n");
                }
                push_tabs(out, indent);
                out.push(')');
            }
        }
        Value::Dict(dict) => {
            if inline {
                out.push('{');
                for (k, v) in dict {
                    write_string(out, k, false);
                    out.push_str(" = ");
                    write_value(out, v, indent, true, comments, Some(k));
                    out.push_str("; ");
                }
                out.push('}');
            } else {
                out.push_str("{\n");
                for (k, v) in dict {
                    push_tabs(out, indent + 1);
                    write_string(out, k, false);
                    out.push_str(" = ");
                    write_value(out, v, indent + 1, false, comments, Some(k));
                    out.push_str(";\n");
                }
                push_tabs(out, indent);
                out.push('}');
            }
        }
    }
}

fn push_tabs(out: &mut String, n: usize) {
    for _ in 0..n {
        out.push('\t');
    }
}

/// Xcode leaves a string unquoted iff it is non-empty, every byte is in
/// `[A-Za-z0-9_./]`, and it contains no comment-opening sequence.
fn needs_quotes(s: &str) -> bool {
    s.is_empty()
        || s.contains("//")
        || s.contains("/*")
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/'))
}

fn write_string(out: &mut String, s: &str, force_quotes: bool) {
    if !force_quotes && !needs_quotes(s) {
        out.push_str(s);
        return;
    }
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
}

// ---------------------------------------------------------------------------
// Annotation regeneration
// ---------------------------------------------------------------------------

/// Compute the `/* … */` annotation for every object GUID. Keyed by GUID;
/// objects with no derivable name (e.g. a path-less `mainGroup`) are absent.
fn build_comments(root: &Value, project_name: &str) -> HashMap<String, String> {
    let Some(objects) = root.get("objects").and_then(Value::as_dict) else {
        return HashMap::new();
    };

    // Reverse maps the per-object rules need: which build phase owns a
    // build file, and which project/target owns a configuration list.
    let mut phase_of: HashMap<&str, &str> = HashMap::new();
    let mut config_list_owner: HashMap<&str, &str> = HashMap::new();
    for (guid, obj) in objects {
        let isa = obj.get("isa").and_then(Value::as_str).unwrap_or("");
        if isa.ends_with("BuildPhase")
            && let Some(files) = obj.get("files").and_then(Value::as_array)
        {
            for f in files {
                if let Some(file_guid) = f.as_str() {
                    phase_of.insert(file_guid, guid);
                }
            }
        }
        if let Some(list) = obj.get("buildConfigurationList").and_then(Value::as_str) {
            config_list_owner.insert(list, guid);
        }
    }

    let ctx = CommentCtx {
        objects,
        phase_of,
        config_list_owner,
        project_name,
    };
    let mut comments = HashMap::new();
    for (guid, _) in objects {
        if let Some(c) = ctx.comment_for(guid, 0) {
            comments.insert(guid.clone(), c);
        }
    }
    comments
}

/// Recursion guard for [`CommentCtx::comment_for`]. A well-formed project
/// needs two hops at most (build file → file ref / phase); a corrupt one
/// whose `fileRef` points back at itself would otherwise recurse forever.
const MAX_COMMENT_DEPTH: usize = 8;

struct CommentCtx<'a> {
    objects: &'a Dict,
    /// build-file GUID → owning build-phase GUID.
    phase_of: HashMap<&'a str, &'a str>,
    /// configuration-list GUID → owning project/target GUID.
    config_list_owner: HashMap<&'a str, &'a str>,
    project_name: &'a str,
}

impl CommentCtx<'_> {
    fn attr<'b>(&'b self, guid: &str, key: &str) -> Option<&'b str> {
        self.objects.get(guid)?.get(key)?.as_str()
    }

    /// Annotation text for an object, derived from its kind. Recursion is
    /// shallow in any well-formed project — a build file names its file
    /// reference and its phase, neither of which recurses further — but a
    /// corrupt `fileRef` can point back at a `PBXBuildFile` (even itself), so
    /// `depth` bounds the walk; past [`MAX_COMMENT_DEPTH`] the reference is
    /// treated like any other failed lookup (`(null)`).
    fn comment_for(&self, guid: &str, depth: usize) -> Option<String> {
        if depth > MAX_COMMENT_DEPTH {
            return None;
        }
        let obj = self.objects.get(guid)?;
        let isa = obj.get("isa").and_then(Value::as_str)?;
        match isa {
            "PBXBuildFile" => {
                let file = obj
                    .get("fileRef")
                    .or_else(|| obj.get("productRef"))
                    .and_then(Value::as_str)
                    .and_then(|r| self.comment_for(r, depth + 1))
                    .unwrap_or_else(|| "(null)".into());
                let phase = self
                    .phase_of
                    .get(guid)
                    .and_then(|p| self.comment_for(p, depth + 1))
                    .unwrap_or_else(|| "(null)".into());
                Some(format!("{file} in {phase}"))
            }
            "PBXFileReference"
            | "PBXReferenceProxy"
            | "PBXGroup"
            | "PBXVariantGroup"
            | "XCVersionGroup"
            | "PBXFileSystemSynchronizedRootGroup" => self.name_or_path_basename(guid),
            "PBXProject" => Some("Project object".into()),
            "PBXNativeTarget" | "PBXAggregateTarget" | "PBXLegacyTarget" => {
                self.attr(guid, "name").map(str::to_string)
            }
            "PBXSourcesBuildPhase" => Some("Sources".into()),
            "PBXFrameworksBuildPhase" => Some("Frameworks".into()),
            "PBXResourcesBuildPhase" => Some("Resources".into()),
            "PBXHeadersBuildPhase" => Some("Headers".into()),
            "PBXRezBuildPhase" => Some("Rez".into()),
            "PBXCopyFilesBuildPhase" => Some(
                self.attr(guid, "name")
                    .map_or_else(|| "CopyFiles".into(), str::to_string),
            ),
            "PBXShellScriptBuildPhase" => Some(
                self.attr(guid, "name")
                    .map_or_else(|| "ShellScript".into(), str::to_string),
            ),
            "PBXContainerItemProxy"
            | "PBXTargetDependency"
            | "PBXBuildRule"
            | "PBXFileSystemSynchronizedBuildFileExceptionSet"
            | "PBXFileSystemSynchronizedGroupBuildPhaseMembershipExceptionSet" => Some(isa.into()),
            "XCBuildConfiguration" => self.attr(guid, "name").map(str::to_string),
            "XCConfigurationList" => {
                let owner = *self.config_list_owner.get(guid)?;
                let owner_isa = self.attr(owner, "isa")?;
                let owner_name = if owner_isa == "PBXProject" {
                    self.project_name
                } else {
                    self.attr(owner, "name")?
                };
                Some(format!(
                    "Build configuration list for {owner_isa} \"{owner_name}\""
                ))
            }
            "XCRemoteSwiftPackageReference" => {
                let url = self.attr(guid, "repositoryURL")?;
                let name = url
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or(url)
                    .trim_end_matches(".git");
                Some(format!("XCRemoteSwiftPackageReference \"{name}\""))
            }
            "XCLocalSwiftPackageReference" => {
                let path = self.attr(guid, "relativePath")?;
                Some(format!("XCLocalSwiftPackageReference \"{path}\""))
            }
            "XCSwiftPackageProductDependency" => self.attr(guid, "productName").map(str::to_string),
            _ => None,
        }
    }

    /// `name`, else the last component of `path`, else nothing (Xcode leaves
    /// e.g. a bare `mainGroup` reference unannotated).
    fn name_or_path_basename(&self, guid: &str) -> Option<String> {
        if let Some(name) = self.attr(guid, "name") {
            return Some(name.to_string());
        }
        let path = self.attr(guid, "path")?;
        Some(path.rsplit('/').next().unwrap_or(path).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbxproj::parse;

    #[test]
    fn round_trips_minimal_project() {
        let src = "// !$*UTF8*$!\n{\n\tarchiveVersion = 1;\n\tclasses = {\n\t};\n\tobjectVersion = 77;\n\tobjects = {\n\n/* Begin PBXProject section */\n\t\tABC /* Project object */ = {\n\t\t\tisa = PBXProject;\n\t\t};\n/* End PBXProject section */\n\t};\n\trootObject = ABC /* Project object */;\n}\n";
        let v = parse(src).unwrap();
        assert_eq!(serialize(&v, "Demo"), src);
    }

    #[test]
    fn quotes_strings_like_xcode() {
        assert!(!needs_quotes("com.apple.product"));
        assert!(!needs_quotes("Base.lproj/Main.storyboard"));
        assert!(!needs_quotes("/bin/sh"));
        assert!(needs_quotes(""));
        assert!(needs_quotes("has space"));
        assert!(needs_quotes("dash-ed"));
        assert!(needs_quotes("$(TARGET_NAME)"));
        assert!(needs_quotes("<group>"));
        assert!(needs_quotes("https://example.com"));
    }

    #[test]
    fn escapes_quoted_strings() {
        let mut out = String::new();
        write_string(&mut out, "a\nb\t\"c\"\\d", false);
        assert_eq!(out, r#""a\nb\t\"c\"\\d""#);
    }
}
