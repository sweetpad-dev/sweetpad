import * as vscode from "vscode";

import { selectDestinationForBuild } from "../build/utils";
import { showYesNoQuestion } from "../common/askers";
import type { AppDeps } from "../common/commands";
import { getWorkspaceConfig, updateWorkspaceConfig } from "../common/config";
import { selectDestinationForTesting } from "../testing/utils";
import type { Destination } from "./types";
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
    await persistDestinationForBuild(deps, item.destination);
    return;
  }

  deps.progressStatusBar.updateText("Searching for destination");
  const destinations = await deps.destinationsManager.getDestinations({
    mostUsedSort: true,
  });

  const destination = await selectDestinationForBuild(deps.destinationsManager, {
    destinations: destinations,
    supportedPlatforms: undefined, // All platforms
  });

  await persistDestinationForBuild(deps, destination, { askToSaveSettings: true });
}

/**
 * Persist the build destination to settings (when already configured or when the user opts in)
 * or to workspace-state cache otherwise.
 */
async function persistDestinationForBuild(
  deps: AppDeps,
  destination: Destination,
  options?: { askToSaveSettings?: boolean },
): Promise<void> {
  let saveToSettings = false;
  if (options?.askToSaveSettings) {
    saveToSettings = await showYesNoQuestion({
      title: "Do you want to update the destination in the workspace settings (.vscode/settings.json)?",
    });
  } else if (getWorkspaceConfig("build.destination")) {
    // Setting already pins the destination — keep it in sync when picking from the tree.
    saveToSettings = true;
  }

  const selected = {
    id: destination.id,
    type: destination.type,
    name: destination.name,
  };

  if (saveToSettings) {
    await updateWorkspaceConfig("build.destination", selected);
    deps.destinationsManager.setWorkspaceDestinationForBuild(undefined);
  } else {
    deps.destinationsManager.setWorkspaceDestinationForBuild(destination);
  }
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
