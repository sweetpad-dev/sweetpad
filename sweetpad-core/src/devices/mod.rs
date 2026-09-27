//! Simulators and physical devices as `simctl` and `devicectl` report them.
//! Parsing only: each frontend runs the tools its own way (the CLI with
//! timeouts and temp files, the extension through its exec layer) and hands
//! the output here, so both read the listings, the destination specifiers
//! and the app's processes the same way.

pub mod devicectl;
pub mod simctl;
