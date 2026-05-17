import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { noopLogger } from "../../core/logger/types";
import { BuildRegistry } from "../registry";
import { createLogsGetMethod } from "./logs-get";

describe("logs.get method", () => {
  let buildsDir: string;

  beforeEach(() => {
    buildsDir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-lg-"));
  });

  afterEach(() => {
    fs.rmSync(buildsDir, { recursive: true, force: true });
  });

  function setup() {
    const registry = new BuildRegistry({ buildsDir, logger: noopLogger });
    const method = createLogsGetMethod({ registry });
    return { registry, method };
  }

  function seedBuild(registry: BuildRegistry, logContent: string | undefined): string {
    const b = registry.start({ scheme: "S", destination: "D", configuration: "Debug", originator: "cli" });
    if (logContent !== undefined) {
      const logPath = registry.getLogPath(b.buildId);
      fs.mkdirSync(path.dirname(logPath), { recursive: true });
      fs.writeFileSync(logPath, logContent);
    }
    return b.buildId;
  }

  it("returns the full log content for a known buildId", async () => {
    const { registry, method } = setup();
    const id = seedBuild(registry, "line1\nline2\nline3\n");

    const result = await method({ buildId: id });
    expect(result.content).toBe("line1\nline2\nline3");
    expect(result.lineCount).toBe(3);
    expect(result.truncated).toBe(false);
  });

  it("returns empty payload when no log file was ever written", async () => {
    const { registry, method } = setup();
    const id = seedBuild(registry, undefined);

    const result = await method({ buildId: id });
    expect(result.content).toBe("");
    expect(result.lineCount).toBe(0);
    expect(result.truncated).toBe(false);
  });

  it("returns only the last N lines when --tail is set", async () => {
    const { registry, method } = setup();
    const id = seedBuild(registry, "a\nb\nc\nd\ne\n");

    const result = await method({ buildId: id, tail: 2 });
    expect(result.content).toBe("d\ne");
    expect(result.lineCount).toBe(5);
    expect(result.truncated).toBe(true);
  });

  it("does not flag truncated when tail >= total line count", async () => {
    const { registry, method } = setup();
    const id = seedBuild(registry, "a\nb\n");

    const result = await method({ buildId: id, tail: 100 });
    expect(result.content).toBe("a\nb");
    expect(result.truncated).toBe(false);
  });

  it("handles a log file without a trailing newline", async () => {
    const { registry, method } = setup();
    const id = seedBuild(registry, "a\nb\nc");
    const result = await method({ buildId: id });
    expect(result.lineCount).toBe(3);
    expect(result.content).toBe("a\nb\nc");
  });

  it("throws BUILD_NOT_FOUND for an unknown buildId", async () => {
    const { method } = setup();
    await expect(method({ buildId: "bX" })).rejects.toMatchObject({ code: "BUILD_NOT_FOUND" });
  });

  it("rejects missing buildId with INVALID_ARGUMENT", async () => {
    const { method } = setup();
    await expect(method({})).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });

  it("rejects negative tail with INVALID_ARGUMENT", async () => {
    const { registry, method } = setup();
    seedBuild(registry, "line\n");
    await expect(method({ buildId: "b1", tail: -1 })).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });
});
