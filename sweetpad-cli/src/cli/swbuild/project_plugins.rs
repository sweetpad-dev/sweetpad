//! Present each selected Xcode target to SwiftPM as a local package target.
//! The manifest and scratch files live under DerivedData. Project files stay
//! in place and the host tools still build in the workspace's package graph.
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Component, Path, PathBuf};
use sweetpad_lib::destination::Platform;
use sweetpad_lib::workspace_pif::Package;

use serde_json::Value;
use sweetpad_lib::workspace_pif::project_plugins::{self, PluginTarget};

use super::{CliError, Invocation, Output, SchemeTarget, scheme_pif_target, write_changed};

pub(super) fn prepare(
    invocation: &Invocation,
    pif: &mut [Value],
    selected: &[SchemeTarget],
    packages: &[Package],
    out: &Output,
) -> Result<(), CliError> {
    let roots = selected
        .iter()
        .filter_map(|target| Some(scheme_pif_target(pif, target)?["guid"].as_str()?.to_owned()))
        .collect::<Vec<_>>();
    let platform = invocation.parameters["activeRunDestination"]["platform"]
        .as_str()
        .and_then(Platform::from_sdk);
    for target in project_plugins::targets(pif, &roots, platform) {
        let root = invocation
            .locations
            .intermediates
            .join("SweetPad/ProjectPlugins")
            .join(&target.guid);
        let scratch = invocation.locations.folder.join("SourcePackages");
        let manifest = inputs(&target, packages, &root).map_err(|e| {
            CliError::new(format!(
                "preparing inputs for plugins on '{}': {e}",
                target.name
            ))
        })?;
        write_changed(&root.join("Package.swift"), manifest.as_bytes())?;
        if let Ok(pins) = fs::read(invocation.packages.join("Package.resolved")) {
            write_changed(&root.join("Package.resolved"), &pins)?;
        }
        let dumped = invocation.dump_pif_in(&root, &scratch, out).map_err(|e| {
            CliError::new(format!(
                "evaluating build-tool plugins on '{}': {e}",
                target.name
            ))
        })?;
        project_plugins::attach(
            pif,
            &target,
            &dumped,
            &root,
            &scratch.join("plugins/outputs"),
        )
        .map_err(CliError::new)?;
    }
    Ok(())
}

/// Link the project tree so plugins can read their normal configuration files.
/// Only members of this target enter the proxy target's manifest.
fn inputs(target: &PluginTarget, packages: &[Package], root: &Path) -> std::io::Result<String> {
    const EXTRA: &str = "SweetPadPluginInputs";
    fs::create_dir_all(root)?;
    let mut entries = BTreeSet::new();
    for entry in fs::read_dir(&target.directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // SwiftPM owns these names in the generated package.
        if matches!(
            name.as_str(),
            "Package.swift" | "Package.resolved" | ".build" | ".swiftpm" | ".git" | EXTRA
        ) || name.starts_with("Package@swift-")
            || root.starts_with(entry.path())
        {
            continue;
        }
        link(&entry.path(), &root.join(&name))?;
        entries.insert(name);
    }
    // Reusing DerivedData for another checkout must not expose files that
    // were removed from the project. SwiftPM owns the .build link.
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        if entry.file_type()?.is_symlink()
            && name != ".build"
            && !entries.contains(name.to_string_lossy().as_ref())
        {
            fs::remove_file(entry.path())?;
        }
    }
    let mut used = BTreeSet::new();
    let relative = |(index, file): (usize, &PathBuf)| -> std::io::Result<String> {
        if let Ok(path) = file.strip_prefix(&target.directory)
            && path.components().all(|c| matches!(c, Component::Normal(_)))
            && path
                .iter()
                .next()
                .is_some_and(|n| entries.contains(n.to_string_lossy().as_ref()))
        {
            return Ok(path.to_string_lossy().into_owned());
        }
        let folder = Path::new(EXTRA).join(index.to_string());
        fs::create_dir_all(root.join(&folder))?;
        let path = folder.join(file.file_name().ok_or(std::io::ErrorKind::InvalidInput)?);
        link(file, &root.join(&path))?;
        Ok(path.to_string_lossy().into_owned())
    };
    let paths = target
        .sources
        .iter()
        .chain(&target.resources)
        .enumerate()
        .map(relative)
        .collect::<Result<Vec<_>, _>>()?;
    let (sources, resources) = paths.split_at(target.sources.len());
    for path in &paths {
        if let Some(top) = path.split('/').next() {
            used.insert(top.to_owned());
        }
    }
    let exclude = entries.difference(&used).cloned().collect::<Vec<_>>();
    let mut sources = sources.to_vec();
    if sources.is_empty() {
        fs::create_dir_all(root.join(EXTRA))?;
        let placeholder = format!("{EXTRA}/Empty.swift");
        if !root.join(&placeholder).exists() {
            fs::write(root.join(&placeholder), "")?;
        }
        sources.push(placeholder);
    }
    // A SwiftPM target cannot mix Swift and Clang sources. Its plugin still
    // receives the non-Swift inputs through the target's file list.
    let mut resources = resources.to_vec();
    let swift = |s: &String| {
        Path::new(s)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("swift"))
    };
    if sources.iter().any(swift) {
        sources.retain(|s| {
            if swift(s) {
                true
            } else {
                resources.push(s.clone());
                false
            }
        });
    }
    Ok(target.manifest(packages, &sources, &resources, &exclude))
}

fn link(source: &Path, dest: &Path) -> std::io::Result<()> {
    if let Ok(old) = fs::read_link(dest) {
        if old == source {
            return Ok(());
        }
        fs::remove_file(dest)?;
    }
    symlink(source, dest)
}
