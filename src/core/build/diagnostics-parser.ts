/**
 * Pure parser for diagnostic lines emitted by `xcodebuild` and `xcbeautify`.
 *
 * Kept free of vscode imports so it can be unit-tested without mocking the
 * editor surface. The `DiagnosticsManager` is the one that translates these
 * results into `vscode.Diagnostic` instances and pushes them to a
 * `DiagnosticCollection`.
 *
 * Three formats are recognized:
 *
 *  1. xcbeautify error    `❌` or `[x]` prefix, e.g.
 *       ❌ /path/Foo.swift:10:5: cannot find 'bar' in scope
 *
 *  2. xcbeautify warning  `⚠️` or `[!]` prefix, e.g.
 *       ⚠️ /path/Foo.swift:12:1: variable 'x' was never used
 *
 *  3. raw xcodebuild      `file:line:column: error|warning: msg`, e.g.
 *       /path/Foo.swift:10:5: error: cannot find 'bar' in scope
 *
 * Anchored to absolute paths (leading `/`) on purpose — xcodebuild emits
 * absolute paths in its `-resultBundlePath` flow, and anchoring keeps the
 * regex from accidentally matching lines like `note: foo:1:2: error: bar`.
 */

export type DiagnosticSeverity = "error" | "warning";
export type DiagnosticSource = "xcodebuild" | "xcbeautify";

/**
 * Which line formats the parser should try:
 *
 *  - "xcbeautify": only the `❌`/`[x]` and `⚠️`/`[!]` prefixed forms. Set
 *    this when xcbeautify is in the pipeline — xcodebuild's raw lines are
 *    consumed and reformatted by xcbeautify, but in some versions the raw
 *    line *also* leaks through with slightly different casing, which
 *    produces a near-duplicate diagnostic if both parsers run.
 *  - "xcodebuild": only the canonical `path:line:col: error|warning: msg`
 *    form. Set this when xcbeautify is *not* in the pipeline — the raw
 *    lines are all we get.
 *  - "auto": try every pattern. The right choice for unit tests and any
 *    caller that doesn't know what produced the line.
 */
export type ParseMode = "xcbeautify" | "xcodebuild" | "auto";

export type ParsedDiagnostic = {
  file: string;
  line: number;
  column: number;
  severity: DiagnosticSeverity;
  message: string;
  source: DiagnosticSource;
};

// Matches `\x1b[...m` SGR escape sequences (the "set graphics rendition"
// family — colors, bold, reset, etc.). xcbeautify color-codes its prefixes
// and messages; we strip them so the diagnostic regexes see plain text.
//   Example match: "\x1b[31m" (red), "\x1b[1;33m" (bold yellow), "\x1b[0m" (reset)
// biome-ignore lint/suspicious/noControlCharactersInRegex: intentional — matches ANSI escape sequences
const ANSI_STRIP_REGEX = /\x1b\[[0-9;]*m/g;

// xcbeautify error lines.
//   Example:  ❌ /Users/me/App/Sources/Foo.swift:10:5: cannot find 'bar' in scope
//   Example:  [x] /Users/me/App/Sources/Foo.swift:10:5: cannot find 'bar' in scope
//
// Pattern breakdown:
//   ^                      anchor to start of line — prevents mid-line matches
//                          like "Build failed ❌ foo.swift:1:1: msg"
//   (?:❌|\[x\])           non-capturing: the emoji form OR the ascii `[x]` form
//                          (xcbeautify picks one based on the --disable-colored-output
//                          flag and the terminal's locale)
//   \s*                    flexible spacing — xcbeautify usually emits one space,
//                          but the emoji is double-width and some terminals add padding
//   (\/[^:]+)              group 1, file path: must start with `/` (absolute) and
//                          contain no `:` so the next `:` belongs to the line number,
//                          not the path. Tradeoff: paths with `:` won't match — fine
//                          on macOS where `:` is illegal in filenames.
//   :(\d+):(\d+)           groups 2 and 3, line and column (1-based, like Xcode shows)
//   :\s*                   separator before the message; `\s*` because the message
//                          may be glued straight to the colon in some xcbeautify versions
//   (.*)                   group 4, the diagnostic message (rest of the line)
//   $                      anchor to end — required because earlier `.trim()` strips CR
const XCBEAUTIFY_ERROR_REGEX = /^(?:❌|\[x\])\s*(\/[^:]+):(\d+):(\d+):\s*(.*)$/;

// xcbeautify warning lines — same shape as the error pattern with a different prefix.
//   Example:  ⚠️ /Users/me/App/Sources/Foo.swift:12:1: variable 'x' was never used
//   Example:  [!] /Users/me/App/Sources/Foo.swift:12:1: variable 'x' was never used
// Severity is implied by which regex matched, not captured in a group.
const XCBEAUTIFY_WARNING_REGEX = /^(?:⚠️|\[!\])\s*(\/[^:]+):(\d+):(\d+):\s*(.*)$/;

// Raw xcodebuild diagnostic lines (no xcbeautify in the pipeline). Format is
// Clang/Swift's canonical `path:line:col: severity: message`.
//   Example:  /Users/me/App/Sources/Foo.swift:10:5: error: cannot find 'bar' in scope
//   Example:  /Users/me/App/Sources/Foo.swift:12:1: warning: variable 'x' was never used
//
// Pattern breakdown:
//   ^(\/[^:]+):(\d+):(\d+)   same file/line/col shape as the xcbeautify patterns
//   :\s+                     separator before the severity word — note `\s+` (one or
//                            more) here, vs `\s*` in the xcbeautify patterns: raw
//                            xcodebuild always emits a space, xcbeautify sometimes doesn't
//   (error|warning)          group 4, the severity word — note we *don't* match `note:`
//                            here, so xcodebuild's note continuation lines (e.g.
//                            "Foo.swift:10:5: note: see declaration of 'bar'") are
//                            deliberately skipped. They could be attached as
//                            relatedInformation on the preceding diagnostic later.
//   :\s+(.*)$                separator, then group 5 — the rest of the message
const XCODEBUILD_REGEX = /^(\/[^:]+):(\d+):(\d+):\s+(error|warning):\s+(.*)$/;

export function parseDiagnosticLine(rawLine: string, mode: ParseMode = "auto"): ParsedDiagnostic | null {
  // `\r` would block the `$` anchor; strip CR + any stray ANSI (the v3 task
  // terminal already strips ANSI before invoking onOutputLine, but the v2
  // fallback does not).
  const line = rawLine.replace(ANSI_STRIP_REGEX, "").replace(/\r$/, "").trim();
  if (line.length === 0) return null;

  const tryXcbeautify = mode === "xcbeautify" || mode === "auto";
  const tryXcodebuild = mode === "xcodebuild" || mode === "auto";

  if (tryXcbeautify) {
    const errMatch = XCBEAUTIFY_ERROR_REGEX.exec(line);
    if (errMatch) {
      return {
        file: errMatch[1],
        line: Number.parseInt(errMatch[2], 10),
        column: Number.parseInt(errMatch[3], 10),
        severity: "error",
        message: errMatch[4].trim(),
        source: "xcbeautify",
      };
    }

    const warnMatch = XCBEAUTIFY_WARNING_REGEX.exec(line);
    if (warnMatch) {
      return {
        file: warnMatch[1],
        line: Number.parseInt(warnMatch[2], 10),
        column: Number.parseInt(warnMatch[3], 10),
        severity: "warning",
        message: warnMatch[4].trim(),
        source: "xcbeautify",
      };
    }
  }

  if (tryXcodebuild) {
    const rawMatch = XCODEBUILD_REGEX.exec(line);
    if (rawMatch) {
      return {
        file: rawMatch[1],
        line: Number.parseInt(rawMatch[2], 10),
        column: Number.parseInt(rawMatch[3], 10),
        severity: rawMatch[4] as DiagnosticSeverity,
        message: rawMatch[5].trim(),
        source: "xcodebuild",
      };
    }
  }

  return null;
}
