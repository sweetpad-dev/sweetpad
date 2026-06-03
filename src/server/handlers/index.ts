import type { RpcDispatch } from "../rpc";
import { buildDiagnostics, buildList, buildLogs, buildStart, buildStatus, buildStop, buildWait } from "./build";
import { buildConfigGet, buildConfigList, buildConfigSet } from "./build-config";
import { appPathFind, bundleIdGet, buildSettingsGet, derivedDataPath } from "./build-settings";
import type { HandlerFn, RpcContext } from "./context";
import { destinationGet, destinationList, destinationSet } from "./destination";
import { deviceInstall, deviceLaunch, deviceTerminate } from "./device";
import { logsTail } from "./logs";
import { metaSchema, metaUsage, metaVersion, metaWorkspacePath } from "./meta";
import { schemeGet, schemeList, schemeSet } from "./scheme";
import { schemeReveal } from "./scheme-file";
import { simulatorList, simulatorRefresh, simulatorStart, simulatorStop } from "./simulator";
import {
  simulatorInstall,
  simulatorLaunchApp,
  simulatorOpenUrl,
  simulatorScreenshot,
  simulatorTerminateApp,
  simulatorUninstall,
} from "./simulator-app";
import { stateGet } from "./state";
import { targetList } from "./target";
import {
  vscodeExecuteCommand,
  vscodeSettingsGet,
  vscodeSettingsInspect,
  vscodeSettingsList,
  vscodeSettingsSet,
} from "./vscode";
import { workspaceDetect, workspaceRecent, workspaceUse } from "./workspace";
import { workspaceStateDelete, workspaceStateGet, workspaceStateKeys, workspaceStateSet } from "./workspace-state";

/**
 * Bind every handler to the per-server context and return the dispatch table
 * the JSON-RPC layer consumes.
 */
export function buildDispatch(ctx: RpcContext): RpcDispatch {
  const bind =
    <P, R>(fn: HandlerFn<P, R>) =>
    (params: unknown) =>
      fn(params as P, ctx);

  return {
    "meta.usage": bind(metaUsage),
    "meta.schema": bind(metaSchema),
    "meta.version": bind(metaVersion),
    "meta.workspacePath": bind(metaWorkspacePath),

    "state.get": bind(stateGet),

    "scheme.list": bind(schemeList),
    "scheme.get": bind(schemeGet),
    "scheme.set": bind(schemeSet),
    "scheme.reveal": bind(schemeReveal),

    "destination.list": bind(destinationList),
    "destination.get": bind(destinationGet),
    "destination.set": bind(destinationSet),

    "simulator.list": bind(simulatorList),
    "simulator.start": bind(simulatorStart),
    "simulator.stop": bind(simulatorStop),
    "simulator.refresh": bind(simulatorRefresh),
    "simulator.install": bind(simulatorInstall),
    "simulator.uninstall": bind(simulatorUninstall),
    "simulator.launchApp": bind(simulatorLaunchApp),
    "simulator.terminateApp": bind(simulatorTerminateApp),
    "simulator.openUrl": bind(simulatorOpenUrl),
    "simulator.screenshot": bind(simulatorScreenshot),

    "device.install": bind(deviceInstall),
    "device.launch": bind(deviceLaunch),
    "device.terminate": bind(deviceTerminate),

    "buildConfig.list": bind(buildConfigList),
    "buildConfig.get": bind(buildConfigGet),
    "buildConfig.set": bind(buildConfigSet),

    "buildSettings.get": bind(buildSettingsGet),
    "target.list": bind(targetList),
    "appPath.find": bind(appPathFind),
    "derivedData.path": bind(derivedDataPath),
    "bundleId.get": bind(bundleIdGet),

    "build.start": bind(buildStart),
    "build.stop": bind(buildStop),
    "build.wait": bind(buildWait),
    "build.status": bind(buildStatus),
    "build.list": bind(buildList),
    "build.logs": bind(buildLogs),
    "build.diagnostics": bind(buildDiagnostics),

    "workspace.detect": bind(workspaceDetect),
    "workspace.use": bind(workspaceUse),
    "workspace.recent": bind(workspaceRecent),

    "workspaceState.get": bind(workspaceStateGet),
    "workspaceState.set": bind(workspaceStateSet),
    "workspaceState.keys": bind(workspaceStateKeys),
    "workspaceState.delete": bind(workspaceStateDelete),

    "vscode.executeCommand": bind(vscodeExecuteCommand),
    "vscodeSettings.get": bind(vscodeSettingsGet),
    "vscodeSettings.set": bind(vscodeSettingsSet),
    "vscodeSettings.inspect": bind(vscodeSettingsInspect),
    "vscodeSettings.list": bind(vscodeSettingsList),

    "logs.tail": bind(logsTail),
  };
}
