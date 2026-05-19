import { promises as fs } from "node:fs";
import * as path from "node:path";

import { getSocketsDir } from "../server/paths";
import { resolveServerName } from "./resolve";

describe("cli/resolve resolveServerName", () => {
  let tmpRoot: string;
  let originalXdg: string | undefined;

  beforeEach(async () => {
    tmpRoot = await fs.mkdtemp("/tmp/sw-resolve-");
    originalXdg = process.env.XDG_STATE_HOME;
    process.env.XDG_STATE_HOME = tmpRoot;
    await fs.mkdir(getSocketsDir(), { recursive: true });
  });

  afterEach(async () => {
    if (originalXdg === undefined) {
      delete process.env.XDG_STATE_HOME;
    } else {
      process.env.XDG_STATE_HOME = originalXdg;
    }
    await fs.rm(tmpRoot, { recursive: true, force: true });
  });

  async function seed(...names: string[]): Promise<void> {
    for (const n of names) {
      await fs.writeFile(path.join(getSocketsDir(), `${n}.json`), "{}");
    }
  }

  it("returns kind: 'none' when sockets dir is empty", async () => {
    const r = await resolveServerName("af");
    expect(r.kind).toBe("none");
  });

  it("returns ok on exact match", async () => {
    await seed("af95d0");
    const r = await resolveServerName("af95d0");
    expect(r).toEqual({ kind: "ok", name: "af95d0" });
  });

  it("returns ok on unique prefix", async () => {
    await seed("af95d0", "b927a6");
    const r = await resolveServerName("af");
    expect(r).toEqual({ kind: "ok", name: "af95d0" });
  });

  it("returns ambiguous with matches when prefix matches multiple", async () => {
    await seed("af95d0", "af11bb", "b927a6");
    const r = await resolveServerName("af");
    if (r.kind !== "ambiguous") {
      throw new Error(`expected ambiguous, got ${r.kind}`);
    }
    expect(r.matches.toSorted()).toEqual(["af11bb", "af95d0"]);
  });

  it("returns none when prefix matches nothing", async () => {
    await seed("af95d0", "b927a6");
    const r = await resolveServerName("zz");
    expect(r.kind).toBe("none");
  });

  it("prefers exact match over prefix-of (e.g., 'a' alone vs 'a' + 'ab')", async () => {
    await seed("a", "ab");
    const r = await resolveServerName("a");
    expect(r).toEqual({ kind: "ok", name: "a" });
  });
});
