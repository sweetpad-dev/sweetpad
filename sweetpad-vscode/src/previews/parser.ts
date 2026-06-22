/**
 * Lightweight scanner that finds SwiftUI previews in Swift source — both the
 * modern `#Preview` macro and the legacy `PreviewProvider` conformance.
 *
 * This is deliberately a line/regex scanner rather than a full SwiftSyntax
 * parse: SweetPad only needs the *location* of each preview (to place a
 * CodeLens and to build a navigable index), not a semantic understanding of
 * the view. It is intentionally conservative — it skips `//` line comments and
 * tolerates extra inheritance/trait arguments — and never throws on malformed
 * input.
 */

export type PreviewKind = "macro" | "provider";

export interface PreviewMatch {
  kind: PreviewKind;
  /**
   * Human label for the preview: the first string-literal argument of
   * `#Preview("…")`, or the type name for a `PreviewProvider`. `undefined` for
   * a bare `#Preview { … }`.
   */
  label?: string;
  /** 0-based line of the preview declaration. */
  line: number;
  /** 0-based column where the `#Preview` / declaration keyword starts. */
  character: number;
}

// `#Preview`, optionally followed by a parenthesised argument list. We capture
// the raw argument text so the caller can pull out the leading string label.
const MACRO_RE = /#Preview\b[ \t]*(\(([^)]*)\))?/;

// A `struct`/`class`/`enum` (or `extension`) whose inheritance clause contains
// `PreviewProvider`. The type name is captured; `final`/`public`/etc. modifiers
// before the keyword are allowed because the match doesn't anchor to column 0.
const PROVIDER_RE = /\b(?:struct|class|enum|extension)[ \t]+([A-Za-z_]\w*)[^{]*:[^{]*\bPreviewProvider\b/;

// First double-quoted string literal inside an argument list.
const FIRST_STRING_RE = /"((?:[^"\\]|\\.)*)"/;

/**
 * Return the byte offset of `//` that starts a line comment, ignoring `//`
 * inside string literals. Returns -1 when the line has no line comment.
 */
function lineCommentIndex(line: string): number {
  let inString = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === "\\") {
      i++; // skip escaped char
      continue;
    }
    if (ch === '"') {
      inString = !inString;
      continue;
    }
    if (!inString && ch === "/" && line[i + 1] === "/") {
      return i;
    }
  }
  return -1;
}

/**
 * Find every SwiftUI preview declaration in `text`, in source order.
 */
export function parsePreviews(text: string): PreviewMatch[] {
  const matches: PreviewMatch[] = [];
  const lines = text.split("\n");

  for (let lineNumber = 0; lineNumber < lines.length; lineNumber++) {
    const rawLine = lines[lineNumber];
    const commentAt = lineCommentIndex(rawLine);
    // Only consider the code portion of the line (before any `//`).
    const line = commentAt === -1 ? rawLine : rawLine.slice(0, commentAt);

    const macro = MACRO_RE.exec(line);
    if (macro) {
      const args = macro[2] ?? "";
      const label = FIRST_STRING_RE.exec(args)?.[1];
      matches.push({
        kind: "macro",
        label: label,
        line: lineNumber,
        character: macro.index,
      });
      continue;
    }

    const provider = PROVIDER_RE.exec(line);
    if (provider) {
      matches.push({
        kind: "provider",
        label: provider[1],
        line: lineNumber,
        character: provider.index,
      });
    }
  }

  return matches;
}

/**
 * Stable identifier for a preview, of the form `<relativePath>:<line>`. Used as
 * the key passed to the preview host so it knows which preview to render, and
 * as the de-dup key in the workspace index.
 */
export function previewId(relativePath: string, match: Pick<PreviewMatch, "line">): string {
  // 1-based line to match how editors and `#fileID:line` report locations.
  return `${relativePath}:${match.line + 1}`;
}
