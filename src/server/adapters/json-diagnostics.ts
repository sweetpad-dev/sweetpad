import type { DiagnosticAccumulator, DiagnosticsCollector, ParseMode } from "../../core/build/diagnostics-types";
import { parseDiagnosticLine, type ParsedDiagnostic } from "../../core/build/diagnostics-parser";

/**
 * In-memory `DiagnosticsCollector` for the CLI/server. Buffers parsed
 * diagnostics from a single build; the server pulls them off via `drain()`
 * after the build finishes and inlines them in the response.
 *
 * No LSP-overlap filtering (that's a VS Code concern). No file-edit
 * invalidation (no editor here).
 */
export class JsonDiagnosticsCollector implements DiagnosticsCollector {
  private current: JsonDiagnosticAccumulator | undefined;

  beginBuild(options: { mode: ParseMode }): DiagnosticAccumulator {
    this.current = new JsonDiagnosticAccumulator(options.mode);
    return this.current;
  }

  /** Pull and clear the diagnostics collected from the most recent build. */
  drain(): ParsedDiagnostic[] {
    const acc = this.current;
    this.current = undefined;
    return acc ? acc.snapshot() : [];
  }
}

class JsonDiagnosticAccumulator implements DiagnosticAccumulator {
  private readonly diagnostics: ParsedDiagnostic[] = [];
  private flushed = false;

  constructor(private readonly mode: ParseMode) {}

  recordLine(rawLine: string): void {
    const diag = parseDiagnosticLine(rawLine, this.mode);
    if (!diag) return;

    // Same dedup as the VS Code path: same (file, line, column, severity)
    // collapses to first-seen. Catches Swift's frontend/driver double-emit
    // and xcbeautify's raw-line passthrough.
    const isDuplicate = this.diagnostics.some(
      (existing) =>
        existing.file === diag.file &&
        existing.line === diag.line &&
        existing.column === diag.column &&
        existing.severity === diag.severity,
    );
    if (!isDuplicate) this.diagnostics.push(diag);
  }

  flush(): void {
    this.flushed = true;
  }

  snapshot(): ParsedDiagnostic[] {
    return [...this.diagnostics];
  }
}
