import { selectDestinationForBuild } from "../build/utils";
import type { ExtensionContext } from "../common/context";
import { selectDestinationForTesting } from "../testing/utils";
import type { DestinationTreeItem } from "./tree";

export async function selectDestinationForBuildCommand(context: ExtensionContext, item?: DestinationTreeItem) {
  if (item) {
    context.destinationsManager.setWorkspaceDestinationForBuild(item.destination);
    return;
  }

  context.updateProgressStatus("Searching for destination");
  const destinations = await context.destinationsManager.getDestinations({
    mostUsedSort: true,
  });

  await selectDestinationForBuild(context, {
    destinations: destinations,
    supportedPlatforms: undefined, // All platforms
  });
}

export async function selectDestinationForTestingCommand(context: ExtensionContext, item?: DestinationTreeItem) {
  if (item) {
    context.destinationsManager.setWorkspaceDestinationForTesting(item.destination);
    return;
  }

  context.updateProgressStatus("Searching for destination");
  const destinations = await context.destinationsManager.getDestinations({
    mostUsedSort: true,
  });

  await selectDestinationForTesting(context, {
    destinations: destinations,
    supportedPlatforms: undefined,
  });
}

export async function removeRecentDestinationCommand(context: ExtensionContext, item?: DestinationTreeItem) {
  if (!item) {
    return;
  }

  const manager = context.destinationsManager;
  manager.removeRecentDestination(item.destination);
}
