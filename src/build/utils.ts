import path from "path";
import * as vscode from "vscode";
import { showQuickPick } from "../common/quick-pick";

import {
  SimulatorOutput,
  createDirectory,
  getBuildConfigurations,
  getSchemes,
  getSimulators,
  removeDirectory,
} from "../common/cli/scripts";
import { CommandExecution } from "../common/commands";
import { ExtensionError } from "../common/errors";
import { findFilesRecursive, isFileExists } from "../common/files";
import { commonLogger } from "../common/logger";

const DEFAULT_CONFIGURATION = "Debug";

/**
 * Ask user to select simulator to run on using quick pick
 */
export async function askSimulatorToRunOn(): Promise<SimulatorOutput> {
  const output = await getSimulators();

  const device = await showQuickPick({
    title: "Select simulator to run on",
    items: Object.entries(output.devices)
      .map(([key, value]) => {
        return value
          .filter((simulator) => simulator.isAvailable)
          .map((simulator) => {
            return {
              label: simulator.name,
              context: {
                simulator,
              },
            };
          });
      })
      .flat(),
  });

  return device.context.simulator;
}

/**
 * Ask user to select scheme to build
 */
export async function askScheme(options?: { title?: string }): Promise<string> {
  const schemes = await getSchemes();

  const scheme = await showQuickPick({
    title: options?.title ?? "Select scheme to build",
    items: schemes.map((scheme) => {
      return {
        label: scheme.name,
        context: {
          scheme: scheme,
        },
      };
    }),
  });

  return scheme.context.scheme.name;
}

/**
 * It's path to current opened workspace
 */
export function getWorkspacePath(): string {
  const workspaceFolder = vscode.workspace.workspaceFolders?.[0].uri.fsPath;
  if (!workspaceFolder) {
    throw new ExtensionError("No workspace folder found");
  }
  return workspaceFolder;
}

/**
 * Prepare storage path for the extension. It's a folder where we store all intermediate files
 */
export async function prepareStoragePath(execution: CommandExecution): Promise<string> {
  const storagePath = execution.context.storageUri?.fsPath;
  if (!storagePath) {
    throw new ExtensionError("No storage path found");
  }
  // Creatre folder at storagePath, because vscode doesn't create it automatically
  await createDirectory(storagePath);
  return storagePath;
}

/**
 * Prepare bundle directory for the given schema in the storage path
 */
export async function prepareBundleDir(execution: CommandExecution, schema: string): Promise<string> {
  const storagePath = await prepareStoragePath(execution);

  const bundleDir = path.join(storagePath, "bundle", schema);

  // Remove old bundle if exists
  await removeDirectory(bundleDir);

  // Remove old .xcresult if exists
  const xcresult = path.join(storagePath, "bundle", `${schema}.xcresult`);
  await removeDirectory(xcresult);

  return bundleDir;
}

export async function askXcodeWorkspacePath(execution: CommandExecution, options: { cwd: string }): Promise<string> {
  return await execution.withPathCache("build.xcodeWorkspacePath", async () => {
    return await selectXcodeWorkspace();
  });
}

export async function askConfiguration(execution: CommandExecution): Promise<string> {
  return await execution.withCache("build.xcodeConfiguration", async () => {
    // Fetch all configurations
    const configurations = await getBuildConfigurations();

    // Use default configuration if it exists
    if (configurations.some((configuration) => configuration.name === DEFAULT_CONFIGURATION)) {
      return DEFAULT_CONFIGURATION;
    }

    // Give user a choice to select configuration if we don't know wich one to use
    const selected = await showQuickPick({
      title: "Select configuration",
      items: configurations.map((configuration) => {
        return {
          label: configuration.name,
          context: {
            configuration,
          },
        };
      }),
    });
    return selected.context.configuration.name;
  });
}

/**
 * Detect xcode workspace in the given directory
 */
export async function detectXcodeProjectPaths(): Promise<string[]> {
  const workspace = getWorkspacePath();

  // Get all files that end with .xcworkspace (4 depth)
  const paths = await findFilesRecursive(
    workspace,
    (file, stats) => {
      return stats.isDirectory() && file.endsWith(".xcworkspace");
    },
    {
      depth: 4,
    }
  );
  return paths;
}

/**
 * Find xcode workspace in the given directory and ask user to select it
 */
export async function selectXcodeWorkspace(): Promise<string> {
  const workspace = getWorkspacePath();
  let path: string | undefined;
  // Get all files that end with .xcworkspace (4 depth)
  const paths = await detectXcodeProjectPaths();

  // No files, nothing to do
  if (paths.length === 0) {
    throw new ExtensionError("No xcode workspaces found", {
      cwd: workspace,
    });
  }

  // One file, use it and save it to the cache
  if (paths.length === 1) {
    path = paths[0];
    commonLogger.log("Xcode workspace was detected", {
      workspace: workspace,
      path: path,
    });
    return path;
  }

  // More then one, ask user to select
  const selected = await showQuickPick({
    title: "Select xcode workspace",
    items: paths.map((path) => {
      return {
        label: path,
        context: { path },
      };
    }),
  });
  return selected.context.path;
}
