/**
 * How an app gets onto a device.
 *
 * - "devicectl" — CoreDevice, iOS 17 and up. Installs and launches, and the debugger finds
 *   the process afterwards through "devicectl device info processes".
 * - "ios-deploy" — iOS 16 and below. Installs and launches in one command.
 * - "ios-deploy-debugserver" — iOS 16 and below, debugging. Installs and leaves a debugserver
 *   listening; LLDB connects to it and launches the process itself. Distinct from the plain
 *   ios-deploy method because letting ios-deploy launch the app would put the process under
 *   its own LLDB, leaving nothing for the debug session to attach to.
 */
export type DeviceRunMethod = "devicectl" | "ios-deploy" | "ios-deploy-debugserver";

/**
 * Check if a device supports devicectl based on its OS version
 * @param osVersion The OS version string (e.g., "17.0", "17 beta", "16.5")
 * @param minVersion The minimum major version required for devicectl support
 * @returns true if the device supports devicectl, false otherwise
 */
export function supportsDevicectl(osVersion: string | undefined, minVersion: number): boolean {
  if (!osVersion || osVersion === "Unknown") {
    // If we don't know the OS version, assume it doesn't support devicectl
    // to be safe and fall back to ios-deploy
    return false;
  }
  // Extract leading digits using regex to handle beta versions like "17 beta" or "17.0 beta 3"
  const match = osVersion.match(/^(\d+)/);
  if (!match) {
    return false;
  }
  const majorVersion = Number.parseInt(match[1], 10);
  return !Number.isNaN(majorVersion) && majorVersion >= minVersion;
}

export function resolveDeviceRunMethod(options: { supportsDevicectl: boolean; debug: boolean }): DeviceRunMethod {
  if (options.supportsDevicectl) {
    return "devicectl";
  }
  return options.debug ? "ios-deploy-debugserver" : "ios-deploy";
}
