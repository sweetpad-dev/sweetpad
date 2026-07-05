//! One submodule per top-level resource. Each defines its clap `Action`
//! subcommand enum and a `run(ctx, action)` dispatcher over the shared
//! resolution, config, state, and output plumbing. See `CLI_DESIGN.md`.

use std::path::Path;

/// The shared `--watch` loop: run `once` now, then rerun it on every
/// Swift save under `root` until Ctrl-C ends the process. A failing iteration
/// reports its error and keeps watching — a broken save must not end the loop.
pub(crate) fn watch_swift(
    ctx: &mut crate::cli::Context,
    root: &Path,
    mut once: impl FnMut(&mut crate::cli::Context) -> crate::cli::CommandResult,
) -> crate::cli::CommandResult {
    use std::sync::mpsc;
    // A watch loop reruns forever and renders per-iteration — there is no
    // single envelope to emit, so a --json consumer would block on silence
    // (and ndjson would interleave iteration payloads with no terminal
    // event). Mirror `app run` and refuse up front.
    if ctx.out.is_json() || ctx.out.is_ndjson() {
        return Err(crate::cli::CliError::new(
            "--watch reruns continuously and has no machine-readable form; drop --watch, or \
             run single passes with -o json/ndjson yourself",
        ));
    }
    loop {
        match once(ctx) {
            Ok(_) => {}
            Err(e) => ctx.out.error(&e),
        }
        ctx.out
            .note("watching for Swift changes (Ctrl-C to stop)…");
        let (tx, rx) = mpsc::channel::<()>();
        let watcher = crate::cli::inject::watcher::Watcher::start(
            root,
            std::sync::Arc::new(move |_path: &Path| {
                let _ = tx.send(());
            }),
        );
        // Block until the first save, then drain the burst so one Save-All
        // triggers one rerun.
        if rx.recv().is_err() {
            return Ok(crate::cli::Rendered::Streamed);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        while rx.try_recv().is_ok() {}
        drop(watcher);
    }
}

pub mod app;
pub mod archive;
pub mod bsp;
pub mod build;
pub mod clean;
pub mod context;
pub mod dependency;
pub mod derived_data;
pub mod destination;
pub mod device;
pub mod doctor;
pub mod format;
pub mod help_topics;
pub mod merge;
pub mod open;
pub mod pbxproj;
pub mod project;
pub mod scheme;
pub mod self_update;
pub mod settings;
pub mod simulator;
pub mod status;
pub mod spm;
pub mod test;
