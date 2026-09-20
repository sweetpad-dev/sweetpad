//! What an edit to a synchronized folder's membership exceptions reports,
//! shared by the two project document formats.
//!
//! A synchronized folder builds whatever is on disk under it, and a
//! *membership exception* adjusts that for one target: a file excluded from a
//! target the folder belongs to, or included for one it doesn't. The two
//! formats record them differently —
//! `PBXFileSystemSynchronizedBuildFileExceptionSet` objects against a
//! `membership-exceptions` list — but the outcome a caller reports is the
//! same, so it lives here and [`crate::sync_pbxproj`] and
//! [`crate::sync_xcproj`] both speak it.

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
