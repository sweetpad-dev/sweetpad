import * as vscode from "vscode";

import { generateXcodeGen, getIsXcodeGenInstalled } from "../../core/cli/scripts";
import { ExtensionError } from "../../core/errors";
import type { AppDeps } from "../commands";

export async function xcodgenGenerateCommand(deps: AppDeps): Promise<void> {
  const cliDeps = { cwd: deps.workspaceRoot.getPath(), config: deps.config, logger: deps.logger };

  const isServerInstalled = await getIsXcodeGenInstalled(cliDeps);
  if (!isServerInstalled) {
    throw new ExtensionError("XcodeGen is not installed");
  }

  deps.progressStatusBar.updateText("Running XcodeGen");
  await generateXcodeGen(cliDeps);

  // Restart LSP to catch changes
  await deps.lspRefresher.refresh();

  vscode.window.showInformationMessage("The Xcode project was successfully generated using XcodeGen.");
}
