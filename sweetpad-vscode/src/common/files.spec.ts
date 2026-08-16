import { type Dirent, promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import * as vscode from "vscode";

import { findFiles, findFilesRecursive, getWorkspaceRelativePath } from "./files";
import { WorkspaceContextService } from "./workspace-context";

// `../build/utils` imports the native `@sweetpad/native` addon at module level; stub it so this
// spec runs without the compiled addon (none of the tested paths touch it).
vi.mock("@sweetpad/native", () => ({}));

/**
 * Build a Dirent-like object that intentionally omits `path`/`parentPath`.
 *
 * This mirrors older Node runtimes (the property was only added in Node
 * 18.17/20.1, and is since deprecated in favor of `parentPath`) where
 * `Dirent.path` is `undefined`. Relying on it made path.join throw
 * "The path argument must be of type string. Received undefined" — see #255.
 */
function direntWithoutPath(name: string, isDir: boolean): Dirent {
  return {
    name,
    isDirectory: () => isDir,
    isFile: () => !isDir,
    isSymbolicLink: () => false,
    isBlockDevice: () => false,
    isCharacterDevice: () => false,
    isFIFO: () => false,
    isSocket: () => false,
  } as unknown as Dirent;
}

describe("findFiles / findFilesRecursive path building", () => {
  it("does not rely on Dirent.path (undefined on older Node) — #255", async () => {
    // Simulate a runtime where Dirent.path is undefined: readdir returns
    // entries without the `path` property.
    const spy = vi
      .spyOn(fs, "readdir")
      .mockResolvedValue([
        direntWithoutPath("App.xcworkspace", true),
        direntWithoutPath("README.md", false),
      ] as unknown as never);

    try {
      const result = await findFiles({
        directory: "/Users/test/project",
        matcher: (file) => file.name.endsWith(".xcworkspace"),
      });
      expect(result).toEqual([path.join("/Users/test/project", "App.xcworkspace")]);
    } finally {
      spy.mockRestore();
    }
  });

  it("recurses into subdirectories using the read directory, not Dirent.path", async () => {
    const root = "/Users/test/project";
    const nested = path.join(root, "App.xcodeproj");

    const spy = vi.spyOn(fs, "readdir").mockImplementation((async (dir: string) => {
      if (dir === root) {
        return [direntWithoutPath("App.xcodeproj", true)];
      }
      if (dir === nested) {
        return [direntWithoutPath("project.xcworkspace", true)];
      }
      return [];
    }) as unknown as typeof fs.readdir);

    try {
      const result = await findFilesRecursive({
        directory: root,
        depth: 4,
        matcher: (file) => file.name.endsWith(".xcworkspace"),
      });
      expect(result).toEqual([path.join(nested, "project.xcworkspace")]);
    } finally {
      spy.mockRestore();
    }
  });

  it("works end-to-end against the real filesystem", async () => {
    const tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-files-"));
    try {
      const projDir = path.join(tmpDir, "App.xcodeproj");
      await fs.mkdir(projDir);
      await fs.writeFile(path.join(tmpDir, "App.xcworkspace"), "");
      await fs.writeFile(path.join(projDir, "project.xcworkspace"), "");

      const result = await findFilesRecursive({
        directory: tmpDir,
        depth: 4,
        matcher: (file) => file.name.endsWith(".xcworkspace"),
      });

      expect(result.toSorted()).toEqual(
        [path.join(tmpDir, "App.xcworkspace"), path.join(projDir, "project.xcworkspace")].toSorted(),
      );
    } finally {
      await fs.rm(tmpDir, { recursive: true, force: true });
    }
  });

  // A multi-root scan runs this once per workspace folder and merges the results, so a folder that
  // cannot be read has to yield nothing rather than fail the whole scan.
  it("returns nothing for a directory that does not exist", async () => {
    const result = await findFilesRecursive({
      directory: path.join(os.tmpdir(), "sweetpad-does-not-exist-a8f3c1"),
      depth: 4,
      matcher: (file) => file.name.endsWith(".xcworkspace"),
    });
    expect(result).toEqual([]);
  });

  // A readdir failure deeper in the tree takes the same path: the subtree contributes nothing and
  // its siblings still come back.
  it("keeps siblings when a subdirectory cannot be read", async () => {
    const root = "/Users/test/project";
    const spy = vi.spyOn(fs, "readdir").mockImplementation((async (dir: string) => {
      if (dir === root) {
        return [direntWithoutPath("App.xcworkspace", true), direntWithoutPath("denied", true)];
      }
      throw Object.assign(new Error("EACCES: permission denied"), { code: "EACCES" });
    }) as unknown as typeof fs.readdir);

    try {
      const result = await findFilesRecursive({
        directory: root,
        depth: 4,
        matcher: (file) => file.name.endsWith(".xcworkspace"),
      });
      expect(result).toEqual([path.join(root, "App.xcworkspace")]);
    } finally {
      spy.mockRestore();
    }
  });
});

describe("getWorkspaceRelativePath", () => {
  let workspaceContext: WorkspaceContextService;

  function setFolders(paths: string[]) {
    (vscode.workspace as { workspaceFolders?: unknown }).workspaceFolders = paths.map((p) => ({
      uri: { fsPath: p },
    }));
  }

  beforeEach(() => {
    workspaceContext = new WorkspaceContextService();
  });

  it("keeps the folder prefix for a project outside the first workspace folder", () => {
    setFolders(["/root-1", "/root-2"]);
    // Selecting a project moves the active folder to the one holding it.
    workspaceContext.setActiveFolder("/root-2/App.xcworkspace");

    // Anchoring to the active folder would yield the bare "App.xcworkspace", which /root-1 also
    // satisfies; the prefix is what makes the stored value name exactly one file.
    expect(getWorkspaceRelativePath("/root-2/App.xcworkspace")).toBe("../root-2/App.xcworkspace");
  });

  it("stays a plain relative path inside the first workspace folder", () => {
    setFolders(["/root-1", "/root-2"]);
    expect(getWorkspaceRelativePath("/root-1/App/App.xcworkspace")).toBe("App/App.xcworkspace");
  });
});
