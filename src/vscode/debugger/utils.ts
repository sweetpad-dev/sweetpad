import type { Logger } from "../../core/logger/types";
import type { WorkspaceRoot } from "../../core/workspace-root";
import { type DeviceCtlProcess, getRunningProcesses } from "../../core/xcode/devicectl";

/**
 * Wait while the process is launched on the device and return the process information.
 */
export async function waitForProcessToLaunch(options: {
  deviceId: string;
  appName: string;
  timeoutMs: number;
  workspaceRoot: WorkspaceRoot;
  logger: Logger;
}): Promise<DeviceCtlProcess> {
  const { appName, deviceId, timeoutMs, workspaceRoot, logger } = options;

  const startTime = Date.now();
  const storagePath = await workspaceRoot.getStoragePath();
  const cwd = workspaceRoot.getPath();

  while (true) {
    const elapsedTime = Date.now() - startTime;
    if (elapsedTime > timeoutMs) {
      throw new Error(`Timeout waiting for the process to launch: ${appName}`);
    }

    const result = await getRunningProcesses({
      storagePath,
      cwd,
      deviceId,
      logger,
    });
    const runningProcesses = result?.result?.runningProcesses ?? [];
    if (runningProcesses.length === 0) {
      throw new Error("No running processes found on the device");
    }

    // Example of a running process:
    // {
    //   "executable" : "file:///private/var/containers/Bundle/Application/5045C7CE-DFB9-4C17-BBA9-94D8BCD8F565/Mastodon.app/Mastodon",
    //   "processIdentifier" : 1234
    // }
    const process = runningProcesses.find((p) => p?.executable?.endsWith(`/${appName}/${appName}`));
    if (process) {
      return process;
    }

    await new Promise((resolve) => setTimeout(resolve, 500));
  }
}
