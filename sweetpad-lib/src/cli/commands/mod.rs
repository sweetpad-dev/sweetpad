//! One submodule per top-level resource. Each defines its clap `Action`
//! subcommand enum and a `run(ctx, action)` dispatcher over the shared
//! resolution, config, state, and output plumbing. See `CLI_DESIGN.md`.

pub mod app;
pub mod bsp;
pub mod build;
pub mod derived_data;
pub mod destination;
pub mod device;
pub mod doctor;
pub mod format;
pub mod pbxproj;
pub mod project;
pub mod scheme;
pub mod settings;
pub mod simulator;
pub mod test;
