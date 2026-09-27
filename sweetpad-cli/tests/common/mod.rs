//! Scaffolding shared by the CLI's integration tests.

#[path = "../../../sweetpad-lib/src/testdir.rs"]
mod testdir;

pub use testdir::TempDir;
