import type * as vscode from "vscode";

import { DiagnosticAccumulator } from "./diagnostics";

// `recordLine` reads build output and never touches the editor surface — the
// collection is only used by `flush`, which these tests don't call.
const accumulator = (mode: "xcodebuild" | "xcbeautify" = "xcodebuild") =>
  new DiagnosticAccumulator({} as vscode.DiagnosticCollection, mode);

describe("DiagnosticAccumulator.recordLine", () => {
  describe("a clang message wrapped to the terminal width", () => {
    // Real xcodebuild output at 80 columns. clang fits the message into the
    // width left after the file path, so a long enough workspace path pushes
    // all of it onto the following line.
    const header = "/tmp/w/p/widget.m:5:28: error: ";
    const continuation = "      use of undeclared identifier 'undeclared_symbol_here'";
    const snippet = "    5 | void broken(void) { return undeclared_symbol_here; }";
    const caret = "      |                            ^~~~~~~~~~~~~~~~~~~~~~";

    it("holds the header back rather than publishing an empty squiggle", () => {
      expect(accumulator().recordLine(header)).toBeNull();
    });

    it("publishes it once the next line supplies the message", () => {
      const acc = accumulator();
      acc.recordLine(header);

      expect(acc.recordLine(continuation)).toMatchObject({
        file: "/tmp/w/p/widget.m",
        line: 5,
        column: 28,
        severity: "error",
        message: "use of undeclared identifier 'undeclared_symbol_here'",
      });
    });

    it("keeps the source snippet and caret rows out of the message", () => {
      const acc = accumulator();
      acc.recordLine(header);
      acc.recordLine(continuation);

      expect(acc.recordLine(snippet)).toBeNull();
      expect(acc.recordLine(caret)).toBeNull();
    });

    it("drops a header the following line never completes", () => {
      const acc = accumulator();
      acc.recordLine(header);

      // The snippet row is the next line when clang has nothing to wrap, so a
      // header followed by one is abandoned rather than given the source text.
      expect(acc.recordLine(snippet)).toBeNull();
    });
  });

  describe("a diagnostic that arrived whole", () => {
    // swiftc doesn't wrap, and echoes the offending line beneath the
    // diagnostic with no `N |` gutter — the shape a broader continuation rule
    // would swallow into the message. Nothing is held back here, so it can't.
    const swiftError = "/tmp/w/App.swift:26:22: error: cannot convert value of type 'String' to specified type 'Int'";
    const sourceEcho = '    let value: Int = "a string literal long enough that the compiler has plenty to say"';

    it("publishes immediately instead of waiting for a continuation", () => {
      expect(accumulator().recordLine(swiftError)).toMatchObject({
        file: "/tmp/w/App.swift",
        message: "cannot convert value of type 'String' to specified type 'Int'",
      });
    });

    it("does not absorb the echoed source line", () => {
      const acc = accumulator();
      acc.recordLine(swiftError);

      expect(acc.recordLine(sourceEcho)).toBeNull();
    });
  });

  it("starts a fresh diagnostic when one header follows another", () => {
    const acc = accumulator();
    acc.recordLine("/tmp/w/a.m:1:1: error: ");

    expect(acc.recordLine("/tmp/w/b.m:2:2: error: something else")).toMatchObject({
      file: "/tmp/w/b.m",
      message: "something else",
    });
  });

  it("keeps the first of two diagnostics at the same position", () => {
    const acc = accumulator();

    expect(acc.recordLine("/tmp/w/a.m:1:1: error: first")).not.toBeNull();
    expect(acc.recordLine("/tmp/w/a.m:1:1: error: First")).toBeNull();
  });
});
