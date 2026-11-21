//! Where a bundle keeps what its signature binds. An iOS-style bundle keeps
//! everything at its top; a macOS one under `Contents`, its executable in
//! `MacOS`; a versioned framework in the version `Versions/Current` points
//! at, its Info.plist in `Resources`.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use super::plist::{self, Plist};

/// Which of `codesign`'s resource rules a bundle is sealed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Flavor {
    /// An iOS-style bundle: every file a resource, nested bundles too.
    Shallow,
    /// A macOS bundle: resources in `Resources`, code where code goes,
    /// sealed as nested code.
    Mac,
}

pub(super) struct Layout {
    /// The folder `_CodeSignature` goes in, which the sealed paths are
    /// relative to.
    pub root: PathBuf,
    pub info_bytes: Vec<u8>,
    pub info: Plist,
    pub info_path: PathBuf,
    /// The executable's path relative to `root`, when it has one.
    pub executable: Option<String>,
    pub flavor: Flavor,
}

/// The file whose bytes form a bundle's code slots.
pub(super) struct BundleCode<'a> {
    pub name: &'a str,
    pub bytes: Cow<'a, [u8]>,
    pub detached: bool,
}

impl Layout {
    pub fn of(bundle: &Path) -> Result<Self, String> {
        let contents = bundle.join("Contents");
        let current = bundle.join("Versions/Current");
        let (root, info_path, folder, flavor) = if contents.is_dir() {
            let info = contents.join("Info.plist");
            (contents, info, "MacOS/", Flavor::Mac)
        } else if current.exists() {
            // `Versions/Current` names the version; sign that, not the link.
            let version = std::fs::read_link(&current).map_or_else(
                |_| current.clone(),
                |target| bundle.join("Versions").join(target),
            );
            let info = version.join("Resources/Info.plist");
            (version, info, "", Flavor::Mac)
        } else if !bundle.join("Info.plist").is_file()
            && bundle.join("Resources/Info.plist").is_file()
        {
            // A framework's version folder, which Xcode signs itself.
            let info = bundle.join("Resources/Info.plist");
            (bundle.to_path_buf(), info, "", Flavor::Mac)
        } else {
            let info = bundle.join("Info.plist");
            (bundle.to_path_buf(), info, "", Flavor::Shallow)
        };
        let info_bytes = std::fs::read(&info_path)
            .map_err(|e| format!("couldn't read {}: {e}", info_path.display()))?;
        let info = plist::read_info(&info_bytes)?;
        let executable =
            plist::string(&info, "CFBundleExecutable").map(|name| format!("{folder}{name}"));
        Ok(Self {
            root,
            info_bytes,
            info,
            info_path,
            executable,
            flavor,
        })
    }

    pub fn code(&self, bundle: &Path) -> Result<BundleCode<'_>, String> {
        let resource_bundle = bundle
            .extension()
            .is_some_and(|s| s == "bundle" || s == "xctest")
            && plist::string(&self.info, "CFBundlePackageType") == Some("BNDL");
        if let Some(name) = &self.executable {
            let path = self.root.join(name);
            match std::fs::read(&path) {
                Ok(bytes) => {
                    return Ok(BundleCode {
                        name,
                        detached: crate::cli::darwin_tools::is_static_archive(&path, &bytes),
                        bytes: Cow::Owned(bytes),
                    });
                }
                Err(e) if resource_bundle && e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("couldn't read {}: {e}", path.display())),
            }
        }
        if resource_bundle {
            // A source-free XCTest bundle can name an executable it never built.
            return Ok(BundleCode {
                name: "Info.plist",
                bytes: Cow::Borrowed(&self.info_bytes),
                detached: true,
            });
        }
        Err(format!(
            "{} names no CFBundleExecutable",
            self.info_path.display()
        ))
    }
}

/// The bundle `executable` is the main executable of, if it is one: Flutter
/// signs `Framework.framework/Framework` itself, which `codesign` still
/// signs as its bundle.
pub(super) fn of_executable(executable: &Path) -> Option<PathBuf> {
    let parent = executable.parent()?;
    let deep = (parent.file_name()? == "MacOS")
        .then(|| parent.parent())
        .flatten()
        .filter(|contents| contents.file_name().is_some_and(|n| n == "Contents"))
        .and_then(Path::parent);
    let bundle = deep.unwrap_or(parent);
    if !["app", "appex", "framework", "xctest", "bundle", "xpc"]
        .iter()
        .any(|extension| bundle.extension().is_some_and(|e| e == *extension))
    {
        return None;
    }
    let layout = Layout::of(bundle).ok()?;
    (layout.root.join(layout.executable?) == executable).then(|| bundle.to_path_buf())
}
