import * as vscode from "vscode";

import { type ParseMode, parseDiagnosticLine, type ParsedDiagnostic } from "../../core/build/diagnostics-parser";
import type { Diagnostic as WireDiagnostic } from "../../protocol/types";
import { commonLogger } from "../logger";

/**
 * Source identifiers used by sourcekit-lsp (via the swiftlang.swift-vscode
 * extension) when it publishes diagnostics. We compare against these in
 * `isAlreadyReportedByLsp` to avoid duplicating errors that the LSP has
 * already surfaced in the Problems panel.
 *
 * Kept as a list because the value has drifted between releases — older
 * builds tagged everything `SourceKit`, newer ones use `sourcekit-lsp`.
 */
const LSP_SOURCES = new Set(["sourcekit-lsp", "SourceKit", "swift"]);

const SWIFT_FILE_REGEX = /\.swift$/i;

/**
 * Owns the `sweetpad` DiagnosticCollection and the lifecycle of build-time
 * diagnostics:
 *
 *  - `beginBuild()` clears the collection (matching VS Code's existing
 *    problem-matcher contract: each build starts from a clean slate) and
 *    returns an Accumulator the build flow feeds lines into.
 *  - `flush()` publishes the freshly-parsed diagnostics, filtering out
 *    entries sourcekit-lsp has already reported at the same position
 *    (Swift files only — ObjC, headers, plist, linker output stay).
 *  - `onDidChangeTextDocument` drops entries for any file the user starts
 *    editing. sourcekit-lsp takes over for the open file; keeping the
 *    build-time entries around just produces ghost errors at the wrong
 *    line as text shifts (issue #171).
 *
 * The collection is reused across builds rather than recreated — VS Code
 * keys diagnostics by collection identity in the Problems panel and we want
 * "sweetpad" to stay a stable grouping.
 */
export class DiagnosticsManager implements vscode.Disposable {
  private readonly collection: vscode.DiagnosticCollection;
  private readonly subscriptions: vscode.Disposable[] = [];

  constructor() {
    this.collection = vscode.languages.createDiagnosticCollection("sweetpad");

    this.subscriptions.push(
      vscode.workspace.onDidChangeTextDocument((event) => {
        if (event.contentChanges.length === 0) return;
        if (this.collection.get(event.document.uri)?.length) {
          this.collection.delete(event.document.uri);
        }
      }),
    );
  }

  /**
   * Open a per-build accumulator. Clears the existing diagnostic set so the
   * upcoming build starts from a blank Problems panel. Call `recordLine` on
   * the returned accumulator for every line of build output, then `flush`
   * exactly once at the end (success or failure — failed builds are when we
   * most want the diagnostics published).
   *
   * `mode` selects which line formats the parser should accept — set it to
   * match what's actually in the build pipeline. See `ParseMode` in
   * `./diagnostics-parser` for the trade-off.
   */
  beginBuild(options: { mode: ParseMode }): DiagnosticAccumulator {
    this.collection.clear();
    return new DiagnosticAccumulator(this.collection, options.mode);
  }

  /**
   * Apply diagnostics that have already been parsed server-side (the
   * `diagnostics` array of a Build wire response). Replaces the existing
   * collection in one go — matches the `beginBuild()` → `flush()` contract
   * so the Problems panel resets per build.
   */
  applyWireDiagnostics(diagnostics: WireDiagnostic[]): void {
    this.collection.clear();
    if (diagnostics.length === 0) return;

    const perFile = new Map<string, WireDiagnostic[]>();
    for (const d of diagnostics) {
      const bucket = perFile.get(d.file);
      if (bucket) bucket.push(d);
      else perFile.set(d.file, [d]);
    }

    for (const [file, diags] of perFile) {
      const uri = vscode.Uri.file(file);
      const surviving = diags.filter((d) => !isAlreadyReportedByLsp(uri, wireToParsed(d)));
      if (surviving.length === 0) continue;
      this.collection.set(uri, surviving.map(wireToVscodeDiagnostic));
    }
  }

  dispose(): void {
    for (const sub of this.subscriptions) sub.dispose();
    this.collection.dispose();
  }
}

function wireToVscodeDiagnostic(d: WireDiagnostic): vscode.Diagnostic {
  return toVscodeDiagnostic(wireToParsed(d));
}

function wireToParsed(d: WireDiagnostic): ParsedDiagnostic {
  // The wire shape carries the parser's `source` as a string for forward-
  // compatibility, but the values it actually emits are always one of the
  // existing union members.
  return {
    file: d.file,
    line: d.line,
    column: d.column,
    severity: d.severity,
    message: d.message,
    source: d.source as ParsedDiagnostic["source"],
  };
}

export class DiagnosticAccumulator {
  private readonly perFile = new Map<string, ParsedDiagnostic[]>();
  private flushed = false;

  constructor(
    private readonly collection: vscode.DiagnosticCollection,
    private readonly mode: ParseMode,
  ) {}

  /**
   * Feed one line of build output to the parser. Lines that don't match any
   * known diagnostic format are silently ignored — this is the hot path for
   * every line xcodebuild prints, so the parser is intentionally cheap.
   *
   * Deduplicates by position: same `(file, line, column, severity)` keeps
   * the first diagnostic seen and drops later ones. The Swift compiler can
   * emit the same error twice from the frontend and the driver with
   * slightly different message casing (lowercase original vs capitalized
   * "rendered" form), and xcbeautify can re-emit a formatted copy of a raw
   * line. Two genuinely distinct diagnostics at the exact same column with
   * the same severity are vanishingly rare; the cost of collapsing them is
   * much lower than the cost of showing a near-duplicate in the panel.
   */
  recordLine(rawLine: string): void {
    const diag = parseDiagnosticLine(rawLine, this.mode);
    if (!diag) return;

    const bucket = this.perFile.get(diag.file);
    if (!bucket) {
      this.perFile.set(diag.file, [diag]);
      return;
    }

    const isDuplicate = bucket.some(
      (existing) =>
        existing.line === diag.line && existing.column === diag.column && existing.severity === diag.severity,
    );
    if (!isDuplicate) bucket.push(diag);
  }

  /**
   * Publish the accumulated diagnostics. Idempotent — calling twice is a no-op.
   */
  flush(): void {
    if (this.flushed) return;
    this.flushed = true;

    for (const [file, diags] of this.perFile) {
      const uri = vscode.Uri.file(file);
      const surviving = diags.filter((d) => !isAlreadyReportedByLsp(uri, d));
      if (surviving.length > 0) {
        this.collection.set(
          uri,
          surviving.map((parsed) => toVscodeDiagnostic(parsed)),
        );
      }
    }
  }
}

function toVscodeDiagnostic(parsed: ParsedDiagnostic): vscode.Diagnostic {
  // VS Code positions are zero-based, xcodebuild's are one-based. Clamp to 0
  // so an off-by-one (or `column: 0` from a tool that emits that) doesn't
  // create a negative position which VS Code rejects silently.
  const line = Math.max(0, parsed.line - 1);
  const column = Math.max(0, parsed.column - 1);
  const range = new vscode.Range(line, column, line, column);
  const severity = parsed.severity === "error" ? vscode.DiagnosticSeverity.Error : vscode.DiagnosticSeverity.Warning;
  const diagnostic = new vscode.Diagnostic(range, parsed.message, severity);
  diagnostic.source = "sweetpad";
  return diagnostic;
}

function isAlreadyReportedByLsp(uri: vscode.Uri, parsed: ParsedDiagnostic): boolean {
  // Only filter Swift files. Errors in ObjC, headers, plist, linker output,
  // or SPM checkouts (which sourcekit-lsp typically doesn't index) come
  // exclusively from xcodebuild and must not be dropped.
  if (!SWIFT_FILE_REGEX.test(uri.fsPath)) return false;

  let existing: readonly vscode.Diagnostic[];
  try {
    existing = vscode.languages.getDiagnostics(uri);
  } catch (error) {
    // Defensive: in older VS Code versions getDiagnostics for unknown URIs
    // has been observed to throw. The cost of letting a duplicate through
    // is much lower than losing a real error.
    commonLogger.debug("getDiagnostics threw, skipping LSP overlap filter", {
      uri: uri.toString(),
      error: error instanceof Error ? error.message : String(error),
    });
    return false;
  }

  const targetLine = Math.max(0, parsed.line - 1);
  const expectedSeverity =
    parsed.severity === "error" ? vscode.DiagnosticSeverity.Error : vscode.DiagnosticSeverity.Warning;
  return existing.some(
    (d) =>
      d.source !== undefined &&
      LSP_SOURCES.has(d.source) &&
      d.range.start.line === targetLine &&
      // Match severity too — if the LSP reported a warning and xcodebuild
      // reports an error at the same line (e.g. -Werror promoted it),
      // surface the error.
      d.severity === expectedSeverity,
  );
}
