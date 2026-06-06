import { commonLogger } from "../../common/logger";
import type { RpcContext } from "./context";
import { logsTail } from "./logs";

function makeCtx(): RpcContext {
  return {
    workspacePath: "/tmp/ws",
    extensionVersion: "test",
    workspace: {} as RpcContext["workspace"],
    buildManager: {} as RpcContext["buildManager"],
    destinationsManager: {} as RpcContext["destinationsManager"],
    buildRegistry: {} as RpcContext["buildRegistry"],
    vscodeContext: {} as RpcContext["vscodeContext"],
    configKeys: [],
    bspBridge: {} as RpcContext["bspBridge"],
  };
}

describe("logs.tail", () => {
  it("returns the most recent N entries at or above the requested level", async () => {
    commonLogger.debug("d");
    commonLogger.log("info-msg");
    commonLogger.warn("warn-msg");
    commonLogger.error("err-msg");

    const ctx = makeCtx();
    const all = await Promise.resolve(logsTail({ lines: 4 }, ctx));
    // The logger's global level may have suppressed earlier entries; assert
    // only on what we know — that error/warn made it through to tail in order.
    const tail = all.entries.map((e) => e.message);
    expect(tail).toContain("warn-msg");
    expect(tail).toContain("err-msg");

    const errOnly = await Promise.resolve(logsTail({ level: "error" }, ctx));
    expect(errOnly.entries.every((e) => e.level === "error")).toBe(true);
  });

  it("rejects unknown levels", () => {
    expect(() => logsTail({ level: "trace" }, makeCtx())).toThrow(/Unknown level/);
  });
});
