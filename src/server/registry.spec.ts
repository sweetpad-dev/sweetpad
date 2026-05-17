import { describe, expect, it } from "vitest";

import { BuildRegistry } from "./registry";

describe("BuildRegistry", () => {
  it("allocates sequential b1, b2, ... ids", () => {
    const reg = new BuildRegistry();
    const a = reg.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });
    const b = reg.start({ scheme: "B", destination: "D", configuration: "Debug", originator: "cli" });
    expect(a.buildId).toBe("b1");
    expect(b.buildId).toBe("b2");
  });

  it("starts builds in 'running' status", () => {
    const reg = new BuildRegistry();
    const build = reg.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });
    expect(build.status).toBe("running");
    expect(build.finishedAt).toBeNull();
    expect(build.exitCode).toBeNull();
  });

  it("finishes a build with diagnostics and counts errors/warnings", () => {
    const reg = new BuildRegistry();
    const build = reg.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });
    const finished = reg.finish(build.buildId, {
      status: "failed",
      exitCode: 65,
      diagnostics: [
        { file: "a.swift", line: 1, column: 1, severity: "error", message: "x", source: "xcodebuild" },
        { file: "b.swift", line: 2, column: 1, severity: "warning", message: "y", source: "xcodebuild" },
        { file: "c.swift", line: 3, column: 1, severity: "error", message: "z", source: "xcodebuild" },
      ],
    });
    expect(finished.status).toBe("failed");
    expect(finished.exitCode).toBe(65);
    expect(finished.errorCount).toBe(2);
    expect(finished.warningCount).toBe(1);
    expect(finished.diagnostics).toHaveLength(3);
    expect(finished.finishedAt).not.toBeNull();
    expect(finished.durationMs).not.toBeNull();
  });

  it("lists only running builds via running()", () => {
    const reg = new BuildRegistry();
    const a = reg.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });
    reg.start({ scheme: "B", destination: "D", configuration: "Debug", originator: "cli" });
    reg.finish(a.buildId, { status: "succeeded", exitCode: 0, diagnostics: [] });
    const running = reg.running();
    expect(running).toHaveLength(1);
    expect(running[0].scheme).toBe("B");
  });
});
