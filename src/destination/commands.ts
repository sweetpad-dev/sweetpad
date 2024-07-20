import { selectDestination } from "../build/utils";
import { CommandExecution } from "../common/commands";

export async function selectDestinationCommand(execution: CommandExecution) {
  await selectDestination(execution.context);
}
