import * as vscode from "vscode";

import { restartSwiftLSP } from "../build/utils";
import { generateXcodeGen, getIsXcodeGenInstalled } from "../common/cli/scripts";
import type { AppDeps } from "../common/commands";
import { ExtensionError } from "../common/errors";

/**
 * `cwd` is the workspace folder holding the project.yml to generate. The watcher passes the folder
 * it detected; invoked from the command palette it is absent and the active folder is used.
 */
export async function xcodgenGenerateCommand(deps: AppDeps, cwd?: string): Promise<void> {
  const isServerInstalled = await getIsXcodeGenInstalled();
  if (!isServerInstalled) {
    throw new ExtensionError("XcodeGen is not installed");
  }

  deps.progressStatusBar.updateText("Running XcodeGen");
  await generateXcodeGen({ cwd: cwd });

  // Restart LSP to catch changes
  await restartSwiftLSP();

  vscode.window.showInformationMessage("The Xcode project was successfully generated using XcodeGen.");
}
