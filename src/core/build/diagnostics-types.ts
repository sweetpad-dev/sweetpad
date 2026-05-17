/**
 * Per-build accumulator. BuildManager calls `recordLine` for every line of
 * xcodebuild output and `flush` exactly once when the build ends.
 */
export interface DiagnosticAccumulator {
  recordLine(rawLine: string): void;
  flush(): void;
}

/** Mode passed to beginBuild(): "xcbeautify" parses xcbeautify's friendlier
 *  one-line-per-issue output, "xcodebuild" parses raw xcodebuild output. */
export type ParseMode = "xcbeautify" | "xcodebuild";

/**
 * Spans the build system and the host's "Problems" surface. The VS Code
 * adapter wraps `vscode.DiagnosticCollection`; the CLI adapter writes a JSON
 * file under `.sweetpad/builds/<id>/`.
 */
export interface DiagnosticsCollector {
  beginBuild(options: { mode: ParseMode }): DiagnosticAccumulator;
}
