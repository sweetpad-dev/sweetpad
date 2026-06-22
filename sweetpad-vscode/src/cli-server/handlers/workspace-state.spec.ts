import type { WorkspaceStateService } from "../../common/workspace-state";
import type { RpcContext } from "./context";
import { workspaceStateDelete, workspaceStateGet, workspaceStateKeys, workspaceStateSet } from "./workspace-state";

function makeContext(): { ctx: RpcContext; state: Map<string, unknown> } {
  const state = new Map<string, unknown>();
  const ws: Pick<WorkspaceStateService, "rawGet" | "rawUpdate" | "rawKeys"> = {
    rawGet: (key: string) => state.get(key),
    rawUpdate: async (key: string, value: unknown) => {
      if (value === undefined) state.delete(key);
      else state.set(key, value);
    },
    rawKeys: () => [...state.keys()],
  };
  const ctx: RpcContext = {
    workspacePath: "/tmp/ws",
    extensionVersion: "test",
    workspaceState: ws as WorkspaceStateService,
    buildManager: {} as RpcContext["buildManager"],
    destinationsManager: {} as RpcContext["destinationsManager"],
    buildRegistry: {} as RpcContext["buildRegistry"],
    vscodeContext: {} as RpcContext["vscodeContext"],
    configKeys: [],
  };
  return { ctx, state };
}

describe("workspaceState handlers", () => {
  it("get returns null when the key is unset (no JSON undefined)", () => {
    const { ctx } = makeContext();
    expect(workspaceStateGet({ key: "missing" }, ctx)).toEqual({ key: "missing", value: null });
  });

  it("set persists then get reads back", async () => {
    const { ctx } = makeContext();
    await workspaceStateSet({ key: "build.xcodeScheme", value: "MyApp" }, ctx);
    expect(workspaceStateGet({ key: "build.xcodeScheme" }, ctx)).toEqual({
      key: "build.xcodeScheme",
      value: "MyApp",
    });
  });

  it("set with value: null clears the key", async () => {
    const { ctx, state } = makeContext();
    state.set("foo", "bar");
    await workspaceStateSet({ key: "foo", value: null }, ctx);
    expect(state.has("foo")).toBe(false);
  });

  it("keys returns every stored key, sorted", () => {
    const { ctx, state } = makeContext();
    state.set("zebra", 1);
    state.set("alpha", 2);
    state.set("middle", 3);
    expect(workspaceStateKeys({}, ctx)).toEqual({ keys: ["alpha", "middle", "zebra"] });
  });

  it("delete returns deleted: true only when the key existed", async () => {
    const { ctx, state } = makeContext();
    state.set("present", "x");
    expect(await workspaceStateDelete({ key: "present" }, ctx)).toEqual({ key: "present", deleted: true });
    expect(await workspaceStateDelete({ key: "absent" }, ctx)).toEqual({ key: "absent", deleted: false });
  });

  it("rejects empty / non-string keys", async () => {
    const { ctx } = makeContext();
    expect(() => workspaceStateGet({ key: "" }, ctx)).toThrow(/requires/);
    await expect(workspaceStateSet({ key: undefined, value: 1 }, ctx)).rejects.toThrow(/requires/);
  });
});
