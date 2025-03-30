import { selectDestinationForBuild } from "../build/utils";
import type { CommandExecution } from "../common/commands";
import { selectDestinationForTesting } from "../testing/utils";
import type { DestinationTreeItem } from "./tree";

export async function selectDestinationForBuildCommand(execution: CommandExecution, item?: DestinationTreeItem) {
  if (item) {
    execution.context.destinationsManager.setWorkspaceDestinationForBuild(item.destination);
    return;
  }
  const destinations = await execution.context.destinationsManager.getDestinations({
    mostUsedSort: true,
  });
  await selectDestinationForBuild(execution.context, {
    destinations: destinations,
    supportedPlatforms: undefined, // All platforms
  });
}

export async function selectDestinationForTestingCommand(execution: CommandExecution, item?: DestinationTreeItem) {
  if (item) {
    execution.context.destinationsManager.setWorkspaceDestinationForTesting(item.destination);
    return;
  }
  const destinations = await execution.context.destinationsManager.getDestinations({
    mostUsedSort: true,
  });
  await selectDestinationForTesting(execution.context, {
    destinations: destinations,
    supportedPlatforms: undefined,
  });
}

export async function removeRecentDestinationCommand(execution: CommandExecution, item?: DestinationTreeItem) {
  if (!item) {
    return;
  }

  const manager = execution.context.destinationsManager;
  manager.removeRecentDestination(item.destination);
}
