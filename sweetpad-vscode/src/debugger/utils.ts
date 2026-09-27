import * as sweetpadLib from "@sweetpad/native";
import type * as vscode from "vscode";

import { type DevicectlAppProcess, getRunningProcessesJson } from "../common/xcode/devicectl";

/**
 * Wrap a value as a double-quoted LLDB command argument. LLDB's command interpreter splits
 * on whitespace unless quoted, so any path that contains a space — "iOS DeviceSupport" in
 * every symbols path, for one — is otherwise read as several arguments.
 */
export function quoteLldbArgument(value: string): string {
  return `"${value.replace(/(["\\])/g, "\\$1")}"`;
}

/**
 * Wrap a value as a single-quoted Python string literal, for the "script ..." commands that
 * reach LLDB's embedded interpreter.
 */
export function quotePythonString(value: string): string {
  return `'${value.replace(/\\/g, "\\\\").replace(/'/g, "\\'")}'`;
}

/**
 * Wait while the process is launched on the device and return the process information.
 * The app's own executable is the one directly inside its bundle ("Mastodon.app/Mastodon");
 * the bundle directory has to match whole, so "App.app" never matches "MyApp.app", and an
 * extension running out of "PlugIns/" is passed over.
 */
export async function waitForProcessToLaunch(
  vscodeContext: vscode.ExtensionContext,
  options: {
    deviceId: string;
    appName: string;
    timeoutMs: number;
  },
): Promise<DevicectlAppProcess> {
  const { appName, deviceId, timeoutMs } = options;

  const startTime = Date.now(); // in milliseconds

  // await pairDevice({ deviceId });

  while (true) {
    // Sometimes launching can go wrong, so we need to stop the waiting process
    // after some time and throw an error.
    const elapsedTime = Date.now() - startTime; // in milliseconds
    if (elapsedTime > timeoutMs) {
      throw new Error(`Timeout waiting for the process to launch: ${appName}`);
    }

    // Query the running processes on the device using the devicectl command
    const json = await getRunningProcessesJson(vscodeContext, { deviceId: deviceId });
    const process = sweetpadLib.devicectlAppProcesses(json, appName).find((p) => p.main);
    if (process) {
      return process;
    }

    // Wait for 1 second before checking again
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
}
