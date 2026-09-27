import { type Dirent, promises as fs } from "node:fs";
import * as path from "node:path";

import * as vscode from "vscode";

import { findFiles, getWorkspaceRelativePath } from "./files";
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

describe("findFiles path building", () => {
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
