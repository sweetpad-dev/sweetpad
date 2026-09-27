import { activateCurrentXcodeWorkspacePath, getWorkspaceRoot, prepareDerivedDataPath } from "../../build/utils";
import {
  type LaunchableApp,
  type XcodeBuildSettings,
  getBuildSettingsList,
  locateBuiltApp,
} from "../../common/cli/scripts";
import { methodHint } from "../method-catalog";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES, type ErrorCode } from "../types";
import type { HandlerFn, RpcContext } from "./context";

type GetParams = { scheme?: string; configuration?: string; sdk?: string; xcworkspace?: string };

/** The scheme, configuration and project a request names, or the ones the extension has selected. */
function selection(params: GetParams, ctx: RpcContext) {
  const scheme = params?.scheme ?? ctx.buildManager.getDefaultSchemeForBuild();
  if (!scheme) {
    throw new SweetpadRpcError(ERROR_CODES.SCHEME_NOT_SET, "scheme is required (none persisted in workspace state)", {
      hint: methodHint("scheme.set", "--name <name>"),
    });
  }
  const configuration = params?.configuration ?? ctx.buildManager.getDefaultConfigurationForBuild() ?? "Debug";
  const xcworkspace =
    params?.xcworkspace ??
    activateCurrentXcodeWorkspacePath({ workspaceState: ctx.workspaceState, workspaceContext: ctx.workspaceContext });
  if (!xcworkspace) {
    throw new SweetpadRpcError(ERROR_CODES.NO_WORKSPACE, "No Xcode workspace detected for this folder.");
  }
  // A caller-supplied `xcworkspace` skips the activation above, so the active folder can name a
  // different project than this one — resolve the root from the project instead.
  const workspaceRoot = getWorkspaceRoot({ xcworkspace, workspaceContext: ctx.workspaceContext });
  return { workspaceRoot, scheme, configuration, sdk: params?.sdk, xcworkspace };
}

async function loadSettings(params: GetParams, ctx: RpcContext): Promise<XcodeBuildSettings[]> {
  const selected = selection(params, ctx);
  try {
    return await getBuildSettingsList(selected);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new SweetpadRpcError(ERROR_CODES.BUILD_SETTINGS_FAILED, message);
  }
}

/**
 * The app a build of the selection produces, found the way a launch finds it: the scheme's Run action target,
 * narrowed to the requested SDK.
 */
async function locateApp(params: GetParams, ctx: RpcContext, notFound: ErrorCode): Promise<LaunchableApp> {
  const selected = selection(params, ctx);
  try {
    return await locateBuiltApp({ ...selected, destination: undefined });
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new SweetpadRpcError(notFound, message);
  }
}

export const buildSettingsGet: HandlerFn<
  GetParams & { keys?: string[] },
  { targets: { target: string; settings: Record<string, string> }[] }
> = async (params, ctx) => {
  const list = await loadSettings(params, ctx);
  const allow = params?.keys && params.keys.length > 0 ? new Set(params.keys) : undefined;
  const targets = list.map((entry) => ({
    target: entry.target,
    settings: allow ? Object.fromEntries(Object.entries(entry.settings).filter(([k]) => allow.has(k))) : entry.settings,
  }));
  return { targets };
};

export const appPathFind: HandlerFn<GetParams, { appPath: string; target: string }> = async (params, ctx) => {
  const app = await locateApp(params, ctx, ERROR_CODES.APP_PATH_NOT_FOUND);
  return { appPath: app.appPath, target: app.target };
};

export const derivedDataPath: HandlerFn<unknown, { derivedDataPath: string | null }> = (_params, ctx) => {
  return { derivedDataPath: prepareDerivedDataPath({ workspaceRoot: ctx.workspacePath }) };
};

export const bundleIdGet: HandlerFn<GetParams, { bundleIdentifier: string; target: string }> = async (params, ctx) => {
  const app = await locateApp(params, ctx, ERROR_CODES.BUNDLE_ID_NOT_FOUND);
  return { bundleIdentifier: app.bundleIdentifier, target: app.target };
};
