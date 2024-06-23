import { Dirent, promises as fs } from "fs";
import * as path from "path";
import { getWorkspacePath, prepareStoragePath } from "../build/utils";
import { ExtensionContext } from "./commands";
import { randomBytes } from "crypto";

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

export async function readJsonFile<T = any>(filePath: string): Promise<T> {
  const rawBuffer = await readFile(filePath);
  const rawString = rawBuffer.toString();
  return JSON.parse(rawString);
}

export function getWorkspaceRelativePath(filePath: string): string {
  const workspacePath = getWorkspacePath();
  return path.relative(workspacePath, filePath);
}

export async function tempFilePath(
  context: ExtensionContext,
  options: {
    prefix: string;
  },
) {
  // Where extension store some intermediate files
  const storagePath = await prepareStoragePath(context);

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
