import { type Dirent, promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { findFiles, findFilesRecursive } from "./files";

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
});
