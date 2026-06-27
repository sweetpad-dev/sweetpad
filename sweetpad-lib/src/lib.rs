//! Interface-agnostic utilities for Xcode project files and build-settings
//! resolution.
//!
//! This crate is the foundation layer: it parses and serializes Apple's
//! project-domain formats (pbxproj, xcconfig, schemes, binary plists), models
//! Xcode projects and workspaces, and resolves build settings and compiler
//! arguments. It knows nothing about any frontend — the `sweetpad` CLI, the VS
//! Code extension's N-API addon, and the BSP server all build on top of it
//! (`sweetpad-core` adds the shared orchestration; the CLI and addon are the
//! interfaces). Public entry points are plain Rust: [`xcode::active_install`],
//! [`project::open`], [`workspace::open`].

pub mod bplist;
pub mod catalog_cache;
pub mod compiler_args;
pub mod condition;
pub mod destination;
mod file_cache;
pub mod pbxproj;
pub mod pbxproj_merge;
pub mod pbxproj_writer;
pub mod project;
pub mod resolver;
pub mod scheme;
pub mod spm_pbxproj;
pub mod spm_resolved;
pub mod workspace;
pub mod xcconfig;
pub mod xcode;
pub mod xcode_hash;
pub mod xcscheme;
pub mod xcspec;
