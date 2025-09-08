import * as vscode from "vscode";
import { restartSwiftLSP } from "../build/utils";
import { generateXcodeGen, getIsXcodeGenInstalled } from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { ExtensionError } from "../common/errors";

export async function xcodgenGenerateCommand(context: ExtensionContext): Promise<void> {
  const isServerInstalled = await getIsXcodeGenInstalled();
  if (!isServerInstalled) {
    throw new ExtensionError("XcodeGen is not installed");
  }

  context.updateProgressStatus("Running XcodeGen");
  await generateXcodeGen();

  // Restart LSP to catch changes
  await restartSwiftLSP();

  vscode.window.showInformationMessage("✅ The Xcode project was successfully generated using XcodeGen.");
}
