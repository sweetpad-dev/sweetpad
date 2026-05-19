import { execa } from "execa";

import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES } from "../types";
import { requireString } from "./_common";
import type { HandlerFn } from "./context";
import { findSimulator } from "./simulator";

async function simctl(args: string[], cwd: string): Promise<string> {
  try {
    const { stdout } = await execa("xcrun", ["simctl", ...args], { cwd });
    return stdout;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new SweetpadRpcError(ERROR_CODES.SIMCTL_FAILED, message);
  }
}

export const simulatorInstall: HandlerFn<
  { udid?: string; appPath?: string },
  { udid: string; appPath: string }
> = async (params, ctx) => {
  const udid = requireString(params?.udid, "simulator.install", "udid");
  const appPath = requireString(params?.appPath, "simulator.install", "appPath");
  const sim = await findSimulator(ctx, udid, { requireBooted: true });
  await simctl(["install", sim.udid, appPath], ctx.workspacePath);
  return { udid: sim.udid, appPath };
};

export const simulatorUninstall: HandlerFn<
  { udid?: string; bundleId?: string },
  { udid: string; bundleId: string }
> = async (params, ctx) => {
  const udid = requireString(params?.udid, "simulator.uninstall", "udid");
  const bundleId = requireString(params?.bundleId, "simulator.uninstall", "bundleId");
  const sim = await findSimulator(ctx, udid, { requireBooted: true });
  await simctl(["uninstall", sim.udid, bundleId], ctx.workspacePath);
  return { udid: sim.udid, bundleId };
};

export const simulatorLaunchApp: HandlerFn<
  { udid?: string; bundleId?: string; args?: string[]; env?: Record<string, string>; waitForDebugger?: boolean },
  { udid: string; bundleId: string; pid: number | null }
> = async (params, ctx) => {
  const udid = requireString(params?.udid, "simulator.launchApp", "udid");
  const bundleId = requireString(params?.bundleId, "simulator.launchApp", "bundleId");
  const sim = await findSimulator(ctx, udid, { requireBooted: true });
  const argv = ["launch"];
  if (params?.waitForDebugger) argv.push("--wait-for-debugger");
  argv.push(sim.udid, bundleId);
  if (params?.args) argv.push(...params.args);
  // simctl prefixes SIMCTL_CHILD_<name> onto the spawned process's env, so the
  // agent passes plain key/value pairs.
  const env: Record<string, string> = {};
  if (params?.env) {
    for (const [k, v] of Object.entries(params.env)) env[`SIMCTL_CHILD_${k}`] = v;
  }
  let stdout: string;
  try {
    const result = await execa("xcrun", ["simctl", ...argv], { env, cwd: ctx.workspacePath });
    stdout = result.stdout;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new SweetpadRpcError(ERROR_CODES.SIMCTL_FAILED, message);
  }
  const m = /:\s*(\d+)\s*$/.exec(stdout.trim());
  return { udid: sim.udid, bundleId, pid: m ? Number.parseInt(m[1], 10) : null };
};

export const simulatorTerminateApp: HandlerFn<
  { udid?: string; bundleId?: string },
  { udid: string; bundleId: string }
> = async (params, ctx) => {
  const udid = requireString(params?.udid, "simulator.terminateApp", "udid");
  const bundleId = requireString(params?.bundleId, "simulator.terminateApp", "bundleId");
  const sim = await findSimulator(ctx, udid, { requireBooted: true });
  await simctl(["terminate", sim.udid, bundleId], ctx.workspacePath);
  return { udid: sim.udid, bundleId };
};

export const simulatorOpenUrl: HandlerFn<{ udid?: string; url?: string }, { udid: string; url: string }> = async (
  params,
  ctx,
) => {
  const udid = requireString(params?.udid, "simulator.openUrl", "udid");
  const url = requireString(params?.url, "simulator.openUrl", "url");
  const sim = await findSimulator(ctx, udid, { requireBooted: true });
  await simctl(["openurl", sim.udid, url], ctx.workspacePath);
  return { udid: sim.udid, url };
};

export const simulatorScreenshot: HandlerFn<{ udid?: string; path?: string }, { udid: string; path: string }> = async (
  params,
  ctx,
) => {
  const udid = requireString(params?.udid, "simulator.screenshot", "udid");
  const sim = await findSimulator(ctx, udid, { requireBooted: true });
  const target = params?.path && params.path.length > 0 ? params.path : `${ctx.workspacePath}/sweetpad-screenshot.png`;
  await simctl(["io", sim.udid, "screenshot", target], ctx.workspacePath);
  return { udid: sim.udid, path: target };
};
