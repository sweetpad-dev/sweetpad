import * as vscode from "vscode";

import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES } from "../types";
import { requireString } from "./_common";
import type { HandlerFn } from "./context";

const CONFIG_NS = "sweetpad";

type SettingsTarget = "global" | "workspace" | "workspaceFolder";

type InspectResult = {
  key: string;
  default: unknown;
  global: unknown;
  workspace: unknown;
  workspaceFolder: unknown;
  effective: unknown;
};

export const vscodeExecuteCommand: HandlerFn<{ command?: string; args?: unknown[] }, { result: unknown }> = async (
  params,
) => {
  const command = requireString(params?.command, "vscode.executeCommand", "command");
  const args = Array.isArray(params?.args) ? params!.args : [];
  try {
    const result = await vscode.commands.executeCommand(command, ...args);
    return { result: serializable(result) };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new SweetpadRpcError(ERROR_CODES.VSCODE_COMMAND_FAILED, `${command}: ${message}`);
  }
};

export const vscodeSettingsGet: HandlerFn<{ key?: string }, { key: string; value: unknown }> = (params) => {
  const key = requireKey(params?.key, "vscodeSettings.get");
  return { key, value: vscode.workspace.getConfiguration(CONFIG_NS).get(key) };
};

export const vscodeSettingsSet: HandlerFn<
  { key?: string; value?: unknown; target?: string },
  { key: string; value: unknown; target: SettingsTarget }
> = async (params) => {
  const key = requireKey(params?.key, "vscodeSettings.set");
  const target = parseTarget(params?.target);
  const cfg = vscode.workspace.getConfiguration(CONFIG_NS);
  const value = params?.value === null ? undefined : params?.value;
  // Skip the write when nothing would change — VS Code fires a config event on
  // every update, even for no-op assignments, which cascades to every listener.
  const inspect = cfg.inspect(key);
  const currentAtTarget =
    target === "global"
      ? inspect?.globalValue
      : target === "workspaceFolder"
        ? inspect?.workspaceFolderValue
        : inspect?.workspaceValue;
  if (currentAtTarget !== value) {
    await cfg.update(key, value, configurationTarget(target));
  }
  return { key, value: cfg.get(key), target };
};

export const vscodeSettingsInspect: HandlerFn<{ key?: string }, InspectResult> = (params) => {
  const key = requireKey(params?.key, "vscodeSettings.inspect");
  const cfg = vscode.workspace.getConfiguration(CONFIG_NS);
  const inspect = cfg.inspect(key);
  return {
    key,
    default: inspect?.defaultValue,
    global: inspect?.globalValue,
    workspace: inspect?.workspaceValue,
    workspaceFolder: inspect?.workspaceFolderValue,
    effective: cfg.get(key),
  };
};

export const vscodeSettingsList: HandlerFn<unknown, { settings: { key: string; value: unknown }[] }> = (
  _params,
  ctx,
) => {
  const cfg = vscode.workspace.getConfiguration(CONFIG_NS);
  return { settings: ctx.configKeys.map((key) => ({ key, value: cfg.get(key) })) };
};

const requireKey = (key: unknown, method: string) => requireString(key, method, "key");

function parseTarget(value: unknown): SettingsTarget {
  if (value === undefined || value === null) return "workspace";
  if (value === "global" || value === "workspace" || value === "workspaceFolder") return value;
  throw new SweetpadRpcError(
    ERROR_CODES.INVALID_PARAMS,
    `Unknown target: ${String(value)}. Expected global|workspace|workspaceFolder.`,
  );
}

function configurationTarget(t: SettingsTarget): vscode.ConfigurationTarget {
  if (t === "global") return vscode.ConfigurationTarget.Global;
  if (t === "workspaceFolder") return vscode.ConfigurationTarget.WorkspaceFolder;
  return vscode.ConfigurationTarget.Workspace;
}

// Drop non-JSON-serializable command results (functions, Maps, ...) so the
// JSON-RPC envelope can't fail mid-stringify.
function serializable(value: unknown): unknown {
  try {
    JSON.stringify(value);
    return value ?? null;
  } catch {
    return null;
  }
}
