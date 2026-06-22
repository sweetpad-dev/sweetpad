import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import type { WorkspaceStateService } from "../../common/workspace-state";
import type { RpcContext } from "./context";
import { workspaceDetect, workspaceRecent, workspaceUse } from "./workspace";

function makeContext(opts: { workspacePath: string; state?: Map<string, unknown> }): RpcContext {
  const state = opts.state ?? new Map<string, unknown>();
  const ws: Pick<WorkspaceStateService, "get" | "update" | "rawGet" | "rawUpdate" | "rawKeys" | "reset"> = {
    get: ((key: string) => state.get(key)) as WorkspaceStateService["get"],
    update: ((key: string, value: unknown) => {
      if (value === undefined) state.delete(key);
      else state.set(key, value);
    }) as WorkspaceStateService["update"],
    rawGet: (key: string) => state.get(key),
    rawUpdate: async (key: string, value: unknown) => {
      if (value === undefined) state.delete(key);
      else state.set(key, value);
    },
    rawKeys: () => [...state.keys()],
    reset: () => state.clear(),
  };
  return {
    workspacePath: opts.workspacePath,
    extensionVersion: "test",
    workspaceState: ws as WorkspaceStateService,
    buildManager: {} as RpcContext["buildManager"],
    destinationsManager: {} as RpcContext["destinationsManager"],
    buildRegistry: {} as RpcContext["buildRegistry"],
    vscodeContext: {} as RpcContext["vscodeContext"],
    configKeys: [],
  };
}

describe("workspace handlers", () => {
  let tmp: string;

  beforeEach(async () => {
    tmp = await fs.mkdtemp(path.join(os.tmpdir(), "sw-workspace-spec-"));
  });

  afterEach(async () => {
    await fs.rm(tmp, { recursive: true, force: true });
  });

  it("workspace.detect finds .xcworkspace, .xcodeproj, and Package.swift with workspaces first", async () => {
    await fs.mkdir(path.join(tmp, "App.xcodeproj"));
    await fs.mkdir(path.join(tmp, "App.xcworkspace"));
    await fs.writeFile(path.join(tmp, "Package.swift"), "// swift-tools-version:5.5");
    // Nested project that should still be found
    await fs.mkdir(path.join(tmp, "sub"));
    await fs.mkdir(path.join(tmp, "sub", "Nested.xcodeproj"));

    const ctx = makeContext({ workspacePath: tmp });
    const out = await workspaceDetect({}, ctx);
    const kinds = out.candidates.map((c) => c.kind);
    expect(kinds[0]).toBe("xcworkspace");
    expect(kinds).toContain("xcodeproj");
    expect(kinds).toContain("spm");
    expect(out.candidates.some((c) => c.path.endsWith("Nested.xcodeproj"))).toBe(true);
  });

  it("workspace.detect skips node_modules and hidden directories", async () => {
    await fs.mkdir(path.join(tmp, "node_modules"));
    await fs.mkdir(path.join(tmp, "node_modules", "Lib.xcodeproj"));
    await fs.mkdir(path.join(tmp, ".vscode"));
    await fs.mkdir(path.join(tmp, "Real.xcodeproj"));

    const ctx = makeContext({ workspacePath: tmp });
    const out = await workspaceDetect({}, ctx);
    expect(out.candidates.map((c) => c.path)).toEqual([path.join(tmp, "Real.xcodeproj")]);
  });

  it("workspace.use writes the path into state and tracks it in recent (newest first)", async () => {
    const xc = path.join(tmp, "App.xcworkspace");
    await fs.mkdir(xc);
    const ctx = makeContext({ workspacePath: tmp });
    const first = await workspaceUse({ path: xc }, ctx);
    expect(first.workspacePath).toBe(xc);
    expect(first.recent).toEqual([xc]);

    // Using a second path moves it to the front of recent.
    const xc2 = path.join(tmp, "Other.xcworkspace");
    await fs.mkdir(xc2);
    const second = await workspaceUse({ path: xc2 }, ctx);
    expect(second.recent).toEqual([xc2, xc]);

    // Reusing the first path bumps it back to position 0 (no duplicate).
    const third = await workspaceUse({ path: xc }, ctx);
    expect(third.recent).toEqual([xc, xc2]);

    expect((await workspaceRecent({}, ctx)).recent).toEqual([xc, xc2]);
  });

  it("workspace.use rejects paths that don't exist", async () => {
    const ctx = makeContext({ workspacePath: tmp });
    await expect(workspaceUse({ path: path.join(tmp, "does-not-exist.xcworkspace") }, ctx)).rejects.toThrow(
      /No file or directory/,
    );
  });
});
