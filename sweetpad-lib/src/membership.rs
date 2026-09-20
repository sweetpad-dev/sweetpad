//! The vocabulary of target membership, shared by the two project document
//! formats.
//!
//! A target builds a file in one of two ways: a classic per-file entry naming
//! a build phase, or a synchronized folder that builds everything under it
//! with per-target *exceptions*. The two formats record both differently — a
//! `PBXBuildFile` against a `target-membership` entry on the file's own node,
//! a `PBXFileSystemSynchronizedRootGroup` against a `kind: "folder"` node —
//! but what a caller asks for and what a report says is the same, so it lives
//! here and [`crate::membership_pbxproj`], [`crate::membership_xcproj`],
//! [`crate::sync_pbxproj`] and [`crate::sync_xcproj`] all speak it.

/// The build phase a classic entry belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Sources,
    Resources,
    Headers,
    Frameworks,
    /// A copy-files phase, with its display name (e.g. `Embed XPC Services`).
    Copy(String),
}

impl Phase {
    /// The stable machine name (`sources`, `resources`, `headers`,
    /// `frameworks`, `copy`).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Phase::Sources => "sources",
            Phase::Resources => "resources",
            Phase::Headers => "headers",
            Phase::Frameworks => "frameworks",
            Phase::Copy(_) => "copy",
        }
    }

    /// The phase a `--phase` flag names. Copy phases are absent on purpose: a
    /// target can carry several and they are told apart by name, so a kind
    /// alone does not address one.
    #[must_use]
    pub fn parse(kind: &str) -> Option<Phase> {
        match kind {
            "sources" => Some(Phase::Sources),
            "resources" => Some(Phase::Resources),
            "headers" => Some(Phase::Headers),
            "frameworks" => Some(Phase::Frameworks),
            _ => None,
        }
    }

    /// Human rendering: the kind, plus the copy phase's name.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Phase::Copy(name) => format!("copy ({name})"),
            other => other.kind().to_string(),
        }
    }
}

/// What kind of node the membership points at — files convert to folders;
/// variant/version groups are the constructs a script leaves classic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    File,
    VariantGroup,
    VersionGroup,
    Other,
}

impl RefKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RefKind::File => "file",
            RefKind::VariantGroup => "variantGroup",
            RefKind::VersionGroup => "versionGroup",
            RefKind::Other => "other",
        }
    }
}

/// One classic membership entry: a file a target builds, with the per-file
/// details the entry carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Project-dir-relative resolved path.
    pub path: String,
    pub phase: Phase,
    pub kind: RefKind,
    /// Per-file compiler flags — `settings.COMPILER_FLAGS`, or `arguments`.
    pub compiler_flags: Option<String>,
    /// Header visibility and copy handling — `settings.ATTRIBUTES`
    /// (`Public`/`Private`, `RemoveHeadersOnCopy`, `CodeSignOnCopy`), or the
    /// `header-role`, `header-preservation` and `code-sign-on-copy` keys.
    pub attributes: Vec<String>,
    /// `platformFilters`, or `platforms`.
    pub platform_filters: Vec<String>,
}

/// The outcome of removing one path's membership from one target. Empty
/// `removed_phases` records the no-op (the path wasn't a member), so re-run
/// scripts stay green.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub path: String,
    /// Display names of the phases entries were removed from.
    pub removed_phases: Vec<String>,
    /// Set when nothing references the file anymore, so the reference itself
    /// left the project. Only a pbxproj does this: there a file exists because
    /// an object says so, while a `project.xcproj` node is the navigator entry
    /// itself and outlives every target that stops building it.
    pub deleted_reference: bool,
    /// Ancestor groups deleted because the reference removal emptied them.
    pub pruned_groups: usize,
}

/// The outcome of adding one path to one target's phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addition {
    pub path: String,
    /// Display name of the phase joined.
    pub phase: String,
    /// The build-file object created, when the format has one.
    pub build_file: Option<String>,
    /// Set when the path was already in that phase — a recorded no-op.
    pub already_member: bool,
}

/// One synchronized folder as seen by one target: where it lives and which of
/// its files that target opts out of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootReport {
    /// Project-dir-relative folder path.
    pub dir: String,
    /// The target's exceptions, folder-relative, in file order.
    pub exceptions: Vec<String>,
}

/// A target's synchronized folders, for `folder list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRoots {
    pub target: String,
    pub roots: Vec<RootReport>,
}

/// The result of attaching a folder: a brand-new one, an existing folder
/// (used by another target) newly attached, or nothing to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddOutcome {
    Created,
    AttachedExisting,
    AlreadyAttached,
}

/// The result of detaching a folder. `deleted_object` is set when the folder
/// itself left the project, which only happens in a pbxproj — there the group
/// object exists to be referenced, while a `project.xcproj` folder node is a
/// navigator entry that stands on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveOutcome {
    Detached { deleted_object: bool },
    NotAttached,
}

/// The result of adding a membership exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExcludeOutcome {
    /// `exception` is the folder-relative path now excepted under `root_dir`.
    Added {
        root_dir: String,
        exception: String,
    },
    AlreadyExcluded {
        root_dir: String,
        exception: String,
    },
}

/// The result of dropping a membership exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncludeOutcome {
    Removed { root_dir: String, exception: String },
    NotExcluded,
}
