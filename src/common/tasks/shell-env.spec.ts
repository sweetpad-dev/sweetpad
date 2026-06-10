import * as sweetpadLib from "@sweetpad/lib";
import type { Mock } from "vitest";

import { syncDeveloperDirIntoProcessEnv } from "./shell-env";

vi.mock("@sweetpad/lib", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@sweetpad/lib")>()),
  flushXcodeCache: vi.fn(),
}));

const mockFlushXcodeCache = sweetpadLib.flushXcodeCache as Mock;

describe("syncDeveloperDirIntoProcessEnv", () => {
  const originalEnv = process.env;

  beforeEach(() => {
    vi.resetAllMocks();
    process.env = { ...originalEnv };
  });

  afterAll(() => {
    process.env = originalEnv;
  });

  it("propagates the shell's DEVELOPER_DIR and flushes the resolver's Xcode memo", () => {
    delete process.env.DEVELOPER_DIR;

    syncDeveloperDirIntoProcessEnv({ DEVELOPER_DIR: "/Applications/Xcode-beta.app/Contents/Developer" });

    expect(process.env.DEVELOPER_DIR).toBe("/Applications/Xcode-beta.app/Contents/Developer");
    expect(mockFlushXcodeCache).toHaveBeenCalledTimes(1);
  });

  it("overwrites a stale process value when the shell disagrees", () => {
    process.env.DEVELOPER_DIR = "/Applications/Xcode-old.app/Contents/Developer";

    syncDeveloperDirIntoProcessEnv({ DEVELOPER_DIR: "/Applications/Xcode-new.app/Contents/Developer" });

    expect(process.env.DEVELOPER_DIR).toBe("/Applications/Xcode-new.app/Contents/Developer");
    expect(mockFlushXcodeCache).toHaveBeenCalledTimes(1);
  });

  it("does nothing when the values already agree", () => {
    process.env.DEVELOPER_DIR = "/Applications/Xcode.app/Contents/Developer";

    syncDeveloperDirIntoProcessEnv({ DEVELOPER_DIR: "/Applications/Xcode.app/Contents/Developer" });

    expect(process.env.DEVELOPER_DIR).toBe("/Applications/Xcode.app/Contents/Developer");
    expect(mockFlushXcodeCache).not.toHaveBeenCalled();
  });

  it("never deletes an existing process value when the shell env lacks one", () => {
    process.env.DEVELOPER_DIR = "/Applications/Xcode.app/Contents/Developer";

    syncDeveloperDirIntoProcessEnv({});

    expect(process.env.DEVELOPER_DIR).toBe("/Applications/Xcode.app/Contents/Developer");
    expect(mockFlushXcodeCache).not.toHaveBeenCalled();
  });
});
