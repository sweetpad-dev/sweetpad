import { selectDestinationForBuild } from "../build/utils";
import type { CommandExecution } from "../common/commands";
import { selectDestinationForTesting } from "../testing/utils";
import type { DestinationTreeItem } from "./tree";

export async function selectDestinationForBuildCommand(execution: CommandExecution, item?: DestinationTreeItem) {
  if (item) {
    execution.context.destinationsManager.setWorkspaceDestinationForBuild(item.destination);
    return;
  }
  await selectDestinationForBuild(execution.context);
}

export async function selectDestinationForTestingCommand(execution: CommandExecution, item?: DestinationTreeItem) {
  if (item) {
    execution.context.destinationsManager.setWorkspaceDestinationForTesting(item.destination);
    return;
  }
  await selectDestinationForTesting(execution.context);
}
