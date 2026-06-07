import { getCurrentXcodeWorkspacePath } from "../../build/utils";
import { getTargets } from "../../common/cli/scripts";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES } from "../types";
import type { HandlerFn } from "./context";

export const targetList: HandlerFn<unknown, { targets: string[] }> = async (_params, ctx) => {
  const xcworkspace = getCurrentXcodeWorkspacePath(ctx.workspaceState);
  if (!xcworkspace) {
    throw new SweetpadRpcError(ERROR_CODES.NO_WORKSPACE, "No Xcode workspace detected for this folder.");
  }
  const targets = await getTargets({ xcworkspace });
  return { targets };
};
