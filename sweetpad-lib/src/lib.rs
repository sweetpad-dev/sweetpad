//! Xcode build-settings resolver.
//!
//! Every module here is **interface-agnostic** — it knows nothing about Node,
//! Python, or any binding technology. The public entry points the bindings wrap
//! are plain Rust: [`xcode::active_install`], [`project::open`],
//! [`workspace::open`], and [`build_settings::resolve_build_settings`].
//!
//! `node` is the *only* binding-aware module: a thin, feature-gated N-API layer
//! (`--features node`) that maps those core functions to JS-facing types. A
//! second interface (e.g. a PyO3 `python` module) would be another sibling thin
//! layer over the same core — nothing in the core would have to change.

pub mod bplist;
pub mod bsp;
pub mod build_context;
pub mod build_settings;
pub mod catalog_cache;
// The standalone CLI (resource-first command tree). Gated on the `cli` feature
// so the N-API addon build never pulls in clap/serde/toml. See `CLI_DESIGN.md`.
#[cfg(feature = "cli")]
pub mod cli;
pub mod compiler_args;
pub mod condition;
pub mod destination;
mod file_cache;
mod framing;
#[cfg(feature = "node")]
pub mod node;
pub mod pbxproj;
pub mod pbxproj_writer;
pub mod project;
pub mod resolver;
pub mod scheme;
pub mod vscode_cli;
pub mod workspace;
pub mod xcconfig;
pub mod xcode;
pub mod xcode_hash;
pub mod xcscheme;
pub mod xcspec;
