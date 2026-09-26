import * as os from "node:os";
import * as path from "node:path";

import {
  generateServerName,
  getBuildDir,
  getBuildsDir,
  getProjectsIndexFile,
  getProjectStateDir,
  getSocketPath,
  getSweetpadStateHome,
  socketPathFor,
  workspaceHash,
} from "./paths";

// `os.tmpdir()` reads `TMPDIR` on every call.
function withTmpdir(value: string, fn: () => void): void {
  const prev = process.env.TMPDIR;
  process.env.TMPDIR = value;
  try {
    fn();
  } finally {
    if (prev === undefined) delete process.env.TMPDIR;
    else process.env.TMPDIR = prev;
  }
}

function withStateHome(value: string | undefined, fn: () => void): void {
  const prev = process.env.XDG_STATE_HOME;
  if (value === undefined) delete process.env.XDG_STATE_HOME;
  else process.env.XDG_STATE_HOME = value;
  try {
    fn();
  } finally {
    if (prev === undefined) delete process.env.XDG_STATE_HOME;
    else process.env.XDG_STATE_HOME = prev;
  }
}

describe("server/paths", () => {
  describe("path layout", () => {
    const ws = "/some/Project";

    it("roots machine-managed state under the XDG state home, never the project", () => {
      withStateHome("/xdg/state", () => {
        const home = path.join("/xdg/state", "sweetpad");
        expect(getSweetpadStateHome()).toBe(home);
        expect(getProjectsIndexFile()).toBe(path.join(home, "projects.json"));
        expect(getProjectStateDir(ws)).toBe(path.join(home, "projects", workspaceHash(ws)));
      });
    });

    it("falls back to ~/.local/state when XDG_STATE_HOME is unset", () => {
      withStateHome(undefined, () => {
        expect(getSweetpadStateHome()).toBe(path.join(os.homedir(), ".local", "state", "sweetpad"));
      });
    });

    it("derives a stable 12-char hex workspace hash", () => {
      expect(workspaceHash(ws)).toMatch(/^[0-9a-f]{12}$/);
      expect(workspaceHash(ws)).toBe(workspaceHash(ws));
    });

    it("puts build history in the per-project state dir", () => {
      withStateHome("/xdg/state", () => {
        const projectDir = getProjectStateDir(ws);
        expect(getBuildsDir(ws)).toBe(path.join(projectDir, "builds"));
        expect(getBuildDir(ws, "b3")).toBe(path.join(projectDir, "builds", "b3"));
      });
    });
  });

  describe("getSocketPath", () => {
    it("puts the socket in tmpdir (short path), keyed by name", () => {
      withTmpdir("/var/folders/ab/cdefgh/T", () => {
        expect(getSocketPath("abc123")).toBe(path.join(os.tmpdir(), "sweetpad-abc123.sock"));
      });
    });
  });

  describe("socketPathFor", () => {
    it("keeps a path that fits sun_path in tmpdir", () => {
      const tmpdir = `/${"t".repeat(103 - "/x.sock".length - 1)}`;
      withTmpdir(tmpdir, () => {
        expect(socketPathFor("x.sock")).toBe(`${tmpdir}/x.sock`);
        expect(Buffer.byteLength(socketPathFor("x.sock"))).toBe(103);
      });
    });

    it("falls back to /tmp when a long TMPDIR would pass sun_path's 104 bytes", () => {
      const tmpdir = `/${"t".repeat(103 - "/x.sock".length)}`;
      withTmpdir(tmpdir, () => {
        expect(socketPathFor("x.sock")).toBe("/tmp/x.sock");
        expect(getSocketPath("abc123")).toBe("/tmp/sweetpad-abc123.sock");
      });
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
});
