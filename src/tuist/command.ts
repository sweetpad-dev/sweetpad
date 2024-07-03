import { restartSwiftLSP } from "../build/utils";
import { tuistClean, tuistEdit, tuistInstall, tuistGenerate, getIsTuistInstalled } from "../common/cli/scripts";
import { ExtensionError } from "../common/errors";
import * as vscode from "vscode";

async function tuistCheckInstalled() {
  const isTuistInstalled = await getIsTuistInstalled();
  if (!isTuistInstalled) {
    throw new ExtensionError("Tuist is not installed");
  }
}

export async function tuistGenerateCommand() {
  await tuistCheckInstalled();

  const raw = await tuistGenerate();
  if (raw.includes("tuist install")) {
    vscode.window.showErrorMessage(`Please run "tuist install" first`);
    return;
  }

  await restartSwiftLSP();

  vscode.window.showInformationMessage(`The Xcode project was successfully generated using Tuist.`);
}

export async function tuistInstallCommand() {
  await tuistCheckInstalled();

  await tuistInstall();

  await restartSwiftLSP();

  vscode.window.showInformationMessage(`The Swift Package was successfully installed using Tuist.`);
}

export async function tuistCleanCommand() {
  await tuistCheckInstalled();

  await tuistClean();

  vscode.window.showInformationMessage(`Tuist cleaned.`);
}

export async function tuistEditComnmand() {
  await tuistCheckInstalled();

  await tuistEdit();
}
