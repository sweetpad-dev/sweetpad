import { describe, expect, it } from "vitest";

import { parsePreviews, previewId } from "./parser";

describe("parsePreviews", () => {
  it("finds a bare #Preview macro", () => {
    const src = ["import SwiftUI", "", "#Preview {", "    ContentView()", "}"].join("\n");
    expect(parsePreviews(src)).toEqual([{ kind: "macro", label: undefined, line: 2, character: 0 }]);
  });

  it('extracts the string label from #Preview("…")', () => {
    const src = '#Preview("Dark mode") {\n    ContentView().preferredColorScheme(.dark)\n}';
    expect(parsePreviews(src)).toEqual([{ kind: "macro", label: "Dark mode", line: 0, character: 0 }]);
  });

  it("ignores trait arguments when there is no label", () => {
    const src = '#Preview(traits: .sizeThatFitsLayout) {\n    Button("Hi") {}\n}';
    expect(parsePreviews(src)).toEqual([{ kind: "macro", label: undefined, line: 0, character: 0 }]);
  });

  it("captures the leading label even with trailing traits", () => {
    const src = '#Preview("Compact", traits: .sizeThatFitsLayout) { Text("x") }';
    expect(parsePreviews(src)).toEqual([{ kind: "macro", label: "Compact", line: 0, character: 0 }]);
  });

  it("records the column for an indented #Preview", () => {
    const src = 'enum Demo {\n    #Preview { Text("x") }\n}';
    expect(parsePreviews(src)).toEqual([{ kind: "macro", label: undefined, line: 1, character: 4 }]);
  });

  it("finds a legacy PreviewProvider and captures its type name", () => {
    const src = [
      "struct ContentView_Previews: PreviewProvider {",
      "    static var previews: some View {",
      "        ContentView()",
      "    }",
      "}",
    ].join("\n");
    expect(parsePreviews(src)).toEqual([{ kind: "provider", label: "ContentView_Previews", line: 0, character: 0 }]);
  });

  it("finds a PreviewProvider with extra protocol conformances", () => {
    const src = "final class MyPreviews: SomeBase, PreviewProvider {\n}";
    const result = parsePreviews(src);
    expect(result).toEqual([{ kind: "provider", label: "MyPreviews", line: 0, character: 6 }]);
  });

  it("skips previews inside line comments", () => {
    const src = ["// #Preview { Old() }", "#Preview { New() }"].join("\n");
    expect(parsePreviews(src)).toEqual([{ kind: "macro", label: undefined, line: 1, character: 0 }]);
  });

  it("does not treat // inside a string as a comment", () => {
    const src = 'let url = "https://example.com" // #Preview { ignored }\n#Preview { Real() }';
    expect(parsePreviews(src)).toEqual([{ kind: "macro", label: undefined, line: 1, character: 0 }]);
  });

  it("finds multiple previews in source order", () => {
    const src = [
      '#Preview("A") { A() }',
      "",
      '#Preview("B") { B() }',
      "",
      "struct C_Previews: PreviewProvider { static var previews: some View { C() } }",
    ].join("\n");
    expect(parsePreviews(src)).toEqual([
      { kind: "macro", label: "A", line: 0, character: 0 },
      { kind: "macro", label: "B", line: 2, character: 0 },
      { kind: "provider", label: "C_Previews", line: 4, character: 0 },
    ]);
  });

  it("returns an empty list for source without previews", () => {
    expect(parsePreviews('struct ContentView: View {\n  var body: some View { Text("hi") }\n}')).toEqual([]);
  });

  it("does not crash on empty input", () => {
    expect(parsePreviews("")).toEqual([]);
  });
});

describe("previewId", () => {
  it("builds a 1-based path:line identifier", () => {
    expect(previewId("Sources/Feature/ContentView.swift", { line: 9 })).toBe("Sources/Feature/ContentView.swift:10");
  });
});
