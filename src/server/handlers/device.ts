import { execa } from "execa";

import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES } from "../types";
import { requireString } from "./_common";
import type { HandlerFn } from "./context";

async function devicectl(args: string[], cwd: string): Promise<string> {
  try {
    const { stdout } = await execa("xcrun", ["devicectl", ...args], { cwd });
    return stdout;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new SweetpadRpcError(ERROR_CODES.DEVICECTL_FAILED, message);
  }
}

export const deviceInstall: HandlerFn<
  { deviceId?: string; appPath?: string },
  { deviceId: string; appPath: string }
> = async (params, ctx) => {
  const deviceId = requireString(params?.deviceId, "device.install", "deviceId");
  const appPath = requireString(params?.appPath, "device.install", "appPath");
  await devicectl(["device", "install", "app", "--device", deviceId, appPath], ctx.workspacePath);
  return { deviceId, appPath };
};

export const deviceLaunch: HandlerFn<
  {
    deviceId?: string;
    bundleId?: string;
    args?: string[];
    env?: Record<string, string>;
    terminateExisting?: boolean;
  },
  { deviceId: string; bundleId: string; pid: number | null }
> = async (params, ctx) => {
  const deviceId = requireString(params?.deviceId, "device.launch", "deviceId");
  const bundleId = requireString(params?.bundleId, "device.launch", "bundleId");
  const argv = ["device", "process", "launch"];
  if (params?.terminateExisting !== false) argv.push("--terminate-existing");
  argv.push("--device", deviceId, bundleId);
  if (params?.args) argv.push(...params.args);
  // devicectl forwards env entries prefixed with DEVICECTL_CHILD_ to the
  // launched process.
  const env: Record<string, string> = {};
  if (params?.env) {
    for (const [k, v] of Object.entries(params.env)) env[`DEVICECTL_CHILD_${k}`] = v;
  }
  let stdout: string;
  try {
    const result = await execa("xcrun", ["devicectl", ...argv], { cwd: ctx.workspacePath, env });
    stdout = result.stdout;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new SweetpadRpcError(ERROR_CODES.DEVICECTL_FAILED, message);
  }
  const m = /processIdentifier[":\s]*(\d+)/.exec(stdout);
  return { deviceId, bundleId, pid: m ? Number.parseInt(m[1], 10) : null };
};

export const deviceTerminate: HandlerFn<
  { deviceId?: string; bundleId?: string },
  { deviceId: string; bundleId: string }
> = async (params, ctx) => {
  const deviceId = requireString(params?.deviceId, "device.terminate", "deviceId");
  const bundleId = requireString(params?.bundleId, "device.terminate", "bundleId");
  await devicectl(["device", "process", "terminate", "--device", deviceId, bundleId], ctx.workspacePath);
  return { deviceId, bundleId };
};
