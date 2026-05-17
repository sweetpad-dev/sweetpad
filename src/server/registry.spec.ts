import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { noopLogger } from "../core/logger/types";
import { BuildRegistry } from "./registry";

function makeBuildsDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-reg-"));
}

describe("BuildRegistry", () => {
  let buildsDir: string;

  beforeEach(() => {
    buildsDir = makeBuildsDir();
  });

  afterEach(() => {
    fs.rmSync(buildsDir, { recursive: true, force: true });
  });

  function newRegistry(): BuildRegistry {
    return new BuildRegistry({ buildsDir, logger: noopLogger });
  }

  it("allocates sequential b1, b2, ... ids", () => {
    const reg = newRegistry();
    const a = reg.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });
    const b = reg.start({ scheme: "B", destination: "D", configuration: "Debug", originator: "cli" });
    expect(a.buildId).toBe("b1");
    expect(b.buildId).toBe("b2");
  });

  it("starts builds in 'running' status", () => {
    const reg = newRegistry();
    const build = reg.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });
    expect(build.status).toBe("running");
    expect(build.finishedAt).toBeNull();
    expect(build.exitCode).toBeNull();
  });

  it("finishes a build with diagnostics and counts errors/warnings", () => {
    const reg = newRegistry();
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
    const reg = newRegistry();
    const a = reg.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });
    reg.start({ scheme: "B", destination: "D", configuration: "Debug", originator: "cli" });
    reg.finish(a.buildId, { status: "succeeded", exitCode: 0, diagnostics: [] });
    const running = reg.running();
    expect(running).toHaveLength(1);
    expect(running[0].scheme).toBe("B");
  });

  describe("disk persistence", () => {
    function readSnapshot(buildId: string): unknown {
      const p = path.join(buildsDir, buildId, "snapshot.json");
      return JSON.parse(fs.readFileSync(p, "utf8"));
    }

    it("writes snapshot.json on start()", () => {
      const reg = newRegistry();
      reg.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });
      const snap = readSnapshot("b1") as { buildId: string; scheme: string; status: string };
      expect(snap.buildId).toBe("b1");
      expect(snap.scheme).toBe("A");
      expect(snap.status).toBe("running");
    });

    it("overwrites snapshot.json on finish()", () => {
      const reg = newRegistry();
      const b = reg.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });
      reg.finish(b.buildId, { status: "succeeded", exitCode: 0, diagnostics: [] });
      const snap = readSnapshot("b1") as { status: string; exitCode: number };
      expect(snap.status).toBe("succeeded");
      expect(snap.exitCode).toBe(0);
    });
  });

  describe("recover()", () => {
    it("loads existing finished builds from disk", () => {
      const reg1 = newRegistry();
      const a = reg1.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });
      reg1.finish(a.buildId, { status: "succeeded", exitCode: 0, diagnostics: [] });

      const reg2 = newRegistry();
      reg2.recover();

      const recovered = reg2.get("b1");
      expect(recovered).toMatchObject({ buildId: "b1", scheme: "A", status: "succeeded" });
    });

    it("bumps nextId past the highest recovered id", () => {
      const reg1 = newRegistry();
      reg1.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });
      reg1.start({ scheme: "B", destination: "D", configuration: "Debug", originator: "cli" });

      const reg2 = newRegistry();
      reg2.recover();
      const next = reg2.start({ scheme: "C", destination: "D", configuration: "Debug", originator: "cli" });
      expect(next.buildId).toBe("b3");
    });

    it("marks recovered 'running' builds as 'interrupted'", () => {
      const reg1 = newRegistry();
      reg1.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });
      // Crash simulation: reg1 never called finish() before going away.

      const reg2 = newRegistry();
      reg2.recover();

      const recovered = reg2.get("b1");
      expect(recovered?.status).toBe("interrupted");
      expect(recovered?.finishedAt).not.toBeNull();
      expect(reg2.running()).toHaveLength(0);
    });

    it("persists the running → interrupted fix-up so a second recover sees it", () => {
      const reg1 = newRegistry();
      reg1.start({ scheme: "A", destination: "D", configuration: "Debug", originator: "cli" });

      newRegistry().recover();

      const snap = JSON.parse(fs.readFileSync(path.join(buildsDir, "b1", "snapshot.json"), "utf8")) as {
        status: string;
      };
      expect(snap.status).toBe("interrupted");
    });

    it("is a no-op when buildsDir doesn't exist", () => {
      fs.rmSync(buildsDir, { recursive: true, force: true });
      const reg = newRegistry();
      expect(() => reg.recover()).not.toThrow();
      expect(reg.running()).toHaveLength(0);
    });

    it("skips corrupt snapshot.json files without throwing", () => {
      fs.mkdirSync(path.join(buildsDir, "b1"), { recursive: true });
      fs.writeFileSync(path.join(buildsDir, "b1", "snapshot.json"), "{not valid json");

      const reg = newRegistry();
      expect(() => reg.recover()).not.toThrow();
      expect(reg.get("b1")).toBeUndefined();
    });
  });
});
