import { promises as fs, Stats } from "fs";
import * as path from "path";

/**
 * Find files or directories in a given directory
 */
export async function findFiles(
  directory: string,
  matcher: (file: string, stats: Stats) => boolean
): Promise<string[]> {
  const files = await fs.readdir(directory);
  const matchedFiles: string[] = [];

  for (const file of files) {
    const fullPath = path.join(directory, file);
    const stats = await fs.stat(fullPath);

    if (matcher(file, stats)) {
      matchedFiles.push(fullPath);
    }
  }

  return matchedFiles;
}

/**
 * Find files or directories in a given directory recursively
 */
export async function findFilesRecursive(
  directory: string,
  matcher: (file: string, stats: Stats) => boolean,
  options: { ignore?: string[]; depth?: number } = {}
): Promise<string[]> {
  const ignore = options.ignore ?? [];
  const depth = options.depth ?? 0;

  const files = await fs.readdir(directory);
  const matchedFiles: string[] = [];

  for (const file of files) {
    const fullPath = path.join(directory, file);
    const stats = await fs.stat(fullPath);

    if (matcher(file, stats)) {
      matchedFiles.push(fullPath);
    }

    if (stats.isDirectory() && !ignore.includes(file) && depth > 0) {
      const subFiles = await findFilesRecursive(fullPath, matcher, {
        ignore,
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
