//! Swift Build's project interchange format (PIF) for an Xcode project.
//!
//! Swift Build (`swiftlang/swift-build`) never reads a project file. Xcode
//! translates the project into PIF before handing it over, and the open-source
//! engine's own `.xcodeproj` support shells out to `xcodebuild -dumpPIF` for
//! that translation. Emitting PIF here is what lets the engine build an
//! unmodified project on a host without Xcode. `xcodebuild -dumpPIF` is the
//! oracle: GUIDs, key spellings and value shapes follow what it prints, and
//! Swift Build's `PIFLoader` decides which keys are required.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value as Json, json};

use crate::pbxproj::{Dict, Value};
use crate::project::{self, Error};
use crate::xcode_hash::md5_hex;

/// The PIF `xcodebuild -dumpPIF -project` prints for one project: a workspace
/// object wrapping it, the project, then one object per target.
pub fn project_pif(value: &Value, xcodeproj_path: &Path) -> Result<Vec<Json>, Error> {
    let xcodeproj =
        fs::canonicalize(xcodeproj_path).unwrap_or_else(|_| xcodeproj_path.to_path_buf());
    let (objects, project_obj) = project::project_root(value)?;
    let emitter = Emitter {
        objects,
        project_guid: md5_hex(xcodeproj.to_string_lossy().as_bytes()),
    };

    let mut targets = Vec::new();
    for id in ids(project_obj, "targets") {
        if let Some(target) = emitter.target(id)? {
            targets.push(target);
        }
    }
    let project = emitter.project(project_obj, &xcodeproj, &targets)?;

    let workspace_path = xcodeproj.join("project.xcworkspace");
    let workspace = object(
        "workspace",
        json!({
            "guid": md5_hex(workspace_path.to_string_lossy().as_bytes()),
            "name": stem(&xcodeproj),
            "path": workspace_path,
            "projects": [project["signature"]],
        }),
    );

    let mut out = vec![workspace, project];
    out.extend(targets);
    Ok(out)
}

struct Emitter<'a> {
    objects: &'a Dict,
    project_guid: String,
}

impl Emitter<'_> {
    /// Xcode's GUID for a pbxproj object: the project's GUID followed by the
    /// MD5 of the object's ID, so a GUID is stable across dumps and unique
    /// across the projects of a workspace.
    fn guid(&self, id: &str) -> String {
        format!("{}{}", self.project_guid, md5_hex(id.as_bytes()))
    }

    fn get(&self, id: &str) -> Option<&Dict> {
        self.objects.get(id).and_then(Value::as_dict)
    }

    fn project(
        &self,
        project_obj: &Value,
        xcodeproj: &Path,
        targets: &[Json],
    ) -> Result<Json, Error> {
        let main_group = project_obj
            .get("mainGroup")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::BadProject("project has no mainGroup".into()))?;
        let products = self.product_references();
        let mut group_tree = self
            .reference(main_group, &products)
            .ok_or_else(|| Error::BadProject(format!("mainGroup {main_group} is not a group")))?;
        if self
            .get(main_group)
            .is_some_and(|g| g.get("name").is_none())
        {
            group_tree["name"] = json!(stem(xcodeproj));
        }

        let config_list = self.configuration_list(project_obj);
        let project_dir = project_directory(project_obj, xcodeproj);
        let contents = json!({
            "appPreferencesBuildSettings": {},
            "buildConfigurations": self.configurations(project_obj)?,
            "classPrefix": str_at(project_obj, "attributes", "CLASSPREFIX").unwrap_or_default(),
            "defaultConfigurationName": config_list
                .and_then(|l| l.get("defaultConfigurationName"))
                .and_then(Value::as_str)
                .unwrap_or("Release"),
            "developmentRegion": project_obj.get("developmentRegion").and_then(Value::as_str).unwrap_or("en"),
            "groupTree": group_tree,
            "guid": self.project_guid,
            "knownRegions": strings(project_obj.get("knownRegions")),
            "path": xcodeproj,
            "projectDirectory": project_dir,
            "targets": targets.iter().map(|t| t["signature"].clone()).collect::<Vec<_>>(),
        });
        Ok(object("project", contents))
    }

    /// Every file reference a target names as its product. Xcode keeps these
    /// out of the PIF group tree; the target's `productReference` carries them.
    fn product_references(&self) -> HashSet<&str> {
        self.objects
            .values()
            .filter_map(|o| o.get("productReference").and_then(Value::as_str))
            .collect()
    }

    fn reference(&self, id: &str, products: &HashSet<&str>) -> Option<Json> {
        let obj = self.get(id)?;
        let isa = obj.get("isa").and_then(Value::as_str)?;
        let source_tree = obj
            .get("sourceTree")
            .and_then(Value::as_str)
            .unwrap_or("<group>");
        let path = obj.get("path").and_then(Value::as_str);
        let name = obj.get("name").and_then(Value::as_str);
        let mut out = Map::new();
        out.insert("guid".into(), json!(self.guid(id)));
        out.insert("sourceTree".into(), json!(source_tree));
        match isa {
            "PBXFileReference" => {
                let file_type = obj
                    .get("explicitFileType")
                    .or_else(|| obj.get("lastKnownFileType"))
                    .and_then(Value::as_str)
                    .unwrap_or("file");
                out.insert("type".into(), json!("file"));
                out.insert("fileType".into(), json!(file_type));
                out.insert("path".into(), json!(path.or(name).unwrap_or_default()));
                if let Some(name) = name.filter(|n| Some(*n) != path) {
                    out.insert("name".into(), json!(name));
                }
                if let Some(encoding) = obj.get("fileEncoding").and_then(Value::as_str) {
                    out.insert("fileTextEncoding".into(), json!(encoding));
                }
            }
            "PBXGroup" | "PBXVariantGroup" | "XCVersionGroup" => {
                let kind = match isa {
                    "PBXGroup" => "group",
                    "PBXVariantGroup" => "variantGroup",
                    _ => "versionGroup",
                };
                out.insert("type".into(), json!(kind));
                out.insert("name".into(), json!(name.or(path).unwrap_or_default()));
                out.insert("path".into(), json!(path.unwrap_or_default()));
                let children: Vec<Json> = ids_in(obj, "children")
                    .filter(|child| !products.contains(child))
                    .filter_map(|child| self.reference(child, products))
                    .collect();
                if !children.is_empty() {
                    out.insert("children".into(), Json::Array(children));
                }
            }
            _ => return None,
        }
        Some(Json::Object(out))
    }

    fn configuration_list(&self, owner: &Value) -> Option<&Dict> {
        owner
            .get("buildConfigurationList")
            .and_then(Value::as_str)
            .and_then(|id| self.get(id))
    }

    fn configurations(&self, owner: &Value) -> Result<Json, Error> {
        let list = self
            .configuration_list(owner)
            .ok_or_else(|| Error::BadProject("object has no buildConfigurationList".into()))?;
        let mut out = Vec::new();
        for id in ids_in(list, "buildConfigurations") {
            let Some(config) = self.get(id) else { continue };
            let settings: Map<String, Json> = config
                .get("buildSettings")
                .and_then(Value::as_dict)
                .map(|d| {
                    d.iter()
                        .map(|(k, v)| (k.clone(), json!(setting_string(v))))
                        .collect()
                })
                .unwrap_or_default();
            let mut entry = json!({
                "buildSettings": settings,
                "guid": self.guid(id),
                "name": config.get("name").and_then(Value::as_str).unwrap_or_default(),
            });
            if let Some(base) = config
                .get("baseConfigurationReference")
                .and_then(Value::as_str)
            {
                entry["baseConfigurationFileReference"] = json!(self.guid(base));
            }
            out.push(entry);
        }
        Ok(Json::Array(out))
    }

    fn target(&self, id: &str) -> Result<Option<Json>, Error> {
        let Some(obj) = self.objects.get(id) else {
            return Ok(None);
        };
        let isa = obj.get("isa").and_then(Value::as_str).unwrap_or_default();
        let name = obj.get("name").and_then(Value::as_str).unwrap_or_default();
        let mut c = Map::new();
        c.insert("guid".into(), json!(self.guid(id)));
        c.insert("name".into(), json!(name));
        c.insert("buildConfigurations".into(), self.configurations(obj)?);
        c.insert("dependencies".into(), self.dependencies(obj));
        match isa {
            "PBXNativeTarget" => {
                c.insert("type".into(), json!("standard"));
                c.insert("buildPhases".into(), self.build_phases(obj));
                c.insert("buildRules".into(), json!([]));
                c.insert(
                    "productTypeIdentifier".into(),
                    json!(
                        obj.get("productType")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                    ),
                );
                if let Some(product) = obj.get("productReference").and_then(Value::as_str) {
                    let product_name = self
                        .get(product)
                        .and_then(|p| p.get("path").or_else(|| p.get("name")))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    c.insert(
                        "productReference".into(),
                        json!({ "guid": self.guid(product), "name": product_name, "type": "product" }),
                    );
                }
                if let Some(language) = self.predominant_language(obj) {
                    c.insert("predominantSourceCodeLanguage".into(), json!(language));
                }
                c.insert("provisioningSourceData".into(), self.provisioning(obj));
            }
            "PBXAggregateTarget" => {
                c.insert("type".into(), json!("aggregate"));
                c.insert("buildPhases".into(), self.build_phases(obj));
            }
            "PBXLegacyTarget" => {
                c.insert("type".into(), json!("external"));
                c.insert("toolPath".into(), json!(str_of(obj, "buildToolPath")));
                c.insert(
                    "arguments".into(),
                    json!(str_of(obj, "buildArgumentsString")),
                );
                c.insert(
                    "workingDirectory".into(),
                    json!(str_of(obj, "buildWorkingDirectory")),
                );
                c.insert(
                    "passBuildSettingsInEnvironment".into(),
                    json!(pif_bool(
                        obj.get("passBuildSettingsInEnvironment")
                            .and_then(Value::as_str)
                            != Some("0")
                    )),
                );
            }
            _ => return Ok(None),
        }
        let contents = Json::Object(c);
        Ok(Some(object("target", contents)))
    }

    fn dependencies(&self, target: &Value) -> Json {
        let mut out = Vec::new();
        for id in ids(target, "dependencies") {
            let Some(dep) = self.get(id) else { continue };
            let Some(target_id) = dep.get("target").and_then(Value::as_str) else {
                continue;
            };
            let mut entry = json!({ "guid": self.guid(target_id) });
            if let Some(name) = self
                .get(target_id)
                .and_then(|t| t.get("name"))
                .and_then(Value::as_str)
            {
                entry["name"] = json!(name);
            }
            out.push(entry);
        }
        Json::Array(out)
    }

    fn build_phases(&self, target: &Value) -> Json {
        Json::Array(
            ids(target, "buildPhases")
                .filter_map(|id| self.build_phase(id))
                .collect(),
        )
    }

    fn build_phase(&self, id: &str) -> Option<Json> {
        let phase = self.get(id)?;
        let kind = match phase.get("isa").and_then(Value::as_str)? {
            "PBXSourcesBuildPhase" => "sources",
            "PBXFrameworksBuildPhase" => "frameworks",
            "PBXHeadersBuildPhase" => "headers",
            "PBXResourcesBuildPhase" => "resources",
            "PBXCopyFilesBuildPhase" => "copy-files",
            "PBXShellScriptBuildPhase" => "shell-script",
            "PBXRezBuildPhase" => "rez",
            _ => return None,
        };
        let mut out = Map::new();
        out.insert("guid".into(), json!(self.guid(id)));
        out.insert("type".into(), json!(format!("com.apple.buildphase.{kind}")));
        out.insert(
            "buildFiles".into(),
            Json::Array(
                ids_in(phase, "files")
                    .filter_map(|f| self.build_file(f))
                    .collect(),
            ),
        );
        match kind {
            "copy-files" => copy_files_keys(phase, &mut out),
            "shell-script" => shell_script_keys(phase, id, &mut out),
            _ => {}
        }
        Some(Json::Object(out))
    }

    fn build_file(&self, id: &str) -> Option<Json> {
        let file = self.get(id)?;
        let mut out = Map::new();
        out.insert("guid".into(), json!(self.guid(id)));
        if let Some(file_ref) = file.get("fileRef").and_then(Value::as_str) {
            out.insert("fileReference".into(), json!(self.guid(file_ref)));
        } else if let Some(product) = file.get("productRef").and_then(Value::as_str) {
            out.insert("targetReference".into(), json!(self.guid(product)));
        } else {
            return None;
        }
        let settings = file.get("settings").and_then(Value::as_dict);
        let attributes = strings(settings.and_then(|s| s.get("ATTRIBUTES")));
        for attribute in &attributes {
            match attribute.as_str() {
                "Public" | "Private" | "Project" => {
                    out.insert("headerVisibility".into(), json!(attribute.to_lowercase()));
                }
                "Weak" => {
                    out.insert("shouldLinkWeakly".into(), json!("true"));
                }
                "CodeSignOnCopy" => {
                    out.insert("codeSignOnCopy".into(), json!("true"));
                }
                "RemoveHeadersOnCopy" => {
                    out.insert("removeHeadersOnCopy".into(), json!("true"));
                }
                _ => {}
            }
        }
        if let Some(flags) = settings
            .and_then(|s| s.get("COMPILER_FLAGS"))
            .and_then(Value::as_str)
        {
            out.insert("additionalCompilerOptions".into(), json!(flags));
        }
        Some(Json::Object(out))
    }

    /// Xcode's guess at a target's main language, from the file types its
    /// Sources phase compiles.
    fn predominant_language(&self, target: &Value) -> Option<&'static str> {
        let mut swift = 0usize;
        let mut objc = 0usize;
        let mut c_family = 0usize;
        for phase in ids(target, "buildPhases").filter_map(|id| self.get(id)) {
            if phase.get("isa").and_then(Value::as_str) != Some("PBXSourcesBuildPhase") {
                continue;
            }
            for file_ref in ids_in(phase, "files")
                .filter_map(|f| self.get(f))
                .filter_map(|f| f.get("fileRef").and_then(Value::as_str))
                .filter_map(|r| self.get(r))
            {
                match file_ref
                    .get("explicitFileType")
                    .or_else(|| file_ref.get("lastKnownFileType"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                {
                    "sourcecode.swift" => swift += 1,
                    "sourcecode.c.objc" | "sourcecode.cpp.objcpp" => objc += 1,
                    "sourcecode.c.c" | "sourcecode.cpp.cpp" => c_family += 1,
                    _ => {}
                }
            }
        }
        if swift == 0 && objc == 0 && c_family == 0 {
            None
        } else if swift >= objc && swift >= c_family {
            Some("Xcode.SourceCodeLanguage.Swift")
        } else if objc >= c_family {
            Some("Xcode.SourceCodeLanguage.Objective-C")
        } else {
            Some("Xcode.SourceCodeLanguage.C")
        }
    }

    fn provisioning(&self, target: &Value) -> Json {
        let Some(list) = self.configuration_list(target) else {
            return json!([]);
        };
        let entries: Vec<Json> = ids_in(list, "buildConfigurations")
            .filter_map(|id| self.get(id))
            .map(|config| {
                let style = match config
                    .get("buildSettings")
                    .and_then(|s| s.get("CODE_SIGN_STYLE"))
                    .and_then(Value::as_str)
                {
                    Some("Manual") => 1,
                    _ => 0,
                };
                json!({
                    "bundleIdentifierFromInfoPlist": "$(PRODUCT_BUNDLE_IDENTIFIER)",
                    "configurationName": config.get("name").and_then(Value::as_str).unwrap_or_default(),
                    "provisioningStyle": style,
                })
            })
            .collect();
        Json::Array(entries)
    }
}

/// A PIF top-level object. The signature is the content hash: Swift Build
/// caches loaded objects by signature, so it must change whenever the
/// contents do.
fn object(kind: &str, contents: Json) -> Json {
    let hash = md5_hex(contents.to_string().as_bytes());
    let mut out = Map::new();
    out.insert(
        "signature".into(),
        json!(format!("{}@v11_hash={hash}", kind.to_uppercase())),
    );
    out.insert("type".into(), json!(kind));
    out.insert("contents".into(), contents);
    Json::Object(out)
}

/// The keys a copy-files phase adds: where its files land.
fn copy_files_keys(phase: &Dict, out: &mut Map<String, Json>) {
    let spec = phase
        .get("dstSubfolderSpec")
        .and_then(Value::as_str)
        .unwrap_or("16");
    out.insert("destinationSubfolder".into(), json!(copy_destination(spec)));
    out.insert(
        "destinationSubpath".into(),
        json!(str_of_dict(phase, "dstPath")),
    );
    out.insert(
        "runOnlyForDeploymentPostprocessing".into(),
        json!(flag(phase, "runOnlyForDeploymentPostprocessing", false)),
    );
}

/// The keys a run-script phase adds: the script, its shell, and the inputs
/// and outputs dependency analysis reads.
fn shell_script_keys(phase: &Dict, id: &str, out: &mut Map<String, Json>) {
    let text =
        |key: &str, default: &str| json!(phase.get(key).and_then(Value::as_str).unwrap_or(default));
    out.insert("name".into(), text("name", "Run Script"));
    out.insert("shellPath".into(), text("shellPath", "/bin/sh"));
    out.insert("scriptContents".into(), text("shellScript", ""));
    out.insert("originalObjectID".into(), json!(id));
    for (key, pif_key) in [
        ("inputPaths", "inputFilePaths"),
        ("outputPaths", "outputFilePaths"),
        ("inputFileListPaths", "inputFileListPaths"),
        ("outputFileListPaths", "outputFileListPaths"),
    ] {
        out.insert(pif_key.into(), json!(strings(phase.get(key))));
    }
    out.insert(
        "emitEnvironment".into(),
        json!(flag(phase, "showEnvVarsInLog", true)),
    );
    out.insert(
        "alwaysOutOfDate".into(),
        json!(flag(phase, "alwaysOutOfDate", false)),
    );
    out.insert(
        "runOnlyForDeploymentPostprocessing".into(),
        json!(flag(phase, "runOnlyForDeploymentPostprocessing", false)),
    );
    if let Some(file) = phase.get("dependencyFile").and_then(Value::as_str) {
        out.insert("dependencyFileFormat".into(), json!("makefile"));
        out.insert("dependencyFilePaths".into(), json!([file]));
    }
}

/// A pbxproj `0`/`1` flag as PIF's `"false"`/`"true"`, `default` when absent.
fn flag(obj: &Dict, key: &str, default: bool) -> &'static str {
    let value = match obj.get(key).and_then(Value::as_str) {
        Some("1") => true,
        Some("0") => false,
        _ => default,
    };
    pif_bool(value)
}

/// The directory `<group>` and `SOURCE_ROOT` paths resolve against:
/// the `.xcodeproj`'s parent, moved by the project's `projectDirPath`.
fn project_directory(project_obj: &Value, xcodeproj: &Path) -> PathBuf {
    let parent = xcodeproj
        .parent()
        .map_or_else(PathBuf::new, Path::to_path_buf);
    match project_obj.get("projectDirPath").and_then(Value::as_str) {
        Some(dir) if !dir.is_empty() => project::standardize(&parent.join(dir)),
        _ => parent,
    }
}

/// A pbxproj setting as the single string PIF stores. A list setting joins
/// with spaces, quoting an element that has whitespace of its own so the
/// value still splits back into the same words.
fn setting_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(|item| {
                if item.contains(char::is_whitespace) && !item.starts_with('"') {
                    format!("\"{item}\"")
                } else {
                    item.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        Value::Dict(_) => String::new(),
    }
}

/// PIF's spelling of a copy-files phase's `dstSubfolderSpec`.
fn copy_destination(spec: &str) -> &'static str {
    match spec {
        "0" => "<absolute>",
        "1" => "$(WRAPPER_NAME)",
        "6" => "$(EXECUTABLE_FOLDER_PATH)",
        "7" => "$(UNLOCALIZED_RESOURCES_FOLDER_PATH)",
        "10" => "$(FRAMEWORKS_FOLDER_PATH)",
        "11" => "$(SHARED_FRAMEWORKS_FOLDER_PATH)",
        "12" => "$(SHARED_SUPPORT_FOLDER_PATH)",
        "13" => "$(PLUGINS_FOLDER_PATH)",
        "15" => "$(JAVA_FOLDER_PATH)",
        _ => "<builtProductsDir>",
    }
}

fn pif_bool(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn ids<'a>(obj: &'a Value, key: &str) -> impl Iterator<Item = &'a str> {
    obj.get(key)
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
}

fn ids_in<'a>(obj: &'a Dict, key: &str) -> impl Iterator<Item = &'a str> {
    obj.get(key)
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn str_of(obj: &Value, key: &str) -> String {
    obj.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn str_of_dict(obj: &Dict, key: &str) -> String {
    obj.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn str_at(obj: &Value, outer: &str, inner: &str) -> Option<String> {
    obj.get(outer)
        .and_then(|o| o.get(inner))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}
