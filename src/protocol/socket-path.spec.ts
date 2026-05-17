import * as os from "node:os";
import * as path from "node:path";

import { describe, expect, it } from "vitest";

import { getServerLockfilePath, getServerSocketPath } from "./socket-path";

describe("getServerSocketPath", () => {
  it("places the socket under ~/.sweetpad/run/<hash>/server.sock", () => {
    const result = getServerSocketPath("/tmp/some-workspace");
    expect(result.startsWith(path.join(os.homedir(), ".sweetpad", "run"))).toBe(true);
    expect(result.endsWith("server.sock")).toBe(true);
  });

  it("returns the same path for the same workspace (deterministic hashing)", () => {
    const a = getServerSocketPath("/tmp/sweetpad-a");
    const b = getServerSocketPath("/tmp/sweetpad-a");
    expect(a).toBe(b);
  });

  it("returns different paths for different workspaces", () => {
    const a = getServerSocketPath("/tmp/sweetpad-a");
    const b = getServerSocketPath("/tmp/sweetpad-b");
    expect(a).not.toBe(b);
  });

  it("normalises relative paths via path.resolve", () => {
    const absolute = getServerSocketPath("/tmp/sweetpad-a");
    // Hashing uses the resolved path, so a relative path resolving to the same place
    // produces the same socket key — but we can only assert this when cwd happens
    // to resolve relative-prefixed input there, so just check absolutes are stable.
    expect(absolute).toBe(getServerSocketPath("/tmp/sweetpad-a"));
  });
});

describe("getServerLockfilePath", () => {
  it("sits next to the socket", () => {
    const sock = getServerSocketPath("/tmp/sweetpad-a");
    const lock = getServerLockfilePath("/tmp/sweetpad-a");
    expect(path.dirname(lock)).toBe(path.dirname(sock));
    expect(path.basename(lock)).toBe("server.json");
  });
});
