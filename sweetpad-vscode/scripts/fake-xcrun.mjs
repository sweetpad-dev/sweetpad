#!/usr/bin/env node
/**
 * Stand-in for xcrun that reports one fake iOS 15.6.1 device, so the ios-deploy debugserver
 * method can be exercised without hardware old enough to need it. Every other subcommand is
 * forwarded to the real xcrun, so builds and simulators keep working.
 *
 * Pair it with scripts/fake-ios-deploy.mjs — see that file for the full walkthrough.
 *
 *   SWEETPAD_FAKE_DEVICE_OS   OS version to report (default 15.6.1)
 *   SWEETPAD_FAKE_DEVICE_UDID UDID to report (default 00008030-000FAKE0FAKE001E)
 */

import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";

const args = process.argv.slice(2);

const osVersion = process.env.SWEETPAD_FAKE_DEVICE_OS ?? "15.6.1";
const udid = process.env.SWEETPAD_FAKE_DEVICE_UDID ?? "00008030-000FAKE0FAKE001E";
const name = "Fake iPad mini 4";

function passThrough() {
  const child = spawn("/usr/bin/xcrun", args, { stdio: "inherit" });
  child.on("exit", (code, signal) => process.exit(signal ? 1 : (code ?? 0)));
  child.on("error", () => process.exit(127));
}

// "xcrun xcdevice list" — stdout JSON. The extension merges this with devicectl's view;
// a device this old is normally seen only here.
if (args[0] === "xcdevice" && args[1] === "list") {
  process.stdout.write(
    `${JSON.stringify(
      [
        {
          simulator: false,
          operatingSystemVersion: `${osVersion} (19G82)`,
          interface: "usb",
          available: true,
          platform: "com.apple.platform.iphoneos",
          modelCode: "iPad5,1",
          identifier: udid,
          architecture: "arm64",
          modelName: "iPad mini 4",
          name,
        },
      ],
      null,
      2,
    )}\n`,
  );
  process.exit(0);
}

// "xcrun devicectl list devices --json-output <path> --timeout 10" — writes JSON to a file.
// Reporting no devices is what a real CoreDevice-less setup looks like, and it leaves the
// xcdevice entry above as the only source, which is the case worth exercising.
if (args[0] === "devicectl" && args[1] === "list" && args[2] === "devices") {
  const outputIndex = args.indexOf("--json-output");
  if (outputIndex !== -1 && args[outputIndex + 1]) {
    writeFileSync(args[outputIndex + 1], JSON.stringify({ result: { devices: [] } }, null, 2));
  }
  process.exit(0);
}

// "xcrun devicectl device info processes" — the call that fails on a real iOS 16 device and
// started all this. Fail loudly: nothing on the debugserver method may reach it.
if (args[0] === "devicectl" && args[1] === "device" && args[2] === "info" && args[3] === "processes") {
  process.stderr.write("fake-xcrun: devicectl is unavailable on this device (as on real iOS <= 16)\n");
  process.exit(1);
}

passThrough();
