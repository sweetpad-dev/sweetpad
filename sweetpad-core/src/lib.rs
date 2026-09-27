//! Shared SweetPad business logic over [`sweetpad_lib`].
//!
//! This crate holds the orchestration shared by both frontends (the `sweetpad`
//! CLI and the VS Code extension's N-API addon): build-settings and
//! compiler-argument resolution ([`build_settings`], [`build_context`]), finding
//! the app a build produced ([`app_locator`]) and the platforms a scheme builds
//! for ([`supported_platforms`]), local SwiftPM package discovery
//! ([`package_members`]), the simctl and devicectl output parsers
//! ([`devices`]), and the Build Server Protocol server ([`bsp`]). It depends on
//! `sweetpad-lib` for the file-format primitives and adds nothing
//! frontend-specific.

pub mod app_locator;
pub mod bsp;
pub mod build_context;
pub mod build_settings;
pub mod devices;
pub mod framing;
pub mod hot;
pub mod package_members;
pub mod paths;
pub mod scratch;
pub mod supported_platforms;
pub mod test_markers;
pub mod xcodebuild_args;
