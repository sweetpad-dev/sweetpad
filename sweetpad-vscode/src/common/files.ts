import { randomBytes } from "node:crypto";
import { promises as fs, type Dirent, type Stats } from "node:fs";
import * as path from "node:path";

import type * as vscode from "vscode";

import { getWorkspaceFolderPaths, prepareStoragePath } from "../build/utils";
import { ExtensionError } from "./errors";

/**
 * Find files or directories in a given directory
 */
export async function findFiles(options: { directory: string; matcher: (file: Dirent) => boolean }): Promise<string[]> {
  const files = await fs.readdir(options.directory, { withFileTypes: true });
  const matchedFiles: string[] = [];

  for (const file of files) {
    // Build the path from the directory we read rather than `file.path`
    // (Dirent.path): that property is undefined on older Node runtimes (added
    // in Node 18.17/20.1, since deprecated in favor of `parentPath`), which
    // makes path.join throw "path must be of type string. Received undefined".
    const fullPath = path.join(options.directory, file.name);

    if (options.matcher(file)) {
      matchedFiles.push(fullPath);
    }
  }

  return matchedFiles;
}

/**
 * Find files or directories in a given directory recursively
 */
export async function findFilesRecursive(options: {
  directory: string;
  matcher: (file: Dirent) => boolean;
  ignore?: string[];
  depth?: number;
  maxResults?: number;
}): Promise<string[]> {
  const ignore = options.ignore ?? [];
  const depth = options.depth ?? 0;

  // A directory that can't be read contributes no results rather than failing the whole scan. This
  // covers both a workspace folder that no longer exists (a stale entry in a .code-workspace, an
  // unmounted volume) and an unreadable subdirectory deeper in the tree.
  let files: Dirent[];
  try {
    files = await fs.readdir(options.directory, { withFileTypes: true });
  } catch {
    return [];
  }
  const matchedFiles: string[] = [];

  for (const file of files) {
    // Use the directory we just read instead of `file.path` (Dirent.path): that
    // property is undefined on older Node runtimes (added in Node 18.17/20.1,
    // since deprecated in favor of `parentPath`), and joining it yields
    // "path must be of type string. Received undefined".
    const fullPath = path.join(options.directory, file.name);

    if (options.matcher(file)) {
      matchedFiles.push(fullPath);
      if (options.maxResults && matchedFiles.length >= options.maxResults) {
        return matchedFiles;
      }
    }

    if (file.isDirectory() && !ignore.includes(file.name) && depth > 0) {
      const subFiles = await findFilesRecursive({
        directory: fullPath,
        matcher: options.matcher,
        ignore: options.ignore,
        depth: depth - 1,
        maxResults: options.maxResults ? options.maxResults - matchedFiles.length : undefined,
      });
      matchedFiles.push(...subFiles);
      if (options.maxResults && matchedFiles.length >= options.maxResults) {
        return matchedFiles;
      }
    }
  }

  return matchedFiles;
}

export async function isFileExists(filePath: string): Promise<boolean> {
  try {
    await fs.access(filePath);
    return true;
  } catch (e) {
    return false;
  }
}

export async function readFile(filePath: string): Promise<Buffer> {
  return await fs.readFile(filePath);
}

export async function statFile(filePath: string): Promise<Stats> {
  return await fs.stat(filePath);
}

export async function readTextFile(filePath: string): Promise<string> {
  const rawBuffer = await readFile(filePath);
  return rawBuffer.toString();
}

export async function readJsonFile<T = unknown>(filePath: string): Promise<T> {
  const rawBuffer = await readFile(filePath);
  const rawString = rawBuffer.toString();
  return JSON.parse(rawString);
}

export function getWorkspaceRelativePath(filePath: string): string {
  // Anchored to the first workspace folder, which is the one fixed point every reader agrees on:
  // `getCurrentXcodeWorkspacePath` resolves a relative setting by joining it onto each folder in
  // turn and taking the first hit, so a project in another folder has to keep its
  // "../other-folder/" prefix to name exactly one file. Anchoring to the active folder, or to the
  // file's own folder, drops that prefix, and then two folders holding the same layout — two
  // checkouts of one repo — produce the same string and resolve back to whichever comes first.
  const anchor = getWorkspaceFolderPaths()[0];
  if (!anchor) {
    throw new ExtensionError("No workspace folder found");
  }
  return path.relative(anchor, filePath);
}

export async function tempFilePath(
  vscodeContext: vscode.ExtensionContext,
  options: {
    prefix: string;
  },
) {
  // Where extension store some intermediate files
  const storagePath = await prepareStoragePath(vscodeContext);

  // Directory for all temporary files
  const tempPath = path.join(storagePath, "_temp");
  await createDirectory(tempPath);

  // Generate random file name
  const random = randomBytes(4).toString("hex");
  const filePath = path.join(tempPath, `${options.prefix}_${random}`);
  return {
    path: filePath,
    [Symbol.asyncDispose]: async () => {
      await removeFile(filePath);
    },
  };
}

export async function createDirectory(directory: string) {
  return fs.mkdir(directory, { recursive: true });
}

export async function removeDirectory(directory: string) {
  return fs.rm(directory, {
    recursive: true,
    // exceptions will be ignored if `path` does not exist.
    force: true,
  });
}

export async function removeFile(filePath: string) {
  return fs.rm(filePath);
}
