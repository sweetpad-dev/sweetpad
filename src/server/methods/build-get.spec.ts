import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { noopLogger } from "../../core/logger/types";
import { BuildRegistry } from "../registry";
import { createBuildGetMethod } from "./build-get";

describe("build.get method", () => {
  let buildsDir: string;

  beforeEach(() => {
    buildsDir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-bg-"));
  });

  afterEach(() => {
    fs.rmSync(buildsDir, { recursive: true, force: true });
  });

  function setup() {
    const registry = new BuildRegistry({ buildsDir, logger: noopLogger });
    const method = createBuildGetMethod({ registry });
    return { registry, method };
  }

  it("returns the build snapshot for a known buildId", async () => {
    const { registry, method } = setup();
    const b = registry.start({ scheme: "S", destination: "D", configuration: "Debug", originator: "cli" });
    registry.finish(b.buildId, { status: "succeeded", exitCode: 0, diagnostics: [] });

    const result = await method({ buildId: "b1" });
    expect(result.buildId).toBe("b1");
    expect(result.status).toBe("succeeded");
  });

  it("throws BUILD_NOT_FOUND for an unknown buildId", async () => {
    const { method } = setup();
    await expect(method({ buildId: "b999" })).rejects.toMatchObject({ code: "BUILD_NOT_FOUND" });
  });

  it("rejects missing buildId with INVALID_ARGUMENT", async () => {
    const { method } = setup();
    await expect(method({})).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });

  it("rejects empty buildId with INVALID_ARGUMENT", async () => {
    const { method } = setup();
    await expect(method({ buildId: "" })).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });

  it("rejects non-string buildId with INVALID_ARGUMENT", async () => {
    const { method } = setup();
    await expect(method({ buildId: 42 })).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });
});
