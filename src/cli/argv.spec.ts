import { parseArgv } from "./argv";

describe("cli/argv", () => {
  it("treats the first dotted token as the RPC method name", () => {
    const r = parseArgv(["scheme.list"]);
    expect(r.method).toBe("scheme.list");
    expect(r.positionals).toEqual([]);
    expect(r.help).toBe(false);
  });

  it("collects remaining tokens as positionals for the method", () => {
    const r = parseArgv(["scheme.set", "MyApp"]);
    expect(r.method).toBe("scheme.set");
    expect(r.positionals).toEqual(["MyApp"]);
  });

  it("does not promote second positional to action when the first has a dot", () => {
    const r = parseArgv(["simulator.install", "udid-abc", "/path/to/app.app"]);
    expect(r.method).toBe("simulator.install");
    expect(r.positionals).toEqual(["udid-abc", "/path/to/app.app"]);
  });

  it("leaves method undefined for a bare (non-dotted) first token", () => {
    const r = parseArgv(["servers", "list"]);
    expect(r.method).toBeUndefined();
    expect(r.positionals).toEqual(["list"]);
  });

  it("supports --flag=value form", () => {
    const r = parseArgv(["build.wait", "b1", "--timeout=300"]);
    expect(r.positionals).toEqual(["b1"]);
    expect(r.flags.timeout).toBe("300");
  });

  it("supports --flag value (no equals) form", () => {
    const r = parseArgv(["build.list", "--limit", "5"]);
    expect(r.flags.limit).toBe("5");
  });

  it("treats trailing --flag as boolean", () => {
    const r = parseArgv(["build.start", "build", "--debug"]);
    expect(r.positionals).toEqual(["build"]);
    expect(r.flags.debug).toBe(true);
  });

  it("detects --help and -h", () => {
    expect(parseArgv(["--help"]).help).toBe(true);
    expect(parseArgv(["-h"]).help).toBe(true);
  });

  it("extracts --raw as a boolean toggle", () => {
    const r = parseArgv(["--raw", "meta.version"]);
    expect(r.raw).toBe(true);
    expect(r.method).toBe("meta.version");
  });
});
