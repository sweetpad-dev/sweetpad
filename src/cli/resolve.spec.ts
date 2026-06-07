import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { getRunDir } from "../server/paths";
import { resolveServerName } from "./resolve";

describe("cli/resolve resolveServerName", () => {
  let project: string;

  beforeEach(async () => {
    project = await fs.mkdtemp(path.join(os.tmpdir(), "sw-resolve-"));
    await fs.mkdir(getRunDir(project), { recursive: true });
  });

  afterEach(async () => {
    await fs.rm(project, { recursive: true, force: true });
  });

  // Seeds `kind: "extension"` connection files; `bsp:<name>` seeds a BSP entry
  // that must be ignored by resolution.
  async function seed(...names: string[]): Promise<void> {
    for (const spec of names) {
      const isBsp = spec.startsWith("bsp:");
      const name = isBsp ? spec.slice("bsp:".length) : spec;
      const meta = { name, kind: isBsp ? "bsp" : "extension", socket: `/tmp/sweetpad-${name}.sock` };
      await fs.writeFile(path.join(getRunDir(project), `${name}.json`), JSON.stringify(meta));
    }
  }

  it("returns kind: 'none' when the run dir is empty", async () => {
    expect((await resolveServerName("af", project)).kind).toBe("none");
  });

  it("returns ok on exact match", async () => {
    await seed("af95d0");
    expect(await resolveServerName("af95d0", project)).toEqual({ kind: "ok", name: "af95d0" });
  });

  it("returns ok on unique prefix", async () => {
    await seed("af95d0", "b927a6");
    expect(await resolveServerName("af", project)).toEqual({ kind: "ok", name: "af95d0" });
  });

  it("returns ambiguous with matches when prefix matches multiple", async () => {
    await seed("af95d0", "af11bb", "b927a6");
    const r = await resolveServerName("af", project);
    if (r.kind !== "ambiguous") throw new Error(`expected ambiguous, got ${r.kind}`);
    expect(r.matches.toSorted()).toEqual(["af11bb", "af95d0"]);
  });

  it("returns none when prefix matches nothing", async () => {
    await seed("af95d0", "b927a6");
    expect((await resolveServerName("zz", project)).kind).toBe("none");
  });

  it("prefers exact match over prefix-of (e.g., 'a' alone vs 'a' + 'ab')", async () => {
    await seed("a", "ab");
    expect(await resolveServerName("a", project)).toEqual({ kind: "ok", name: "a" });
  });

  it("ignores BSP connection files (not CLI targets)", async () => {
    await seed("bsp:dd11ee");
    expect((await resolveServerName("dd", project)).kind).toBe("none");
  });
});
