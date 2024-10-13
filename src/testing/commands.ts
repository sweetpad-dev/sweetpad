import { askXcodeWorkspacePath } from "../build/utils";
import type { CommandExecution } from "../common/commands";
import { askTestingTarget } from "./utils";

export async function selectTestingTarget(execution: CommandExecution): Promise<void> {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  await askTestingTarget(execution.context, {
    title: "Select default testing target",
    xcworkspace: xcworkspace,
    force: true,
  });
}
