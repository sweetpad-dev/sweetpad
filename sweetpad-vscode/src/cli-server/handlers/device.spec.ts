import { beforeEach, describe, expect, it, vi } from "vitest";

import type { RpcContext } from "./context";
import { deviceLaunch } from "./device";

// Capture every argv handed to `execa("xcrun", [...])` so we can inspect the
// exact command line SweetPad builds for `devicectl`. `vi.mock` is hoisted
// above the imports, so `deviceLaunch` picks up this fake execa.
const execaCalls: string[][] = [];

vi.mock("execa", () => ({
  execa: (_file: string, args: string[]) => {
    execaCalls.push(args);
    // devicectl prints the launched process identifier on success; give the
    // handler something parseable so it doesn't blow up before we assert.
    return Promise.resolve({ stdout: "processIdentifier: 4242" });
  },
}));

function makeCtx(): RpcContext {
  return {
    workspacePath: "/tmp/ws",
    extensionVersion: "test",
    workspaceState: {} as RpcContext["workspaceState"],
    buildManager: {} as RpcContext["buildManager"],
    destinationsManager: {} as RpcContext["destinationsManager"],
    buildRegistry: {} as RpcContext["buildRegistry"],
    vscodeContext: {} as RpcContext["vscodeContext"],
    configKeys: [],
  };
}

describe("device.launch", () => {
  beforeEach(() => {
    execaCalls.length = 0;
  });

  // Regression test for issue #296: launching on a physical device via
  // devicectl failed when the scheme had "Arguments Passed On Launch" values
  // that start with `-`, because SweetPad forwarded them without a `--`
  // separator and devicectl parsed them as its own options.
  it("inserts a `--` separator before dash-prefixed app arguments (issue #296)", async () => {
    await deviceLaunch(
      {
        deviceId: "00008120-000000000000000E",
        bundleId: "com.example.app",
        args: ["-com.example.someFlag", "-com.example.displayLog"],
      },
      makeCtx(),
    );

    const argv = execaCalls[0];
    const bundleIdx = argv.indexOf("com.example.app");
    const sepIdx = argv.indexOf("--");

    // A `--` terminator must sit right after the bundle id...
    expect(sepIdx).toBe(bundleIdx + 1);
    // ...and every forwarded app argument must come after it, so devicectl
    // treats them as app arguments rather than its own options.
    expect(argv.slice(sepIdx + 1)).toEqual(["-com.example.someFlag", "-com.example.displayLog"]);
  });

  it("does not add a `--` separator when there are no launch arguments", async () => {
    await deviceLaunch({ deviceId: "device-id", bundleId: "com.example.app" }, makeCtx());

    expect(execaCalls[0]).not.toContain("--");
    expect(execaCalls[0].at(-1)).toBe("com.example.app");
  });
});
