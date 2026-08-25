import * as vscode from "vscode";

import { selectDestinationForBuild } from "../build/utils";
import { showYesNoQuestion } from "../common/askers";
import type { AppDeps } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { selectDestinationForTesting } from "../testing/utils";
import type { DestinationTreeItem } from "./tree";
import { assertRunnableDestination } from "./utils";

/**
 * Trigger VS Code's built-in tree find on the Destinations view. Workaround until
 * `showFindControl` lands in TreeViewOptions (microsoft/vscode#173742).
 */
export async function searchDestinationsViewCommand(_deps: AppDeps) {
  await vscode.commands.executeCommand("sweetpad.destinations.view.focus");
  await vscode.commands.executeCommand("list.find");
}

export async function selectDestinationForBuildCommand(deps: AppDeps, item?: DestinationTreeItem) {
  if (item) {
    await deps.destinationsManager.persistDestinationForBuild(item.destination);
    return;
  }

  deps.progressStatusBar.updateText("Searching for destination");
  const destinations = await deps.destinationsManager.getDestinations({
    mostUsedSort: true,
  });

  // The picker applies the pick, so dismissing the prompt below still selects it.
  const destination = await selectDestinationForBuild(deps.destinationsManager, {
    destinations: destinations,
    supportedPlatforms: undefined, // All platforms
    action: "build",
  });
  if (getWorkspaceConfig("build.destination")) {
    return;
  }

  const pin = await showYesNoQuestion({
    title: "Do you want to update the destination in the workspace settings (.vscode/settings.json)?",
  });
  if (pin) {
    await deps.destinationsManager.persistDestinationForBuild(destination, { pin: true });
  }
}

export async function selectDestinationForTestingCommand(deps: AppDeps, item?: DestinationTreeItem) {
  if (item) {
    assertRunnableDestination(item.destination, "test");
    deps.destinationsManager.setWorkspaceDestinationForTesting(item.destination);
    return;
  }

  deps.progressStatusBar.updateText("Searching for destination");
  const destinations = await deps.destinationsManager.getDestinations({
    mostUsedSort: true,
  });

  await selectDestinationForTesting(deps.destinationsManager, {
    destinations: destinations,
    supportedPlatforms: undefined,
  });
}

export async function removeRecentDestinationCommand(deps: AppDeps, item?: DestinationTreeItem) {
  if (!item) {
    return;
  }

  const manager = deps.destinationsManager;
  manager.removeRecentDestination(item.destination);
}
