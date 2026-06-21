//! The render contract: a command produces a payload, and the dispatcher in
//! [`crate::cli::run`] renders it once — choosing human output or the JSON
//! envelope centrally, so a command never branches on `--json` itself.
//!
//! Migrated commands return [`Rendered::Data`] with a typed payload; commands
//! that stream their own output live (or haven't migrated yet) return
//! [`Rendered::Streamed`] and the dispatcher renders nothing for them.

use crate::cli::output::Output;

/// A renderable command payload. `human` writes the human view through `out`'s
/// sinks; `json` returns the DATA only — the `{schema, ok, data}` envelope is
/// added centrally by the dispatcher, so a payload never knows about it.
pub trait Render {
    /// Human-mode rendering. Free to use `out.line`/`out.item`/`out.use_color()`.
    fn human(&self, out: &Output);
    /// The `data` field of the success envelope. Pure; no I/O.
    fn json(&self) -> serde_json::Value;
}

/// What a command hands back to the dispatcher.
///
/// - Query/action commands return [`Rendered::Data`] — rendered once, centrally.
/// - Streaming commands (and not-yet-migrated ones that self-emit) return
///   [`Rendered::Streamed`]; the dispatcher emits nothing for them.
///
/// The `exit` on `Data` lets a command render its report *and* exit non-zero
/// (e.g. `doctor` with problems, a red `test` suite) without a separate error
/// path — process exit is a dispatch concern, not part of the `Render` trait.
pub enum Rendered {
    Data {
        payload: Box<dyn Render>,
        exit: u8,
    },
    Streamed,
}

impl Rendered {
    /// A payload that renders and exits 0.
    pub fn data(r: impl Render + 'static) -> Self {
        Self::Data {
            payload: Box::new(r),
            exit: 0,
        }
    }

    /// A payload that renders but forces a non-zero process exit.
    pub fn data_with_exit(r: impl Render + 'static, exit: u8) -> Self {
        Self::Data {
            payload: Box::new(r),
            exit,
        }
    }
}
