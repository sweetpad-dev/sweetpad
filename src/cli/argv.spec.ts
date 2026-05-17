import { describe, expect, it } from "vitest";

import { getBool, getString, parseArgv } from "./argv";

describe("parseArgv", () => {
  it("parses --key=value", () => {
    const args = parseArgv(["--scheme=MyApp", "--destination=ABCD"]);
    expect(getString(args, "scheme")).toBe("MyApp");
    expect(getString(args, "destination")).toBe("ABCD");
  });

  it("parses --key value", () => {
    const args = parseArgv(["--scheme", "MyApp"]);
    expect(getString(args, "scheme")).toBe("MyApp");
  });

  it("parses boolean flags", () => {
    const args = parseArgv(["--debug"]);
    expect(getBool(args, "debug")).toBe(true);
    expect(getBool(args, "release")).toBe(false);
  });

  it("collects positional args into _", () => {
    const args = parseArgv(["pos1", "--scheme=X", "pos2"]);
    expect(args._).toEqual(["pos1", "pos2"]);
  });

  it("treats the value-less flag before another flag as boolean", () => {
    const args = parseArgv(["--debug", "--scheme=X"]);
    expect(getBool(args, "debug")).toBe(true);
    expect(getString(args, "scheme")).toBe("X");
  });
});
