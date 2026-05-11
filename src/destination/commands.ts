import { selectDestinationForBuild } from "../build/utils";
import type { AppDeps } from "../common/commands";
import { selectDestinationForTesting } from "../testing/utils";
import type { DestinationTreeItem } from "./tree";

export async function selectDestinationForBuildCommand(deps: AppDeps, item?: DestinationTreeItem) {
  if (item) {
    deps.destinationsManager.setWorkspaceDestinationForBuild(item.destination);
    return;
  }

  deps.progressStatusBar.updateText("Searching for destination");
  const destinations = await deps.destinationsManager.getDestinations({
    mostUsedSort: true,
  });

  await selectDestinationForBuild(deps.destinationsManager, {
    destinations: destinations,
    supportedPlatforms: undefined, // All platforms
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
