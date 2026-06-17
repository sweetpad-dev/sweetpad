import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { ENV_PREVIEW_APPEARANCE, ENV_PREVIEW_ID, PREVIEW_HOST_BOOTSTRAP } from "./host-bootstrap";

const GENERATED = "examples/preview-bridge/Sources/GeneratedHost/SweetPadPreviewHost.swift";

describe("PREVIEW_HOST_BOOTSTRAP", () => {
  it("interpolates the env var names (no leftover template syntax)", () => {
    expect(PREVIEW_HOST_BOOTSTRAP).toContain(`environment["${ENV_PREVIEW_ID}"]`);
    expect(PREVIEW_HOST_BOOTSTRAP).toContain(`environment["${ENV_PREVIEW_APPEARANCE}"]`);
    // A stray "${" would mean an interpolation leaked into the emitted Swift.
    expect(PREVIEW_HOST_BOOTSTRAP).not.toContain("${");
  });

  it("preserves Swift backslashes (String.raw)", () => {
    // Key paths and string interpolations must survive into the Swift source.
    expect(PREVIEW_HOST_BOOTSTRAP).toContain("offset(of: \\.parent)");
    expect(PREVIEW_HOST_BOOTSTRAP).toContain("offset(of: \\.name)");
    expect(PREVIEW_HOST_BOOTSTRAP).toContain('"\\(parentName).\\(typeName)"');
    // A real newline escape inside a Swift string literal (not an actual newline).
    expect(PREVIEW_HOST_BOOTSTRAP).toContain("no #Preview matched\\n\\(id)");
  });

  it("contains no backticks (would terminate the JS template / clutter Swift)", () => {
    expect(PREVIEW_HOST_BOOTSTRAP).not.toContain("`");
  });

  it("includes the discovery + resolution surface", () => {
    for (const token of [
      "func getPreviewTypes()",
      "__swift5_proto",
      "MH_DYLIB_IN_CACHE",
      "any PreviewRegistry.Type",
      "public static func rootView() -> AnyView?",
    ]) {
      expect(PREVIEW_HOST_BOOTSTRAP, `missing ${token}`).toContain(token);
    }
  });

  it("matches the committed Swift file CI compiles (no drift)", () => {
    const committed = readFileSync(GENERATED, "utf8");
    expect(
      committed,
      "Generated Swift is stale — run `node scripts/emit-preview-host.mjs` and commit the result.",
    ).toBe(PREVIEW_HOST_BOOTSTRAP);
  });
});
