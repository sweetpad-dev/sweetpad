import { getWorkspacePath } from "../build/utils";
import { CommandExecution } from "../common/commands";
import { findAndSaveXcodeWorkspace } from "./utils";

export async function setWorkspaceCommand(execution: CommandExecution) {
  const workspacePath = getWorkspacePath();
  await findAndSaveXcodeWorkspace(execution, {
    cwd: workspacePath,
  });
}
