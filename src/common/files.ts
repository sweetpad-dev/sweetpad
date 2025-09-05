import { randomBytes } from "node:crypto";
import { promises as fs, constants as fsConstants, type Dirent, type Stats } from "node:fs";
import * as path from "node:path";
import { getWorkspacePath, prepareStoragePath } from "../build/utils";
import type { ExtensionContext } from "./commands";

/**
 * Find files or directories in a given directory
 */
export async function findFiles(options: { 
  directory: string; 
  matcher: (file: Dirent) => boolean;
  maxResults?: number;
}): Promise<string[]> {
  try {
    const files = await fs.readdir(options.directory, { withFileTypes: true });
    const matchedFiles: string[] = [];

    for (const file of files) {
      // Early termination if we have enough results
      if (options.maxResults && matchedFiles.length >= options.maxResults) {
        break;
      }

      if (options.matcher(file)) {
        const fullPath = path.join(options.directory, file.name); // Fixed: construct path properly
        matchedFiles.push(fullPath);
      }
    }

    return matchedFiles;
  } catch (error) {
    // Handle permission errors gracefully
    if ((error as NodeJS.ErrnoException).code === 'EACCES' || 
        (error as NodeJS.ErrnoException).code === 'EPERM') {
      return [];
    }
    throw error;
  }
}

/**
 * Find files or directories in a given directory recursively
 */
export async function findFilesRecursive(options: {
  directory: string;
  matcher: (file: Dirent) => boolean;
  ignore?: string[];
  depth?: number;
  maxResults?: number; // Early termination for large searches
}): Promise<string[]> {
  const ignoreSet = new Set(options.ignore ?? []); // O(1) lookup instead of O(n)
  const depth = options.depth ?? 0;

  try {
    const files = await fs.readdir(options.directory, { withFileTypes: true });
    const matchedFiles: string[] = [];
    const subdirPromises: Promise<string[]>[] = [];

    // Process files and collect subdirectory promises in parallel
    for (const file of files) {
      // Early termination if we have enough results
      if (options.maxResults && matchedFiles.length >= options.maxResults) {
        break;
      }

      const fullPath = path.join(options.directory, file.name); // Use options.directory directly

      if (options.matcher(file)) {
        matchedFiles.push(fullPath);
      }

      // Process subdirectories in parallel
      if (file.isDirectory() && !ignoreSet.has(file.name) && depth > 0) {
        const remainingResults = options.maxResults ? options.maxResults - matchedFiles.length : undefined;
        const subDirPromise = findFilesRecursive({
          directory: fullPath,
          matcher: options.matcher,
          ignore: options.ignore,
          depth: depth - 1,
          maxResults: remainingResults,
        });
        subdirPromises.push(subDirPromise);
      }
    }

    // Wait for all subdirectory searches to complete in parallel
    if (subdirPromises.length > 0 && (!options.maxResults || matchedFiles.length < options.maxResults)) {
      const subResults = await Promise.all(subdirPromises);
      // Flatten results efficiently with max limit respect
      for (const subFiles of subResults) {
        if (options.maxResults && matchedFiles.length >= options.maxResults) {
          break;
        }
        
        if (options.maxResults) {
          const remainingSlots = options.maxResults - matchedFiles.length;
          matchedFiles.push(...subFiles.slice(0, remainingSlots));
        } else {
          matchedFiles.push(...subFiles);
        }
      }
    }

    return matchedFiles;
  } catch (error) {
    // Handle permission errors gracefully - common in filesystem traversal
    if ((error as NodeJS.ErrnoException).code === 'EACCES' || 
        (error as NodeJS.ErrnoException).code === 'EPERM') {
      return []; // Skip inaccessible directories
    }
    throw error;
  }
}

export async function isFileExists(filePath: string): Promise<boolean> {
  try {
    await fs.access(filePath, fsConstants.F_OK); // More explicit check for existence
    return true;
  } catch (error) {
    // Only return false for ENOENT (file not found), re-throw other errors
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
      return false;
    }
    // For permission errors or other issues, still return false but could log
    return false;
  }
}

export async function readFile(filePath: string): Promise<Buffer> {
  try {
    return await fs.readFile(filePath);
  } catch (error) {
    // Add context to errors for better debugging
    const err = error as NodeJS.ErrnoException;
    err.message = `Failed to read file '${filePath}': ${err.message}`;
    throw err;
  }
}

export async function statFile(filePath: string): Promise<Stats> {
  try {
    return await fs.stat(filePath);
  } catch (error) {
    // Add context to errors
    const err = error as NodeJS.ErrnoException;
    err.message = `Failed to stat file '${filePath}': ${err.message}`;
    throw err;
  }
}

export async function readTextFile(filePath: string, encoding: BufferEncoding = 'utf8'): Promise<string> {
  try {
    // Read as text directly instead of buffer -> string conversion
    return await fs.readFile(filePath, encoding);
  } catch (error) {
    const err = error as NodeJS.ErrnoException;
    err.message = `Failed to read text file '${filePath}': ${err.message}`;
    throw err;
  }
}

export async function readJsonFile<T = unknown>(filePath: string, encoding: BufferEncoding = 'utf8'): Promise<T> {
  try {
    // Read as text directly for better performance
    const rawString = await fs.readFile(filePath, encoding);
    return JSON.parse(rawString);
  } catch (error) {
    const err = error as Error;
    if (err instanceof SyntaxError) {
      // JSON parsing error
      throw new Error(`Invalid JSON in file '${filePath}': ${err.message}`);
    } else {
      // File reading error
      const fsErr = err as NodeJS.ErrnoException;
      fsErr.message = `Failed to read JSON file '${filePath}': ${fsErr.message}`;
      throw fsErr;
    }
  }
}

export function getWorkspaceRelativePath(filePath: string): string {
  const workspacePath = getWorkspacePath();
  return path.relative(workspacePath, filePath);
}

export async function tempFilePath(
  context: ExtensionContext,
  options: {
    prefix: string;
    extension?: string;
  },
) {
  try {
    // Where extension store some intermediate files
    const storagePath = await prepareStoragePath(context);

    // Directory for all temporary files
    const tempPath = path.join(storagePath, "_temp");
    await createDirectory(tempPath);

    // Generate more secure random file name with timestamp for uniqueness
    const timestamp = Date.now().toString(36); // Base36 for shorter string
    const random = randomBytes(6).toString("hex"); // Increased randomness
    const extension = options.extension ? `.${options.extension}` : '';
    const fileName = `${options.prefix}_${timestamp}_${random}${extension}`;
    const filePath = path.join(tempPath, fileName);
    
    return {
      path: filePath,
      [Symbol.asyncDispose]: async () => {
        try {
          await removeFile(filePath);
        } catch (error) {
          // Ignore errors during cleanup - file might already be deleted
          const err = error as NodeJS.ErrnoException;
          if (err.code !== 'ENOENT') {
            console.warn(`Failed to cleanup temp file ${filePath}:`, err.message);
          }
        }
      },
    };
  } catch (error) {
    const err = error as Error;
    throw new Error(`Failed to create temp file path: ${err.message}`);
  }
}

export async function createDirectory(directory: string): Promise<string | undefined> {
  try {
    return await fs.mkdir(directory, { recursive: true });
  } catch (error) {
    const err = error as NodeJS.ErrnoException;
    // Ignore if directory already exists
    if (err.code === 'EEXIST') {
      return undefined;
    }
    err.message = `Failed to create directory '${directory}': ${err.message}`;
    throw err;
  }
}

export async function removeDirectory(directory: string): Promise<void> {
  try {
    await fs.rm(directory, {
      recursive: true,
      // exceptions will be ignored if `path` does not exist.
      force: true,
    });
  } catch (error) {
    const err = error as NodeJS.ErrnoException;
    // Only throw if it's not a "not found" error
    if (err.code !== 'ENOENT') {
      err.message = `Failed to remove directory '${directory}': ${err.message}`;
      throw err;
    }
  }
}

export async function removeFile(filePath: string): Promise<void> {
  try {
    await fs.rm(filePath);
  } catch (error) {
    const err = error as NodeJS.ErrnoException;
    // Only throw if it's not a "not found" error
    if (err.code !== 'ENOENT') {
      err.message = `Failed to remove file '${filePath}': ${err.message}`;
      throw err;
    }
  }
}

/**
 * Check if a path is a directory
 */
export async function isDirectory(filePath: string): Promise<boolean> {
  try {
    const stats = await fs.stat(filePath);
    return stats.isDirectory();
  } catch (error) {
    const err = error as NodeJS.ErrnoException;
    if (err.code === 'ENOENT') {
      return false;
    }
    throw err;
  }
}

/**
 * Get file size in bytes
 */
export async function getFileSize(filePath: string): Promise<number> {
  try {
    const stats = await fs.stat(filePath);
    return stats.size;
  } catch (error) {
    const err = error as NodeJS.ErrnoException;
    err.message = `Failed to get size of file '${filePath}': ${err.message}`;
    throw err;
  }
}

/**
 * Copy a file with optional overwrite control
 */
export async function copyFile(source: string, destination: string, overwrite: boolean = true): Promise<void> {
  try {
    const flags = overwrite ? 0 : fsConstants.COPYFILE_EXCL;
    await fs.copyFile(source, destination, flags);
  } catch (error) {
    const err = error as NodeJS.ErrnoException;
    if (err.code === 'EEXIST' && !overwrite) {
      throw new Error(`Destination file '${destination}' already exists and overwrite is disabled`);
    }
    err.message = `Failed to copy file from '${source}' to '${destination}': ${err.message}`;
    throw err;
  }
}
