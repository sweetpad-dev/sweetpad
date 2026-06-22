import { getShellDeveloperDir, refreshShellEnv } from "./shell-env";

describe("getShellDeveloperDir", () => {
  const originalEnv = process.env;

  beforeEach(() => {
    process.env = { ...originalEnv };
  });

  afterAll(() => {
    process.env = originalEnv;
  });

  it("returns the login shell's DEVELOPER_DIR (inherited from the host env)", async () => {
    process.env.DEVELOPER_DIR = "/Applications/Xcode-beta.app/Contents/Developer";
    // Re-resolve so the probe shell (or its process.env fallback) sees the
    // value set above instead of a cached earlier resolution.
    await refreshShellEnv();

    await expect(getShellDeveloperDir()).resolves.toBe("/Applications/Xcode-beta.app/Contents/Developer");
  });

  it("returns undefined when no shell exports one", async () => {
    delete process.env.DEVELOPER_DIR;
    await refreshShellEnv();

    await expect(getShellDeveloperDir()).resolves.toBeUndefined();
  });
});
