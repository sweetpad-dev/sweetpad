import type { ExtensionContext } from "../common/commands";
import { type DeviceCtlProcess, getRunningProcesses } from "../common/xcode/devicectl";

/**
 * Wait while the process is launched on the device and return the process information.
 */
export async function waitForProcessToLaunch(
  context: ExtensionContext,
  options: {
    deviceId: string;
    appName: string;
    timeoutMs: number;
  },
): Promise<DeviceCtlProcess> {
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
    const result = await getRunningProcesses(context, { deviceId: deviceId });
    const runningProcesses = result?.result?.runningProcesses ?? [];
    if (runningProcesses.length === 0) {
      throw new Error("No running processes found on the device");
    }

    // Example of a running process:
    // {
    //   "executable" : "file:///private/var/containers/Bundle/Application/5045C7CE-DFB9-4C17-BBA9-94D8BCD8F565/Mastodon.app/Mastodon",
    //   "processIdentifier" : 19350
    // },
    // Example of appName: "Mastodon.app"
    const process = runningProcesses.find((p) => p.executable?.includes(appName));
    if (process) {
      return process;
    }

    // Wait for 1 second before checking again
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
}
