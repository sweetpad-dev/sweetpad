import * as os from "node:os";
import * as path from "node:path";

import {
  generateServerName,
  getActiveFile,
  getBuildsDir,
  getMetadataPath,
  getSocketPath,
  getSocketsDir,
  getStateRoot,
  getWorkspaceDir,
  workspacePathHash,
} from "./paths";

describe("server/paths", () => {
  const originalXdg = process.env.XDG_STATE_HOME;
  beforeEach(() => {
    delete process.env.XDG_STATE_HOME;
  });
  afterEach(() => {
    if (originalXdg === undefined) {
      delete process.env.XDG_STATE_HOME;
    } else {
      process.env.XDG_STATE_HOME = originalXdg;
    }
  });

  describe("getStateRoot", () => {
    it("defaults to ~/.local/state/sweetpad when XDG_STATE_HOME is unset", () => {
      expect(getStateRoot()).toBe(path.join(os.homedir(), ".local", "state", "sweetpad"));
    });

    it("respects $XDG_STATE_HOME when set to an absolute path", () => {
      process.env.XDG_STATE_HOME = "/custom/state";
      expect(getStateRoot()).toBe("/custom/state/sweetpad");
    });

    it("ignores $XDG_STATE_HOME when it's not absolute", () => {
      process.env.XDG_STATE_HOME = "relative/path";
      expect(getStateRoot()).toBe(path.join(os.homedir(), ".local", "state", "sweetpad"));
    });
  });

  describe("nested paths", () => {
    it("nests sockets, active.json, and workspaces under the state root", () => {
      const root = getStateRoot();
      expect(getSocketsDir()).toBe(path.join(root, "sockets"));
      expect(getActiveFile()).toBe(path.join(root, "active.json"));
    });

    it("composes socket and metadata paths from a name", () => {
      expect(getSocketPath("abc123")).toBe(path.join(getSocketsDir(), "abc123.sock"));
      expect(getMetadataPath("abc123")).toBe(path.join(getSocketsDir(), "abc123.json"));
    });

    it("nests workspace artifacts under workspaces/<sha1>/builds/<id>", () => {
      const ws = "/some/Project";
      const hash = workspacePathHash(ws);
      expect(getWorkspaceDir(ws).endsWith(path.join("workspaces", hash))).toBe(true);
      expect(getBuildsDir(ws).endsWith(path.join("workspaces", hash, "builds"))).toBe(true);
    });
  });

  describe("generateServerName", () => {
    it("produces 6-char lowercase hex names", () => {
      for (let i = 0; i < 50; i += 1) {
        const name = generateServerName();
        expect(name).toMatch(/^[0-9a-f]{6}$/);
      }
    });

    it("returns different names across calls (very low collision probability)", () => {
      const set = new Set<string>();
      for (let i = 0; i < 100; i += 1) set.add(generateServerName());
      expect(set.size).toBeGreaterThan(90);
    });
  });

  describe("workspacePathHash", () => {
    it("is deterministic for the same input", () => {
      expect(workspacePathHash("/a/b/c")).toBe(workspacePathHash("/a/b/c"));
    });

    it("differs for different inputs", () => {
      expect(workspacePathHash("/a")).not.toBe(workspacePathHash("/b"));
    });

    it("is a 40-char hex string (sha1)", () => {
      expect(workspacePathHash("/x")).toMatch(/^[0-9a-f]{40}$/);
    });
  });
});
