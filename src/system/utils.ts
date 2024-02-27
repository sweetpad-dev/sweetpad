import { CommandExecution } from "../common/commands";
import { ExtensionError } from "../common/errors";
import { findFilesRecursive } from "../common/files";
import { commonLogger } from "../common/logger";
import { showQuickPick } from "../common/quick-pick";

export async function findAndSaveXcodeWorkspace(
  execution: CommandExecution,
  options: { cwd: string }
): Promise<string> {
  // Get all files that end with .xcworkspace (4 depth)
  const files = await findFilesRecursive(
    options.cwd,
    (file, stats) => {
      return stats.isDirectory() && file.endsWith(".xcworkspace");
    },
    {
      depth: 4,
    }
  );

  // No files, nothing to do
  if (files.length === 0) {
    throw new ExtensionError("No xcode workspaces found", {
      cwd: options.cwd,
    });
  }

  // One file, use it and save it to the cache
  if (files.length === 1) {
    commonLogger.log("Xcode workspace was detected", {
      cwd: options.cwd,
      file: files[0],
    });
    execution.xcodeWorkspacePath = files[0];
    return files[0];
  }

  // More then one, ask user to select
  const workspace = await showQuickPick({
    title: "Select xcode workspace",
    items: files.map((file) => {
      return {
        label: file,
        context: { file },
      };
    }),
  });

  // Save selected workspace to the cache
  const pathSelected = workspace.context.file;
  execution.xcodeWorkspacePath = pathSelected;

  return pathSelected;
}
