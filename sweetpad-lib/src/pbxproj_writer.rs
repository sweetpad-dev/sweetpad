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
//! Xcode writes annotations in two dialects, and which one a file uses is a
//! property of the Xcode that last wrote it, not of the file's contents. The
//! only durable signal is `objectVersion`: the formats numbered
//! [`DESCRIPTIVE_COMMENTS_MIN_OBJECT_VERSION`] and above did not exist before
//! Xcode 16.3, so a file carrying one was written by an Xcode that spells
//! annotations the long way — a build configuration names the target it
//! configures, a synchronized folder's exception set names the folder and the
//! target, and a reference with no `name` is annotated with its whole `path`
//! rather than the last component. Below that, the short spellings.
//!
//! A newer Xcode asked to save in an older format writes the long spellings
//! into it, so a project at `objectVersion` 77 last touched by Xcode 26.3 or
//! later is annotated one way while every other 77 project is annotated the
//! other, and nothing in the file distinguishes them. The threshold takes the
//! older reading, which is what every such project in the fixture corpus has.
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
    let ctx = WriteCtx {
        descriptive: writes_descriptive_comments(root),
        comments: build_comments(root, project_name),
    };
    let mut out = String::with_capacity(1 << 16);
    out.push_str("// !$*UTF8*$!\n");
    let Some(top) = root.as_dict() else {
        // Not a pbxproj-shaped document; fall back to a bare value.
        write_value(&mut out, root, 0, false, &ctx, None);
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
            write_objects(&mut out, objects, &ctx);
        } else {
            write_value(&mut out, value, 1, false, &ctx, Some(key));
        }
        out.push_str(";\n");
    }
    out.push_str("}\n");
    out
}

/// The lowest `objectVersion` whose format postdates the annotation change.
/// Xcode 16.3 introduced it along with format 90; 26.3 writes 100 and 27.0
/// writes 110, all three sharing the same spellings.
const DESCRIPTIVE_COMMENTS_MIN_OBJECT_VERSION: u32 = 90;

/// Whether this document's annotations use the long spellings. A document with
/// no readable `objectVersion` is treated as old, which is the safer guess for
/// anything hand-written.
fn writes_descriptive_comments(root: &Value) -> bool {
    root.get("objectVersion")
        .and_then(Value::as_str)
        .and_then(|v| v.parse::<u32>().ok())
        .is_some_and(|v| v >= DESCRIPTIVE_COMMENTS_MIN_OBJECT_VERSION)
}

/// What every writing function needs: the annotation for each GUID, and which
/// annotation dialect the document is in.
struct WriteCtx {
    comments: HashMap<String, String>,
    descriptive: bool,
}

/// The `objects` dict: entries grouped into `/* Begin <isa> section */`
/// blocks (sections sorted by isa, entries in source order), each object's
/// key annotated, and single-line bodies replayed from the parser's layout
/// hint.
fn write_objects(out: &mut String, objects: &Dict, ctx: &WriteCtx) {
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
            if let Some(c) = ctx.comments.get(guid.as_str()) {
                out.push_str(" /* ");
                out.push_str(c);
                out.push_str(" */");
            }
            out.push_str(" = ");
            let inline = obj.as_dict().is_some_and(Dict::is_single_line);
            write_value(out, obj, 2, inline, ctx, None);
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
    ctx: &WriteCtx,
    key: Option<&str>,
) {
    match value {
        Value::String(s) => {
            // Xcode quotes `explicitFolders` elements that contain a path
            // separator even though `/` is otherwise quote-free, and, in the
            // formats that spell a shell script as one array element per line,
            // quotes every one of those lines however plain it is.
            let force_quotes = (key == Some("explicitFolders") && s.contains('/'))
                || (ctx.descriptive && key == Some("shellScript"));
            write_string(out, s, force_quotes);
            if !suppresses_annotation(key)
                && let Some(c) = ctx.comments.get(s.as_str())
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
                    write_value(out, item, indent, true, ctx, key);
                    out.push_str(", ");
                }
                out.push(')');
            } else {
                out.push_str("(\n");
                for item in items {
                    push_tabs(out, indent + 1);
                    write_value(out, item, indent + 1, false, ctx, key);
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
                    write_value(out, v, indent, true, ctx, Some(k));
                    out.push_str("; ");
                }
                out.push('}');
            } else {
                out.push_str("{\n");
                for (k, v) in dict {
                    push_tabs(out, indent + 1);
                    write_string(out, k, false);
                    out.push_str(" = ");
                    write_value(out, v, indent + 1, false, ctx, Some(k));
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

    // Reverse maps the per-object rules need: which build phase owns a build
    // file, which project/target owns a configuration list, which list owns a
    // configuration, and which synchronized folder owns an exception set.
    let mut phase_of: HashMap<&str, &str> = HashMap::new();
    let mut config_list_owner: HashMap<&str, &str> = HashMap::new();
    let mut list_of_config: HashMap<&str, &str> = HashMap::new();
    let mut folder_of_exception: HashMap<&str, &str> = HashMap::new();
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
        if let Some(configs) = obj.get("buildConfigurations").and_then(Value::as_array) {
            for c in configs {
                if let Some(config_guid) = c.as_str() {
                    list_of_config.insert(config_guid, guid);
                }
            }
        }
        if let Some(exceptions) = obj.get("exceptions").and_then(Value::as_array) {
            for e in exceptions {
                if let Some(exception_guid) = e.as_str() {
                    folder_of_exception.insert(exception_guid, guid);
                }
            }
        }
    }

    let ctx = CommentCtx {
        objects,
        phase_of,
        config_list_owner,
        list_of_config,
        folder_of_exception,
        project_name,
        descriptive: writes_descriptive_comments(root),
    };
    let mut comments = HashMap::new();
    for (guid, _) in objects {
        // A name containing `*/` would terminate the block comment early and
        // make the written file unparseable (by Xcode too); the annotation is
        // cosmetic, so drop it rather than corrupt the file.
        if let Some(c) = ctx.comment_for(guid, 0)
            && !c.contains("*/")
        {
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
    /// build-configuration GUID → owning configuration-list GUID.
    list_of_config: HashMap<&'a str, &'a str>,
    /// exception-set GUID → owning synchronized-folder GUID.
    folder_of_exception: HashMap<&'a str, &'a str>,
    project_name: &'a str,
    /// See [`writes_descriptive_comments`].
    descriptive: bool,
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
            | "PBXFileSystemSynchronizedRootGroup" => self.name_or_path(guid),
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
            "PBXFileSystemSynchronizedBuildFileExceptionSet"
            | "PBXFileSystemSynchronizedGroupBuildPhaseMembershipExceptionSet"
                if self.descriptive =>
            {
                self.exception_set_comment(guid)
                    .or_else(|| Some(isa.into()))
            }
            "PBXContainerItemProxy"
            | "PBXTargetDependency"
            | "PBXBuildRule"
            | "PBXFileSystemSynchronizedBuildFileExceptionSet"
            | "PBXFileSystemSynchronizedGroupBuildPhaseMembershipExceptionSet" => Some(isa.into()),
            "XCBuildConfiguration" if self.descriptive => self.configuration_comment(guid),
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
                // A URL fragment is not part of the repository's name:
                // `…/swift-markdown-ui#installation` is annotated
                // `swift-markdown-ui`.
                let name = url
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or(url)
                    .split('#')
                    .next()
                    .unwrap_or(url)
                    .trim_end_matches(".git");
                Some(format!("XCRemoteSwiftPackageReference \"{name}\""))
            }
            "XCLocalSwiftPackageReference" => {
                let path = self.attr(guid, "relativePath")?;
                Some(format!("XCLocalSwiftPackageReference \"{path}\""))
            }
            // A build-tool plug-in's product name carries a `plugin:` prefix
            // that the annotation drops.
            "XCSwiftPackageProductDependency" => self
                .attr(guid, "productName")
                .map(|n| n.strip_prefix("plugin:").unwrap_or(n).to_string()),
            _ => None,
        }
    }

    /// `name`, else `path`, else nothing (Xcode leaves e.g. a bare `mainGroup`
    /// reference unannotated). In the older dialect only the last component of
    /// `path` is used, so a reference to `Tests/Unit/FooTests.swift` reads
    /// `FooTests.swift` there and in full in the newer one.
    fn name_or_path(&self, guid: &str) -> Option<String> {
        if let Some(name) = self.attr(guid, "name") {
            return Some(name.to_string());
        }
        let path = self.attr(guid, "path")?;
        if self.descriptive {
            return Some(path.to_string());
        }
        Some(path.rsplit('/').next().unwrap_or(path).to_string())
    }

    /// `"Debug configuration for PBXNativeTarget \"MyApp\""` — the newer
    /// dialect's build-configuration annotation, which names the object whose
    /// configuration list holds it. Falls back to the bare configuration name
    /// when the configuration is orphaned.
    fn configuration_comment(&self, guid: &str) -> Option<String> {
        let name = self.attr(guid, "name")?;
        let Some(owner) = self
            .list_of_config
            .get(guid)
            .and_then(|list| self.config_list_owner.get(list))
        else {
            return Some(name.to_string());
        };
        let Some(owner_isa) = self.attr(owner, "isa") else {
            return Some(name.to_string());
        };
        let owner_name = if owner_isa == "PBXProject" {
            self.project_name
        } else {
            self.attr(owner, "name")?
        };
        Some(format!(
            "{name} configuration for {owner_isa} \"{owner_name}\""
        ))
    }

    /// `"Exceptions for \"Shared\" folder in \"MyApp\" target"` — the newer
    /// dialect's annotation for a synchronized folder's exception set. The
    /// folder is spelled exactly as its own annotation spells it.
    fn exception_set_comment(&self, guid: &str) -> Option<String> {
        let folder = self
            .folder_of_exception
            .get(guid)
            .and_then(|f| self.name_or_path(f))?;
        let target = self
            .attr(guid, "target")
            .and_then(|t| self.attr(t, "name"))?;
        Some(format!(
            "Exceptions for \"{folder}\" folder in \"{target}\" target"
        ))
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

    /// One project in both dialects: the same objects, only `objectVersion`
    /// differing. `{ov}` is substituted per call.
    fn dialect_sample(object_version: u32) -> String {
        format!(
            "// !$*UTF8*$!\n{{\n\tarchiveVersion = 1;\n\tclasses = {{\n\t}};\n\tobjectVersion = {object_version};\n\tobjects = {{\n\
\n/* Begin PBXFileReference section */\n\
\t\tF1 = {{isa = PBXFileReference; path = \"Tests/Unit/FooTests.swift\"; sourceTree = SOURCE_ROOT; }};\n\
/* End PBXFileReference section */\n\
\n/* Begin PBXFileSystemSynchronizedBuildFileExceptionSet section */\n\
\t\tE1 = {{isa = PBXFileSystemSynchronizedBuildFileExceptionSet; target = T1; }};\n\
/* End PBXFileSystemSynchronizedBuildFileExceptionSet section */\n\
\n/* Begin PBXFileSystemSynchronizedRootGroup section */\n\
\t\tS1 = {{isa = PBXFileSystemSynchronizedRootGroup; exceptions = (E1, ); path = Shared; sourceTree = \"<group>\"; }};\n\
/* End PBXFileSystemSynchronizedRootGroup section */\n\
\n/* Begin PBXNativeTarget section */\n\
\t\tT1 = {{isa = PBXNativeTarget; buildConfigurationList = L1; name = MyApp; }};\n\
/* End PBXNativeTarget section */\n\
\n/* Begin PBXShellScriptBuildPhase section */\n\
\t\tP1 = {{\n\t\t\tisa = PBXShellScriptBuildPhase;\n\t\t\tname = Stamp;\n\t\t\tshellScript = (\n\t\t\t\tfi,\n\t\t\t);\n\t\t}};\n\
/* End PBXShellScriptBuildPhase section */\n\
\n/* Begin XCBuildConfiguration section */\n\
\t\tC1 = {{isa = XCBuildConfiguration; name = Debug; }};\n\
/* End XCBuildConfiguration section */\n\
\n/* Begin XCConfigurationList section */\n\
\t\tL1 = {{isa = XCConfigurationList; buildConfigurations = (C1, ); }};\n\
/* End XCConfigurationList section */\n\
\t}};\n\trootObject = R1;\n}}\n"
        )
    }

    #[test]
    fn the_older_dialect_annotates_briefly() {
        let out = serialize(&parse(&dialect_sample(77)).unwrap(), "Demo");

        assert!(out.contains("C1 /* Debug */"), "{out}");
        assert!(
            out.contains("E1 /* PBXFileSystemSynchronizedBuildFileExceptionSet */"),
            "{out}"
        );
        assert!(out.contains("F1 /* FooTests.swift */"), "{out}");
        assert!(out.contains("\t\t\t\tfi,\n"), "{out}");
    }

    #[test]
    fn the_newer_dialect_names_what_each_object_belongs_to() {
        let out = serialize(&parse(&dialect_sample(110)).unwrap(), "Demo");

        assert!(
            out.contains("C1 /* Debug configuration for PBXNativeTarget \"MyApp\" */"),
            "{out}"
        );
        assert!(
            out.contains("E1 /* Exceptions for \"Shared\" folder in \"MyApp\" target */"),
            "{out}"
        );
        assert!(out.contains("F1 /* Tests/Unit/FooTests.swift */"), "{out}");
        assert!(out.contains("\t\t\t\t\"fi\",\n"), "{out}");
    }

    /// The threshold sits between the formats Xcode 16.0 and 16.3 write.
    #[test]
    fn the_dialect_turns_over_at_object_version_90() {
        for (version, descriptive) in [(77, false), (89, false), (90, true), (110, true)] {
            let root = parse(&dialect_sample(version)).unwrap();
            assert_eq!(
                writes_descriptive_comments(&root),
                descriptive,
                "objectVersion {version}"
            );
        }
    }

    /// A document with nothing to read the version from is treated as old.
    #[test]
    fn a_versionless_document_takes_the_older_dialect() {
        let root = parse("// !$*UTF8*$!\n{\n\tobjects = {\n\t};\n}\n").unwrap();
        assert!(!writes_descriptive_comments(&root));
    }

    /// Two spellings Xcode drops from an annotation, in both dialects.
    #[test]
    fn package_annotations_drop_the_plugin_prefix_and_the_url_fragment() {
        let src = "// !$*UTF8*$!\n{\n\tobjectVersion = 56;\n\tobjects = {\n\
\n/* Begin XCRemoteSwiftPackageReference section */\n\
\t\tK1 = {isa = XCRemoteSwiftPackageReference; repositoryURL = \"https://github.com/gonzalezreal/swift-markdown-ui#installation\"; };\n\
/* End XCRemoteSwiftPackageReference section */\n\
\n/* Begin XCSwiftPackageProductDependency section */\n\
\t\tD1 = {isa = XCSwiftPackageProductDependency; productName = \"plugin:SwiftLintPlugin\"; };\n\
/* End XCSwiftPackageProductDependency section */\n\
\t};\n}\n";
        let out = serialize(&parse(src).unwrap(), "Demo");

        assert!(
            out.contains("K1 /* XCRemoteSwiftPackageReference \"swift-markdown-ui\" */"),
            "{out}"
        );
        assert!(out.contains("D1 /* SwiftLintPlugin */"), "{out}");
    }

    #[test]
    fn escapes_quoted_strings() {
        let mut out = String::new();
        write_string(&mut out, "a\nb\t\"c\"\\d", false);
        assert_eq!(out, r#""a\nb\t\"c\"\\d""#);
    }
}
