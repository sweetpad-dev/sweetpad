//! Scaffolding shared by the crate's integration tests.

#[path = "../../src/testdir.rs"]
mod testdir;

pub use testdir::TempDir;
