import * as vscode from "vscode";

import {
  getIsTuistInstalled,
  tuistClean,
  tuistEdit,
  tuistGenerate,
  tuistInstall,
  tuistTest,
} from "../../core/cli/scripts";
import { ExtensionError } from "../../core/errors";
import type { AppDeps } from "../commands";

function cliDeps(deps: AppDeps) {
  return { cwd: deps.workspaceRoot.getPath(), config: deps.config, logger: deps.logger };
}

async function tuistCheckInstalled(deps: AppDeps) {
  const isTuistInstalled = await getIsTuistInstalled(cliDeps(deps));
  if (!isTuistInstalled) {
    throw new ExtensionError("Tuist is not installed");
  }
}

export async function tuistGenerateCommand(deps: AppDeps) {
  deps.progressStatusBar.updateText("Running 'tuist generate'");
  await tuistCheckInstalled(deps);

  const raw = await tuistGenerate(cliDeps(deps));
  if (raw.includes("tuist install")) {
    vscode.window.showErrorMessage(`Please run "tuist install" first`);
    return;
  }

  await deps.lspRefresher.refresh();

  vscode.window.showInformationMessage("The Xcode project was successfully generated using Tuist.");
}

export async function tuistInstallCommand(deps: AppDeps) {
  deps.progressStatusBar.updateText("Running 'tuist install'");
  await tuistCheckInstalled(deps);

  await tuistInstall(cliDeps(deps));

  await deps.lspRefresher.refresh();

  vscode.window.showInformationMessage("The Swift Package was successfully installed using Tuist.");
}

export async function tuistCleanCommand(deps: AppDeps) {
  deps.progressStatusBar.updateText("Running 'tuist clean'");
  await tuistCheckInstalled(deps);

  await tuistClean(cliDeps(deps));

  vscode.window.showInformationMessage("Tuist cleaned.");
}

export async function tuistEditComnmand(deps: AppDeps) {
  deps.progressStatusBar.updateText("Running 'tuist edit'");
  await tuistCheckInstalled(deps);

  await tuistEdit(cliDeps(deps));
}

export async function tuistTestComnmand(deps: AppDeps) {
  deps.progressStatusBar.updateText("Running 'tuist test'");
  await tuistCheckInstalled(deps);

  await tuistTest(cliDeps(deps));
}
