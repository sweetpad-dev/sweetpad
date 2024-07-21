import { selectDestination } from "../build/utils";
import { CommandExecution } from "../common/commands";
import { DestinationTreeItem } from "./tree";

export async function selectDestinationCommand(execution: CommandExecution, item?: DestinationTreeItem) {
  if (item) {
    execution.context.destinationsManager.setWorkspaceDestination(item.destination);
    return;
  }
  await selectDestination(execution.context);
}
