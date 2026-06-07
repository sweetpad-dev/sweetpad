import { getCurrentXcodeWorkspacePath } from "../../build/utils";
import { getSchemes } from "../../common/cli/scripts";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES, type SchemeEntity } from "../types";
import type { HandlerFn, RpcContext } from "./context";

async function loadSchemeNames(ctx: RpcContext): Promise<string[]> {
  const xcworkspace = getCurrentXcodeWorkspacePath(ctx.workspace);
  if (!xcworkspace) {
    throw new SweetpadRpcError(ERROR_CODES.NO_WORKSPACE, "No Xcode workspace detected for this folder.", {
      hint: "open the project in VS Code so SweetPad can detect the workspace",
    });
  }
  const schemes = await getSchemes({ xcworkspace });
  return schemes.map((s) => s.name);
}

export const schemeList: HandlerFn<unknown, { schemes: SchemeEntity[] }> = async (_params, ctx) => {
  const names = await loadSchemeNames(ctx);
  const selected = ctx.buildManager.getDefaultSchemeForBuild();
  const schemes: SchemeEntity[] = names.map((name) => ({ name, isSelected: name === selected }));
  return { schemes };
};

export const schemeGet: HandlerFn<unknown, { scheme: SchemeEntity | null }> = (_params, ctx) => {
  const name = ctx.buildManager.getDefaultSchemeForBuild();
  const scheme: SchemeEntity | null = name ? { name, isSelected: true } : null;
  return { scheme };
};

export const schemeSet: HandlerFn<{ name?: string }, { scheme: SchemeEntity }> = async (params, ctx) => {
  if (!params?.name || typeof params.name !== "string") {
    throw new SweetpadRpcError(ERROR_CODES.INVALID_PARAMS, "scheme.set requires { name: string }");
  }
  const names = await loadSchemeNames(ctx);
  if (!names.includes(params.name)) {
    throw new SweetpadRpcError(ERROR_CODES.SCHEME_NOT_FOUND, `Scheme not found: ${params.name}`, {
      hint: "sweetpad scheme list",
      data: { available: names },
    });
  }
  ctx.buildManager.setDefaultSchemeForBuild(params.name);
  return { scheme: { name: params.name, isSelected: true } satisfies SchemeEntity };
};
