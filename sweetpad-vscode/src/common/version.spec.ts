import { isVersionAtLeast, parseVersion } from "./version";

describe("parseVersion", () => {
  it("reads a bare version", () => {
    expect(parseVersion("0.1.5")).toEqual([0, 1, 5]);
  });

  it("reads the version out of a whole --version line", () => {
    expect(parseVersion("sweetpad 0.1.5")).toEqual([0, 1, 5]);
  });

  it("reads a dev build as the version it was built from", () => {
    // `build.rs` stamps anything off-tag as `<version>-dev+<sha>`; the sha can
    // start with digits, so the first match is what has to win.
    expect(parseVersion("sweetpad 0.1.5-dev+4e8e57fa")).toEqual([0, 1, 5]);
  });

  it("returns nothing for input carrying no version", () => {
    expect(parseVersion("sweetpad")).toBeUndefined();
  });
});

describe("isVersionAtLeast", () => {
  it.each([
    ["0.1.5", "0.1.5", true],
    ["0.1.6", "0.1.5", true],
    ["0.2.0", "0.1.9", true],
    ["1.0.0", "0.9.9", true],
    ["0.1.4", "0.1.5", false],
    ["0.0.9", "0.1.0", false],
  ])("%s vs minimum %s -> %s", (actual, minimum, expected) => {
    expect(isVersionAtLeast(actual, minimum)).toBe(expected);
  });

  it("compares numerically rather than as text", () => {
    // "0.1.10" sorts before "0.1.9" as a string, and after it as a version.
    expect(isVersionAtLeast("0.1.10", "0.1.9")).toBe(true);
  });

  it("treats unreadable input as not new enough", () => {
    expect(isVersionAtLeast("unknown", "0.1.5")).toBe(false);
  });
});
