import { getCurrentXcodeWorkspacePath } from "../../build/utils";
import { getBuildConfigurations } from "../../common/cli/scripts";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES, type ConfigurationEntity } from "../types";
import type { HandlerFn, RpcContext } from "./context";

async function loadConfigurations(ctx: RpcContext): Promise<string[]> {
  const xcworkspace = getCurrentXcodeWorkspacePath(ctx.workspaceState);
  if (!xcworkspace) {
    throw new SweetpadRpcError(ERROR_CODES.NO_WORKSPACE, "No Xcode workspace detected for this folder.");
  }
  const configs = await getBuildConfigurations({ xcworkspace });
  return configs.map((c) => c.name);
}

export const buildConfigList: HandlerFn<unknown, { configurations: ConfigurationEntity[] }> = async (_params, ctx) => {
  const names = await loadConfigurations(ctx);
  const selected = ctx.buildManager.getDefaultConfigurationForBuild();
  const configurations: ConfigurationEntity[] = names.map((name) => ({ name, isSelected: name === selected }));
  return { configurations };
};

export const buildConfigGet: HandlerFn<unknown, { configuration: ConfigurationEntity | null }> = (_params, ctx) => {
  const name = ctx.buildManager.getDefaultConfigurationForBuild();
  const configuration: ConfigurationEntity | null = name ? { name, isSelected: true } : null;
  return { configuration };
};

export const buildConfigSet: HandlerFn<{ name?: string }, { configuration: ConfigurationEntity }> = async (
  params,
  ctx,
) => {
  if (!params?.name || typeof params.name !== "string") {
    throw new SweetpadRpcError(ERROR_CODES.INVALID_PARAMS, "buildConfig.set requires { name: string }");
  }
  const available = await loadConfigurations(ctx);
  if (!available.includes(params.name)) {
    throw new SweetpadRpcError(ERROR_CODES.CONFIG_NOT_FOUND, `Configuration not found: ${params.name}`, {
      hint: "sweetpad buildConfig list",
      data: { available },
    });
  }
  ctx.buildManager.setDefaultConfigurationForBuild(params.name);
  return { configuration: { name: params.name, isSelected: true } satisfies ConfigurationEntity };
};
