//! What a build file is: its app's bundle identifier, version, build number
//! and platform, read from inside an `.ipa` or a flat `.pkg` without
//! unpacking it.

use std::path::{Path, PathBuf};

use crate::cli::{CliError, ErrorKind, xar::Xar, zip};

/// An App Store Connect platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    Ios,
    MacOs,
    TvOs,
    VisionOs,
}

impl Platform {
    /// The value App Store Connect's `platform` attributes take.
    pub(crate) fn api(self) -> &'static str {
        match self {
            Platform::Ios => "IOS",
            Platform::MacOs => "MAC_OS",
            Platform::TvOs => "TV_OS",
            Platform::VisionOs => "VISION_OS",
        }
    }

    pub(crate) fn from_api(value: &str) -> Option<Self> {
        [Self::Ios, Self::MacOs, Self::TvOs, Self::VisionOs]
            .into_iter()
            .find(|p| p.api() == value)
    }

    /// From an app's `DTPlatformName`. watchOS has no App Store Connect
    /// platform: a watch app ships inside its iOS app.
    pub(crate) fn from_sdk(name: &str) -> Option<Self> {
        match name {
            "iphoneos" => Some(Self::Ios),
            "macosx" => Some(Self::MacOs),
            "appletvos" => Some(Self::TvOs),
            "xros" => Some(Self::VisionOs),
            _ => None,
        }
    }
}

/// A file ready to upload.
#[derive(Debug, Clone)]
pub(crate) struct Artifact {
    pub(crate) path: PathBuf,
    pub(crate) file_name: String,
    pub(crate) size: u64,
    pub(crate) bundle_id: String,
    pub(crate) version: String,
    pub(crate) build: String,
    pub(crate) platform: Platform,
    /// `com.apple.ipa` or `com.apple.pkg`.
    pub(crate) uti: &'static str,
}

fn usage(message: String) -> CliError {
    CliError::new(message).kind(ErrorKind::Usage)
}

/// Read `path`'s app metadata. A file that is neither an IPA nor a package is
/// a usage error.
pub(crate) fn inspect(path: &Path) -> Result<Artifact, CliError> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| usage(format!("can't read {}: {e}", path.display())))?;
    if !metadata.is_file() {
        return Err(usage(format!(
            "{} isn't a file; upload takes an .ipa or a .pkg",
            path.display()
        )));
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| usage(format!("{} has no usable file name", path.display())))?
        .to_string();
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let (values, uti, default_platform) = match extension.as_deref() {
        Some("ipa") => (ipa_info(path)?, "com.apple.ipa", None),
        Some("pkg") => (pkg_info(path)?, "com.apple.pkg", Some(Platform::MacOs)),
        _ => {
            return Err(usage(format!(
                "{} isn't an .ipa or a .pkg, the files App Store Connect takes",
                path.display()
            )));
        }
    };
    let value = |key: &str| {
        values
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let missing = |key: &str| {
        usage(format!(
            "{}: the app's Info.plist has no {key}",
            path.display()
        ))
    };
    let platform = match value("DTPlatformName") {
        Some(name) => Platform::from_sdk(&name).ok_or_else(|| {
            usage(format!(
                "{}: App Store Connect takes no '{name}' builds; a watch app uploads inside its iOS app",
                path.display()
            ))
        })?,
        None => default_platform.ok_or_else(|| missing("DTPlatformName"))?,
    };
    Ok(Artifact {
        path: path.to_path_buf(),
        file_name,
        size: metadata.len(),
        bundle_id: value("CFBundleIdentifier").ok_or_else(|| missing("CFBundleIdentifier"))?,
        version: value("CFBundleShortVersionString")
            .ok_or_else(|| missing("CFBundleShortVersionString"))?,
        build: value("CFBundleVersion").ok_or_else(|| missing("CFBundleVersion"))?,
        platform,
        uti,
    })
}

fn is_app(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("app"))
}

/// `Payload/<App>.app/Info.plist`'s string values.
fn ipa_info(path: &Path) -> Result<Vec<(String, String)>, CliError> {
    let is_info = |name: &str| {
        let parts: Vec<&str> = name.split('/').collect();
        parts.len() == 3 && parts[0] == "Payload" && is_app(parts[1]) && parts[2] == "Info.plist"
    };
    let (_, data) = zip::read_member(path, is_info)
        .map_err(usage)?
        .ok_or_else(|| {
            usage(format!(
                "{} has no Payload/<App>.app/Info.plist",
                path.display()
            ))
        })?;
    Ok(sweetpad_lib::derived_data::plist_strings_from_bytes(&data))
}

/// The app bundle a package installs, as its `PackageInfo` lists it: the
/// shallowest `.app` among the bundles. `DTPlatformName` is implied.
fn pkg_info(path: &Path) -> Result<Vec<(String, String)>, CliError> {
    let xar = Xar::open(path).map_err(usage)?;
    let mut bundles = Vec::new();
    for (name, file) in xar.files() {
        if name.rsplit('/').next() != Some("PackageInfo") {
            continue;
        }
        let data = xar
            .read(file)
            .map_err(|e| usage(format!("{}: {name}: {e}", path.display())))?;
        let text = String::from_utf8_lossy(&data);
        let root = sweetpad_lib::xcscheme::parse(&text)
            .map_err(|e| usage(format!("{}: {name} isn't XML: {e}", path.display())))?;
        for bundle in root.descendants_named("bundle") {
            let bundle_path = bundle.attr("path").unwrap_or_default();
            if is_app(bundle_path) {
                bundles.push((bundle_path.matches('/').count(), bundle.clone()));
            }
        }
    }
    bundles.sort_by_key(|(depth, _)| *depth);
    let (_, app) = bundles.into_iter().next().ok_or_else(|| {
        usage(format!(
            "{} installs no app bundle; App Store Connect takes a package made from an app",
            path.display()
        ))
    })?;
    let mut values = Vec::new();
    for (attribute, key) in [
        ("id", "CFBundleIdentifier"),
        ("CFBundleShortVersionString", "CFBundleShortVersionString"),
        ("CFBundleVersion", "CFBundleVersion"),
    ] {
        if let Some(value) = app.attr(attribute) {
            values.push((key.to_string(), value.to_string()));
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ipa_names_its_app_version_and_platform() {
        let dir = crate::cli::testdir::TempDir::new("artifact-ipa");
        let app = dir.join("Payload/Delivery.app");
        std::fs::create_dir_all(app.join("PlugIns/Widget.appex")).unwrap();
        let plist = |id: &str, platform: &str| {
            crate::cli::xml_plist::document(&format!(
                "<dict><key>CFBundleIdentifier</key><string>{id}</string>\
                 <key>CFBundleShortVersionString</key><string>1.2</string>\
                 <key>CFBundleVersion</key><string>42</string>\
                 <key>DTPlatformName</key><string>{platform}</string></dict>"
            ))
        };
        std::fs::write(app.join("Info.plist"), plist("dev.example.app", "iphoneos")).unwrap();
        std::fs::write(
            app.join("PlugIns/Widget.appex/Info.plist"),
            plist("dev.example.app.Widget", "iphoneos"),
        )
        .unwrap();
        let ipa = dir.join("Delivery.ipa");
        assert!(
            std::process::Command::new("zip")
                .args(["-qr", ipa.to_str().unwrap(), "Payload"])
                .current_dir(&*dir)
                .status()
                .unwrap()
                .success()
        );
        let artifact = inspect(&ipa).unwrap();
        assert_eq!(artifact.bundle_id, "dev.example.app");
        assert_eq!(
            (artifact.version.as_str(), artifact.build.as_str()),
            ("1.2", "42")
        );
        assert_eq!(artifact.platform, Platform::Ios);
        assert_eq!(artifact.uti, "com.apple.ipa");
        assert_eq!(artifact.file_name, "Delivery.ipa");

        std::fs::write(app.join("Info.plist"), plist("dev.example.app", "watchos")).unwrap();
        std::fs::remove_file(&ipa).unwrap();
        std::process::Command::new("zip")
            .args(["-qr", ipa.to_str().unwrap(), "Payload"])
            .current_dir(&*dir)
            .status()
            .unwrap();
        let error = inspect(&ipa).unwrap_err();
        assert_eq!(error.error_kind(), ErrorKind::Usage);
        assert!(error.to_string().contains("watch app"), "{error}");
        assert!(inspect(&dir.join("Payload")).is_err());
    }
}
