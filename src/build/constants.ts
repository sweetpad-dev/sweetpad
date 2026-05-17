/**
 * Problem matchers attached to every sweetpad build task. Defined in
 * package.json.
 *
 * `sweetpad-watch` is a background-task lifecycle matcher — it watches for
 * the 🍭/🍩 markers `writeWatchMarkers` emits to detect when a background
 * task has reached steady state. It carries no diagnostic patterns.
 *
 * Build-output diagnostics are produced programmatically by
 * `DiagnosticsManager` in `src/build/diagnostics.ts`, not by problem matchers.
 */
export const BUILD_TASK_PROBLEM_MATCHERS = ["$sweetpad-watch"];
