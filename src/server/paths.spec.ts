import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import {
  findProjectRoot,
  generateServerName,
  getActiveFile,
  getBuildDir,
  getBuildsDir,
  getConnectionFile,
  getRunDir,
  getSocketPath,
  getStateRoot,
  SWEETPAD_DIR_NAME,
} from "./paths";

describe("server/paths", () => {
  describe("project-local layout", () => {
    const ws = "/some/Project";
    const root = path.join(ws, ".sweetpad");

    it("roots state at <workspace>/.sweetpad", () => {
      expect(getStateRoot(ws)).toBe(root);
    });

    it("nests run/, active.json and builds under the state root", () => {
      expect(getRunDir(ws)).toBe(path.join(root, "run"));
      expect(getActiveFile(ws)).toBe(path.join(root, "active.json"));
      expect(getBuildsDir(ws)).toBe(path.join(root, "builds"));
      expect(getBuildDir(ws, "b3")).toBe(path.join(root, "builds", "b3"));
    });

    it("places a server's connection file at run/<name>.json", () => {
      expect(getConnectionFile(ws, "abc123")).toBe(path.join(root, "run", "abc123.json"));
    });
  });

  describe("getSocketPath", () => {
    it("puts the socket in tmpdir (short path), keyed by name", () => {
      expect(getSocketPath("abc123")).toBe(path.join(os.tmpdir(), "sweetpad-abc123.sock"));
    });
  });

  describe("generateServerName", () => {
    it("produces 6-char lowercase hex names", () => {
      for (let i = 0; i < 50; i += 1) {
        expect(generateServerName()).toMatch(/^[0-9a-f]{6}$/);
      }
    });

    it("returns different names across calls (very low collision probability)", () => {
      const set = new Set<string>();
      for (let i = 0; i < 100; i += 1) set.add(generateServerName());
      expect(set.size).toBeGreaterThan(90);
    });
  });

  describe("findProjectRoot", () => {
    let tmp: string;
    beforeEach(async () => {
      tmp = await fs.mkdtemp(path.join(os.tmpdir(), "sw-paths-"));
    });
    afterEach(async () => {
      await fs.rm(tmp, { recursive: true, force: true });
    });

    it("walks up to the nearest ancestor containing .sweetpad", async () => {
      const proj = path.join(tmp, "proj");
      const nested = path.join(proj, "a", "b");
      await fs.mkdir(path.join(proj, SWEETPAD_DIR_NAME), { recursive: true });
      await fs.mkdir(nested, { recursive: true });
      expect(await findProjectRoot(nested)).toBe(proj);
    });

    it("returns undefined when no .sweetpad exists up the tree", async () => {
      const nested = path.join(tmp, "x", "y");
      await fs.mkdir(nested, { recursive: true });
      expect(await findProjectRoot(nested)).toBeUndefined();
    });
  });
});
