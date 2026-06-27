//! Shared SweetPad business logic over [`sweetpad_lib`].
//!
//! This crate holds the orchestration shared by both frontends (the `sweetpad`
//! CLI and the VS Code extension's N-API addon): build-settings and
//! compiler-argument resolution ([`build_settings`], [`build_context`]) and the
//! Build Server Protocol server ([`bsp`]). It depends on `sweetpad-lib` for the
//! file-format primitives and adds nothing frontend-specific.

pub mod bsp;
pub mod build_context;
pub mod build_settings;
pub mod framing;
pub mod paths;
