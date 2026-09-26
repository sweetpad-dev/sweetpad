//! The vocabulary of a project's Swift package dependencies, shared by the two
//! project document formats.
//!
//! A project declares packages, remote or local, and its targets consume their
//! products. `project.pbxproj` keeps that as a graph of
//! `XC*SwiftPackageReference` and `XCSwiftPackageProductDependency` objects;
//! `project.xcproj` as a `packages` list plus product references on each
//! target. What a caller asks for and what a report says is the same either
//! way, so it lives here and [`crate::spm_pbxproj`] and [`crate::spm_xcproj`]
//! both speak it.

/// A declared package, plus the products it provides and where they are
/// linked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredPackage {
    /// How the document itself refers to the package, which is what the
    /// backends take back to edit it: the `XC*SwiftPackageReference` object
    /// GUID in a pbxproj, the [`package_name`] product references spell it by
    /// in a `project.xcproj`.
    pub id: String,
    /// SwiftPM identity (lowercased basename of the URL/path), used to correlate
    /// with a `Package.resolved` pin.
    pub identity: String,
    pub kind: PackageKind,
    /// The version requirement, for remote packages (locals have none).
    pub requirement: Option<Requirement>,
    /// `(product, target)` links this package's products participate in.
    pub products: Vec<ProductLink>,
}

/// A declared package is either a remote git repo or a local directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageKind {
    Remote { url: String },
    Local { relative_path: String },
}

impl PackageKind {
    /// The repository URL (remote) or relative path (local) — the human display.
    #[must_use]
    pub fn display(&self) -> &str {
        match self {
            PackageKind::Remote { url } => url,
            PackageKind::Local { relative_path } => relative_path,
        }
    }

    #[must_use]
    pub fn is_remote(&self) -> bool {
        matches!(self, PackageKind::Remote { .. })
    }
}

/// A remote package's version requirement, flattened to the one or two
/// version-ish values each `kind` carries. The kinds are named the way a
/// pbxproj's `requirement` dict spells them; the `project.xcproj` reader maps
/// its own keys onto the same names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    /// `upToNextMajorVersion` / `upToNextMinorVersion` / `exactVersion` /
    /// `versionRange` / `branch` / `revision`.
    pub kind: String,
    /// `minimumVersion` (ranges), `version` (exact), `branch`, or `revision`.
    pub value: Option<String>,
    /// `maximumVersion`, for `versionRange` only.
    pub upper: Option<String>,
}

impl Requirement {
    /// A compact human rendering, e.g. `from 5.0.0`, `branch main`,
    /// `5.0.0 ..< 6.0.0`.
    #[must_use]
    pub fn display(&self) -> String {
        let v = self.value.as_deref().unwrap_or("?");
        match self.kind.as_str() {
            "upToNextMajorVersion" => format!("from {v}"),
            "upToNextMinorVersion" => format!("up-to-next-minor from {v}"),
            "exactVersion" => format!("exact {v}"),
            "versionRange" => format!("{v} ..< {}", self.upper.as_deref().unwrap_or("?")),
            "branch" => format!("branch {v}"),
            "revision" => format!("revision {v}"),
            other => format!("{other} {v}"),
        }
    }
}

/// A product linked into a target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProductLink {
    pub product: String,
    pub target: String,
}

/// The version requirement to record when adding a remote package. Mirrors the
/// `swift package add-dependency` requirement flags; the CLI maps its flags onto
/// this so the backends stay clap-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementSpec {
    /// `--from` (up to the next major).
    UpToNextMajor(String),
    /// `--up-to-next-minor-from`.
    UpToNextMinor(String),
    /// `--exact`.
    Exact(String),
    /// `--from … --to …` (half-open range).
    Range { from: String, to: String },
    /// `--branch`.
    Branch(String),
    /// `--revision`.
    Revision(String),
}

/// SwiftPM's package identity for a repository URL: the last path component,
/// without a trailing `.git`, lowercased. Matches the `identity` key in
/// `Package.resolved` (and the basename the serializer annotates with).
#[must_use]
pub fn identity_from_url(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(url)
        .trim_end_matches(".git")
        .to_ascii_lowercase()
}

/// Identity for a local package: the lowercased last path component of its
/// relative path.
#[must_use]
pub fn identity_from_path(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
}

/// The name Xcode gives a package, which is how a `project.xcproj` product
/// reference spells the package it comes from.
///
/// Unlike [`identity_from_url`] it keeps its case — `SFSafeSymbols`, not
/// `sfsafesymbols`. A repository URL gives its last component without `.git`
/// or a `#fragment`; a registry identity `scope.name` gives the name; a local
/// package gives its directory's name. Measured by converting projects with
/// Xcode 27.2: `wishkit-ios.git` becomes `wishkit-ios`, `Alamofire.Alamofire`
/// becomes `Alamofire`, and `Packages/LocalKit` becomes `LocalKit`.
#[must_use]
pub fn package_name(kind: &PackageKind) -> String {
    match kind {
        PackageKind::Remote { url } => {
            let trimmed = url.trim_end_matches('/');
            let last = trimmed.rsplit(['/', ':']).next().unwrap_or(trimmed);
            let last = last.split('#').next().unwrap_or(last);
            let last = last.strip_suffix(".git").unwrap_or(last);
            let is_registry = !trimmed.contains(['/', ':']);
            match last.split_once('.') {
                Some((_, name)) if is_registry => name.to_string(),
                _ => last.to_string(),
            }
        }
        PackageKind::Local { relative_path } => relative_path
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(relative_path)
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_normalizes_url_and_path() {
        assert_eq!(
            identity_from_url("https://github.com/mergesort/Bodega"),
            "bodega"
        );
        assert_eq!(
            identity_from_url("https://github.com/kaishin/Gifu.git"),
            "gifu"
        );
        assert_eq!(identity_from_url("https://github.com/foo/Bar/"), "bar");
        assert_eq!(identity_from_url("keychain-swift"), "keychain-swift");
        assert_eq!(identity_from_path("Packages/Env"), "env");
    }

    #[test]
    fn a_package_is_named_the_way_xcode_names_it() {
        let remote = |url: &str| {
            package_name(&PackageKind::Remote {
                url: url.to_string(),
            })
        };
        assert_eq!(
            remote("https://github.com/SFSafeSymbols/SFSafeSymbols"),
            "SFSafeSymbols"
        );
        assert_eq!(
            remote("https://github.com/wishkit/wishkit-ios.git"),
            "wishkit-ios"
        );
        assert_eq!(
            remote("https://github.com/gonzalezreal/swift-markdown-ui#installation"),
            "swift-markdown-ui"
        );
        assert_eq!(remote("git@github.com:apple/swift-log.git"), "swift-log");
        assert_eq!(remote("Alamofire.Alamofire"), "Alamofire");
        let local = PackageKind::Local {
            relative_path: "Packages/LocalKit".to_string(),
        };
        assert_eq!(package_name(&local), "LocalKit");
    }
}
