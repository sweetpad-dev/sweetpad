import { restartSwiftLSP } from "../build/utils";
import { cleanTuist, editTuist, fetchTuist, generateTuist, getIsTuistInstalled } from "../common/cli/scripts";
import { ExtensionError } from "../common/errors";
import * as vscode from "vscode";
import { commonLogger } from "../common/logger";
import { buildCommand, generateBuildServerConfigCommand } from "../build/commands";
import { CommandExecution } from "../common/commands";

export async function tuistGenerateCommand() {
  const isTuistInstalled = await getIsTuistInstalled();
  if (!isTuistInstalled) {
    throw new ExtensionError("Tuist is not installed");
  }

  const raw = await generateTuist();
  if (raw.includes("tuist install")) {
    vscode.window.showErrorMessage(`Please run "tuist install" first`);
    return;
  }

  await restartSwiftLSP();

  vscode.window.showInformationMessage(`The Xcode project was successfully generated using Tuist.`);
}

export async function tuistFetchCommand() {
  const isTuistInstalled = await getIsTuistInstalled();
  if (!isTuistInstalled) {
    throw new ExtensionError("Tuist is not installed");
  }

  await fetchTuist();

  await restartSwiftLSP();

  vscode.window.showInformationMessage(`The Swift Package was successfully installed using Tuist.`);
}

export async function tuistCleanCommand() {
  const isTuistInstalled = await getIsTuistInstalled();
  if (!isTuistInstalled) {
    throw new ExtensionError("Tuist is not installed");
  }

  await cleanTuist();

  vscode.window.showInformationMessage(`Tuist cleaned.`);
}

export async function tuistEditComnmand() {
  const isTuistInstalled = await getIsTuistInstalled();
  if (!isTuistInstalled) {
    throw new ExtensionError("Tuist is not installed");
  }

  await editTuist();
}
