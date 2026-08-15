import { continuationMessage, isHeaderOnly, parseDiagnosticLine, type ParsedDiagnostic } from "./diagnostics-parser";

describe("parseDiagnosticLine", () => {
  describe("xcbeautify error format", () => {
    it("parses the unicode emoji form", () => {
      const result = parseDiagnosticLine("❌ /path/to/Foo.swift:10:5: cannot find 'bar' in scope");
      expect(result).toEqual({
        file: "/path/to/Foo.swift",
        line: 10,
        column: 5,
        severity: "error",
        message: "cannot find 'bar' in scope",
        source: "xcbeautify",
      });
    });

    it("parses the ascii form [x]", () => {
      const result = parseDiagnosticLine("[x] /path/to/Foo.swift:42:9: use of undeclared identifier 'q'");
      expect(result).toEqual({
        file: "/path/to/Foo.swift",
        line: 42,
        column: 9,
        severity: "error",
        message: "use of undeclared identifier 'q'",
        source: "xcbeautify",
      });
    });

    it("tolerates no space between the prefix and the path", () => {
      const result = parseDiagnosticLine("❌/path/to/Foo.swift:1:1: boom");
      expect(result?.severity).toBe("error");
      expect(result?.file).toBe("/path/to/Foo.swift");
    });

    it("tolerates extra whitespace between the prefix and the path", () => {
      const result = parseDiagnosticLine("[x]   /path/to/Foo.swift:1:1: boom");
      expect(result?.severity).toBe("error");
    });

    it("preserves colons inside the diagnostic message", () => {
      const result = parseDiagnosticLine("❌ /path/to/Foo.swift:10:5: expected ':' after type name");
      expect(result?.message).toBe("expected ':' after type name");
    });
  });

  describe("xcbeautify warning format", () => {
    it("parses the unicode emoji form", () => {
      const result = parseDiagnosticLine("⚠️  /path/to/Foo.swift:12:1: variable 'x' was never used");
      expect(result).toEqual({
        file: "/path/to/Foo.swift",
        line: 12,
        column: 1,
        severity: "warning",
        message: "variable 'x' was never used",
        source: "xcbeautify",
      });
    });

    it("parses the ascii form [!]", () => {
      const result = parseDiagnosticLine("[!] /path/to/Foo.swift:7:14: deprecated API");
      expect(result).toEqual({
        file: "/path/to/Foo.swift",
        line: 7,
        column: 14,
        severity: "warning",
        message: "deprecated API",
        source: "xcbeautify",
      });
    });
  });

  describe("raw xcodebuild format", () => {
    it("parses an error line", () => {
      const result = parseDiagnosticLine("/path/to/Foo.swift:10:5: error: cannot find 'bar' in scope");
      expect(result).toEqual({
        file: "/path/to/Foo.swift",
        line: 10,
        column: 5,
        severity: "error",
        message: "cannot find 'bar' in scope",
        source: "xcodebuild",
      });
    });

    it("parses a warning line", () => {
      const result = parseDiagnosticLine("/path/to/Foo.swift:12:1: warning: variable 'x' was never used");
      expect(result).toEqual({
        file: "/path/to/Foo.swift",
        line: 12,
        column: 1,
        severity: "warning",
        message: "variable 'x' was never used",
        source: "xcodebuild",
      });
    });

    it("ignores `note:` continuation lines", () => {
      // xcodebuild emits notes as `file:line:col: note: msg` — we deliberately
      // don't surface them as standalone diagnostics; future work could attach
      // them as relatedInformation on the preceding error.
      const result = parseDiagnosticLine("/path/to/Foo.swift:10:5: note: see declaration of 'bar'");
      expect(result).toBeNull();
    });
  });

  describe("path handling", () => {
    it("rejects relative paths", () => {
      // The fileLocation in the old problem matcher was "absolute"; matching
      // unanchored paths would let `note: src/Foo.swift:1:2: error: bar`-style
      // lines sneak through.
      const result = parseDiagnosticLine("src/Foo.swift:10:5: error: bad");
      expect(result).toBeNull();
    });

    it("accepts deeply nested absolute paths", () => {
      const result = parseDiagnosticLine(
        "/Users/me/Library/Developer/Xcode/DerivedData/App-abc/SourcePackages/Foo.swift:1:1: error: x",
      );
      expect(result?.file).toBe("/Users/me/Library/Developer/Xcode/DerivedData/App-abc/SourcePackages/Foo.swift");
    });

    it("accepts paths with spaces", () => {
      const result = parseDiagnosticLine("/Users/me/My Project/Foo.swift:3:4: error: oops");
      expect(result?.file).toBe("/Users/me/My Project/Foo.swift");
      expect(result?.line).toBe(3);
      expect(result?.column).toBe(4);
    });

    it("does not match when the path contains a colon", () => {
      // Colons inside paths break the regex (and shouldn't occur on macOS) —
      // documenting the limit so a future contributor doesn't think it's a bug.
      const result = parseDiagnosticLine("/weird:path/Foo.swift:10:5: error: bad");
      expect(result).toBeNull();
    });
  });

  describe("non-matching lines", () => {
    it("returns null for an empty line", () => {
      expect(parseDiagnosticLine("")).toBeNull();
    });

    it("returns null for whitespace-only lines", () => {
      expect(parseDiagnosticLine("   \t  ")).toBeNull();
    });

    it("returns null for ordinary build progress lines", () => {
      expect(parseDiagnosticLine("Compile Foo.swift (in target 'App')")).toBeNull();
      expect(parseDiagnosticLine("** BUILD SUCCEEDED **")).toBeNull();
      expect(parseDiagnosticLine("CompileSwiftSources normal x86_64")).toBeNull();
    });

    it("returns null when the emoji prefix appears mid-line", () => {
      // Anchor is required — otherwise xcbeautify summary lines like
      // "Build failed ❌ 1 error" would match.
      expect(parseDiagnosticLine("Build failed ❌ /path/Foo.swift:1:1: msg")).toBeNull();
    });
  });

  describe("input sanitization", () => {
    it("strips trailing CR (CRLF terminators)", () => {
      const result = parseDiagnosticLine("/path/to/Foo.swift:10:5: error: msg\r");
      expect(result?.message).toBe("msg");
    });

    it("strips ANSI color escape sequences", () => {
      // v3's line buffer strips ANSI before us, but v2 does not — defend either way.
      const line = "\x1b[31m❌\x1b[0m /path/to/Foo.swift:10:5: \x1b[1mboom\x1b[0m";
      const result = parseDiagnosticLine(line);
      expect(result).toEqual({
        file: "/path/to/Foo.swift",
        line: 10,
        column: 5,
        severity: "error",
        message: "boom",
        source: "xcbeautify",
      });
    });

    it("trims leading whitespace", () => {
      const result = parseDiagnosticLine("   /path/to/Foo.swift:10:5: error: msg");
      expect(result?.file).toBe("/path/to/Foo.swift");
    });

    it("handles multi-digit line and column numbers", () => {
      const result = parseDiagnosticLine("/path/Foo.swift:1234:567: error: msg");
      expect(result?.line).toBe(1234);
      expect(result?.column).toBe(567);
    });
  });

  describe("mode parameter", () => {
    const xcbeautifyLine = "❌ /path/Foo.swift:10:5: cannot find 'bar' in scope";
    const xcodebuildLine = "/path/Foo.swift:10:5: error: cannot find 'bar' in scope";

    describe("mode = 'xcbeautify'", () => {
      it("matches xcbeautify-formatted lines", () => {
        const result = parseDiagnosticLine(xcbeautifyLine, "xcbeautify");
        expect(result?.source).toBe("xcbeautify");
      });

      it("ignores raw xcodebuild lines (which xcbeautify would have reformatted)", () => {
        // When xcbeautify is in the pipeline, both forms can appear in the
        // captured stream — the raw one is a near-duplicate of the formatted
        // one (often differing only by message casing). Filtering it out at
        // parse time avoids two diagnostics for the same error.
        const result = parseDiagnosticLine(xcodebuildLine, "xcbeautify");
        expect(result).toBeNull();
      });
    });

    describe("mode = 'xcodebuild'", () => {
      it("matches raw xcodebuild lines", () => {
        const result = parseDiagnosticLine(xcodebuildLine, "xcodebuild");
        expect(result?.source).toBe("xcodebuild");
      });

      it("ignores xcbeautify-formatted lines (xcbeautify is not in the pipeline)", () => {
        const result = parseDiagnosticLine(xcbeautifyLine, "xcodebuild");
        expect(result).toBeNull();
      });
    });

    describe("mode = 'auto' (default)", () => {
      it("matches both formats", () => {
        expect(parseDiagnosticLine(xcbeautifyLine, "auto")?.source).toBe("xcbeautify");
        expect(parseDiagnosticLine(xcodebuildLine, "auto")?.source).toBe("xcodebuild");
      });

      it("is the default when no mode is provided", () => {
        // All 22 existing tests above implicitly verify this; this is a
        // belt-and-braces assertion against accidental default changes.
        expect(parseDiagnosticLine(xcbeautifyLine)?.source).toBe("xcbeautify");
        expect(parseDiagnosticLine(xcodebuildLine)?.source).toBe("xcodebuild");
      });
    });
  });
});

// clang writes to a TTY whenever no xcbeautify sits in the pipe, and wraps its
// own diagnostics to the terminal width when it does. Past that width the
// message lands on the following line, leaving a header that ends at "error:"
// — the shape every build produced while this went unhandled.
describe("clang diagnostics wrapped at the terminal width", () => {
  const header = "/path/to/Foo.m:5:28: error: ";
  const continuation = "      use of undeclared identifier 'undeclared_symbol_here'";

  it("parses a header whose message wrapped away, and marks it incomplete", () => {
    const result = parseDiagnosticLine(header, "xcodebuild");

    expect(result).toMatchObject({ file: "/path/to/Foo.m", line: 5, column: 28, severity: "error", message: "" });
    expect(isHeaderOnly(result as ParsedDiagnostic)).toBe(true);
  });

  it("still marks a diagnostic that kept its message as complete", () => {
    const result = parseDiagnosticLine(`${header}use of undeclared identifier 'x'`, "xcodebuild");

    expect(isHeaderOnly(result as ParsedDiagnostic)).toBe(false);
  });

  it("reads the wrapped remainder off the next line", () => {
    expect(continuationMessage(continuation)).toBe("use of undeclared identifier 'undeclared_symbol_here'");
  });

  it("strips colour and a trailing carriage return from the remainder", () => {
    expect(continuationMessage("   \u001b[1muse of undeclared identifier\u001b[0m\r")).toBe(
      "use of undeclared identifier",
    );
  });

  it("does not read the source snippet or caret rows as the message", () => {
    // clang prints these directly beneath the diagnostic; taking either as the
    // message would put source code in the Problems panel.
    expect(continuationMessage("    1 | void broken(void) { return x; }")).toBeNull();
    expect(continuationMessage("      |                            ^~~~")).toBeNull();
  });

  it("does not treat an unindented line as a continuation", () => {
    expect(continuationMessage("1 error generated.")).toBeNull();
  });

  it("does not swallow the next diagnostic as a continuation", () => {
    expect(continuationMessage("  /path/to/Bar.m:9:1: error: something else")).toBeNull();
  });
});
