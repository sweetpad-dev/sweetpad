import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { noopLogger } from "../../core/logger/types";
import { BuildRegistry } from "../registry";
import { createBuildsListMethod } from "./builds-list";

function newRegistry(): { registry: BuildRegistry; dir: string } {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-bl-"));
  return { registry: new BuildRegistry({ buildsDir: dir, logger: noopLogger }), dir };
}

function seed(registry: BuildRegistry, ...specs: Array<{ status: "succeeded" | "failed" | "running" }>) {
  for (const spec of specs) {
    const b = registry.start({ scheme: "S", destination: "D", configuration: "Debug", originator: "cli" });
    if (spec.status !== "running") {
      registry.finish(b.buildId, { status: spec.status, exitCode: spec.status === "succeeded" ? 0 : 65, diagnostics: [] });
    }
  }
}

describe("builds.list method", () => {
  let cleanup: () => void;

  beforeEach(() => {
    cleanup = () => {};
  });

  afterEach(() => {
    cleanup();
  });

  it("returns every build, most recent first", async () => {
    const { registry, dir } = newRegistry();
    cleanup = () => fs.rmSync(dir, { recursive: true, force: true });
    seed(registry, { status: "succeeded" }, { status: "failed" }, { status: "succeeded" });

    const method = createBuildsListMethod({ registry });
    const result = await method({});

    expect(result.builds.map((b) => b.buildId)).toEqual(["b3", "b2", "b1"]);
  });

  it("filters by status", async () => {
    const { registry, dir } = newRegistry();
    cleanup = () => fs.rmSync(dir, { recursive: true, force: true });
    seed(registry, { status: "succeeded" }, { status: "failed" }, { status: "running" });

    const method = createBuildsListMethod({ registry });
    const result = await method({ status: "failed" });

    expect(result.builds).toHaveLength(1);
    expect(result.builds[0].status).toBe("failed");
  });

  it("caps results with limit", async () => {
    const { registry, dir } = newRegistry();
    cleanup = () => fs.rmSync(dir, { recursive: true, force: true });
    seed(registry, { status: "succeeded" }, { status: "succeeded" }, { status: "succeeded" });

    const method = createBuildsListMethod({ registry });
    const result = await method({ limit: 2 });

    expect(result.builds).toHaveLength(2);
    expect(result.builds.map((b) => b.buildId)).toEqual(["b3", "b2"]);
  });

  it("returns an empty list when the filter matches nothing", async () => {
    const { registry, dir } = newRegistry();
    cleanup = () => fs.rmSync(dir, { recursive: true, force: true });
    seed(registry, { status: "succeeded" });

    const method = createBuildsListMethod({ registry });
    const result = await method({ status: "failed" });
    expect(result.builds).toEqual([]);
  });

  it("rejects invalid status with INVALID_ARGUMENT", async () => {
    const { registry, dir } = newRegistry();
    cleanup = () => fs.rmSync(dir, { recursive: true, force: true });
    const method = createBuildsListMethod({ registry });
    await expect(method({ status: "bogus" })).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });

  it("rejects negative limit with INVALID_ARGUMENT", async () => {
    const { registry, dir } = newRegistry();
    cleanup = () => fs.rmSync(dir, { recursive: true, force: true });
    const method = createBuildsListMethod({ registry });
    await expect(method({ limit: -1 })).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });

  it("rejects non-integer limit with INVALID_ARGUMENT", async () => {
    const { registry, dir } = newRegistry();
    cleanup = () => fs.rmSync(dir, { recursive: true, force: true });
    const method = createBuildsListMethod({ registry });
    await expect(method({ limit: 1.5 })).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });
});
