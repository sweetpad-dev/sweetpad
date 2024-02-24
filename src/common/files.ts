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
