import { buildDispatch } from "./handlers";
import type { RpcContext } from "./handlers/context";
import { METHOD_CATALOG, methodHint } from "./method-catalog";

// buildDispatch only closes over the context; nothing reads it while the table
// is assembled, so a bare object is enough to enumerate the method names.
const dispatch = buildDispatch({} as RpcContext);

describe("method catalog", () => {
  it("documents exactly the methods the server dispatches", () => {
    expect(Object.keys(METHOD_CATALOG).toSorted()).toEqual(Object.keys(dispatch).toSorted());
  });

  it("renders a hint the generic client can parse, with and without flags", () => {
    expect(methodHint("buildConfig.list")).toBe("sweetpad vscode buildConfig.list");
    expect(methodHint("scheme.set", "--name <name>")).toBe("sweetpad vscode scheme.set --name <name>");
  });
});
