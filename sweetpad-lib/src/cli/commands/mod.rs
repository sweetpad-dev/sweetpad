//! One submodule per top-level resource. Each defines its clap `Action`
//! subcommand enum and a `run(ctx, action)` dispatcher. Bodies are scaffold
//! stubs (see [`crate::cli::not_implemented`]) over the wired-up resolution,
//! config, state, and output plumbing — implemented one vertical slice at a
//! time per `CLI_DESIGN.md`.

pub mod app;
pub mod build;
pub mod destination;
pub mod project;
pub mod scheme;
pub mod settings;
pub mod simulator;
