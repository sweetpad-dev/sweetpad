import { Dirent, promises as fs } from "fs";
import * as path from "path";
import { getWorkspacePath } from "../build/utils";

/**
 * Find files or directories in a given directory
 */
export async function findFiles(options: { directory: string; matcher: (file: Dirent) => boolean }): Promise<string[]> {
  const files = await fs.readdir(options.directory, { withFileTypes: true });
  const matchedFiles: string[] = [];

  for (const file of files) {
    const fullPath = file.path;

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
}): Promise<string[]> {
  const ignore = options.ignore ?? [];
  const depth = options.depth ?? 0;

  const files = await fs.readdir(options.directory, { withFileTypes: true });
  const matchedFiles: string[] = [];

  for (const file of files) {
    const fullPath = path.join(file.path, file.name);

    if (options.matcher(file)) {
      matchedFiles.push(fullPath);
    }

    if (file.isDirectory() && !ignore.includes(file.name) && depth > 0) {
      const subFiles = await findFilesRecursive({
        directory: fullPath,
        matcher: options.matcher,
        ignore: options.ignore,
        depth: depth - 1,
      });
      matchedFiles.push(...subFiles);
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

export function getWorkspaceRelativePath(filePath: string): string {
  const workspacePath = getWorkspacePath();
  return path.relative(workspacePath, filePath);
}
