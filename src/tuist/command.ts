import * as vscode from "vscode";
import { restartSwiftLSP } from "../build/utils";
import { getIsTuistInstalled, tuistClean, tuistEdit, tuistGenerate, tuistInstall, tuistTest } from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { ExtensionError } from "../common/errors";

async function tuistCheckInstalled() {
  const isTuistInstalled = await getIsTuistInstalled();
  if (!isTuistInstalled) {
    throw new ExtensionError("Tuist is not installed");
  }
}

export async function tuistGenerateCommand(context: ExtensionContext) {
  context.updateProgressStatus("Running 'tuist generate'");
  await tuistCheckInstalled();

  const raw = await tuistGenerate();
  if (raw.includes("tuist install")) {
    vscode.window.showErrorMessage(`Please run "tuist install" first`);
    return;
  }

  await restartSwiftLSP();

  vscode.window.showInformationMessage("The Xcode project was successfully generated using Tuist.");
}

export async function tuistInstallCommand(context: ExtensionContext) {
  context.updateProgressStatus("Running 'tuist install'");
  await tuistCheckInstalled();

  await tuistInstall();

  await restartSwiftLSP();

  vscode.window.showInformationMessage("The Swift Package was successfully installed using Tuist.");
}

export async function tuistCleanCommand(context: ExtensionContext) {
  context.updateProgressStatus("Running 'tuist clean'");
  await tuistCheckInstalled();

  await tuistClean();

  vscode.window.showInformationMessage("Tuist cleaned.");
}

export async function tuistEditComnmand(context: ExtensionContext) {
  context.updateProgressStatus("Running 'tuist edit'");
  await tuistCheckInstalled();

  await tuistEdit();
}

export async function tuistTestComnmand(context: ExtensionContext) {
  context.updateProgressStatus("Running 'tuist test'");
  await tuistCheckInstalled();

  await tuistTest();
}
