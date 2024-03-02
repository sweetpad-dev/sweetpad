import { CommandExecution } from "../common/commands";

export async function resetSweetpadCache(execution: CommandExecution) {
  execution.resetWorkspaceState();
}
