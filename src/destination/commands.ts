import { selectDestination } from "../build/utils";
import type { CommandExecution } from "../common/commands";
import type { DestinationTreeItem } from "./tree";

export async function selectDestinationCommand(execution: CommandExecution, item?: DestinationTreeItem) {
  if (item) {
    execution.context.destinationsManager.setWorkspaceDestination(item.destination);
    return;
  }
  await selectDestination(execution.context);
}
