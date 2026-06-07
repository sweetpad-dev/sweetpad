import * as path from "node:path";

import { getCurrentXcodeWorkspacePath, prepareDerivedDataPath } from "../../build/utils";
import { getBuildSettingsList, type XcodeBuildSettings } from "../../common/cli/scripts";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES } from "../types";
import type { HandlerFn, RpcContext } from "./context";

type GetParams = { scheme?: string; configuration?: string; sdk?: string; xcworkspace?: string };

async function loadSettings(params: GetParams, ctx: RpcContext): Promise<XcodeBuildSettings[]> {
  const scheme = params?.scheme ?? ctx.buildManager.getDefaultSchemeForBuild();
  if (!scheme) {
    throw new SweetpadRpcError(ERROR_CODES.SCHEME_NOT_SET, "scheme is required (none persisted in workspace state)", {
      hint: "sweetpad scheme.set <name>",
    });
  }
  const configuration = params?.configuration ?? ctx.buildManager.getDefaultConfigurationForBuild() ?? "Debug";
  const xcworkspace = params?.xcworkspace ?? getCurrentXcodeWorkspacePath(ctx.workspace);
  if (!xcworkspace) {
    throw new SweetpadRpcError(ERROR_CODES.NO_WORKSPACE, "No Xcode workspace detected for this folder.");
  }
  try {
    return await getBuildSettingsList({ scheme, configuration, sdk: params?.sdk, xcworkspace });
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new SweetpadRpcError(ERROR_CODES.BUILD_SETTINGS_FAILED, message);
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
  const list = await loadSettings(params, ctx);
  for (const entry of list) {
    const buildDir = entry.settings.TARGET_BUILD_DIR;
    const wrapper = entry.settings.WRAPPER_NAME ?? entry.settings.FULL_PRODUCT_NAME;
    if (buildDir && wrapper) {
      return { appPath: path.join(buildDir, wrapper), target: entry.target };
    }
  }
  throw new SweetpadRpcError(
    ERROR_CODES.APP_PATH_NOT_FOUND,
    "No app bundle found in build settings — TARGET_BUILD_DIR/WRAPPER_NAME absent.",
  );
};

export const derivedDataPath: HandlerFn<unknown, { derivedDataPath: string | null }> = () => {
  return { derivedDataPath: prepareDerivedDataPath() };
};

export const bundleIdGet: HandlerFn<GetParams, { bundleIdentifier: string; target: string }> = async (params, ctx) => {
  const list = await loadSettings(params, ctx);
  for (const entry of list) {
    const id = entry.settings.PRODUCT_BUNDLE_IDENTIFIER;
    if (id) return { bundleIdentifier: id, target: entry.target };
  }
  throw new SweetpadRpcError(ERROR_CODES.BUNDLE_ID_NOT_FOUND, "PRODUCT_BUNDLE_IDENTIFIER not found in build settings.");
};
