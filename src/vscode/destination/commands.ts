import * as vscode from "vscode";

import { selectDestinationForBuild } from "../../core/build/utils";
import type { AppDeps } from "../commands";
import { selectDestinationForTesting } from "../testing/utils";
import type { DestinationTreeItem } from "./tree";

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
    deps.destinationsManager.setWorkspaceDestinationForBuild(item.destination);
    return;
  }

  deps.progressStatusBar.updateText("Searching for destination");
  const destinations = await deps.destinationsManager.getDestinations({
    mostUsedSort: true,
  });

  await selectDestinationForBuild(deps.asker, deps.destinationsManager, {
    destinations: destinations,
    supportedPlatforms: undefined,
  });
}

export async function selectDestinationForTestingCommand(deps: AppDeps, item?: DestinationTreeItem) {
  if (item) {
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
