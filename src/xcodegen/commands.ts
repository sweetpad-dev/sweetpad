import { restartSwiftLSP } from "../build/utils";
import { generateXcodeGen, getIsXcodeGenInstalled } from "../common/cli/scripts";
import { ExtensionError } from "../common/errors";
import * as vscode from "vscode";

export async function xcodgenGenerateCommand() {
  const isServerInstalled = await getIsXcodeGenInstalled();
  if (!isServerInstalled) {
    throw new ExtensionError("XcodeGen is not installed");
  }
  await generateXcodeGen();

  // Restart LSP to catch changes
  await restartSwiftLSP();

  vscode.window.showInformationMessage(`The Xcode project was successfully generated using XcodeGen.`);
}
