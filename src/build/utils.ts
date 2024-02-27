import path from "path";
import * as vscode from "vscode";
import { showQuickPick } from "../common/quick-pick";

import { SimulatorOutput, createDirectory, getSchemes, getSimulators, removeDirectory } from "../common/cli/scripts";
import { CommandExecution } from "../common/commands";
import { ExtensionError } from "../common/errors";
import { findFilesRecursive, isFileExists } from "../common/files";
import { commonLogger } from "../common/logger";
import { findAndSaveXcodeWorkspace } from "../system/utils";

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
        label: scheme,
        context: {
          scheme,
        },
      };
    }),
  });

  return scheme.context.scheme;
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
  // try get from config
  const config = vscode.workspace.getConfiguration("sweetpad");
  const pathConfig = config.get("build.xcodeWorkspacePath");
  if (pathConfig) {
    return pathConfig as string;
  }

  const pathCache = execution.xcodeWorkspacePath;
  if (pathCache) {
    if (!(await isFileExists(pathCache as string))) {
      execution.xcodeWorkspacePath = undefined;
    } else {
      return pathCache as string;
    }
  }

  const pathSelected = await findAndSaveXcodeWorkspace(execution, options);
  return pathSelected;
}
