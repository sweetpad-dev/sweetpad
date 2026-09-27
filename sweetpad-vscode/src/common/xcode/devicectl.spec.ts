/**
 * The devicectl listing as the addon reads it, and the two narrowing accessors on top.
 */

import * as sweetpadLib from "@sweetpad/native";

import { createMockDevice } from "../../__mocks__/devices";
import { deviceTunnelState, deviceType } from "./devicectl";

function listing(devices: unknown[]): string {
  return JSON.stringify({ result: { devices } });
}

describe("parseDevicectlDevices", () => {
  it("reads a jsonVersion 4 device from the deprecated property bags", () => {
    const [device] = sweetpadLib.parseDevicectlDevices(
      listing([
        {
          identifier: "ID-1",
          connectionProperties: { tunnelState: "connected", lastConnectionDate: "2026-09-19T22:21:56.000Z" },
          deviceProperties: { name: "iPhone 14 Pro", osVersionNumber: "17.0" },
          hardwareProperties: {
            deviceType: "iPhone",
            marketingName: "iPhone 14 Pro",
            productType: "iPhone15,2",
            udid: "00008110-001234567890001E",
            platform: "iOS",
          },
        },
      ]),
    );

    expect(device).toMatchObject({
      identifier: "ID-1",
      udid: "00008110-001234567890001E",
      name: "iPhone 14 Pro",
      marketingName: "iPhone 14 Pro",
      productType: "iPhone15,2",
      deviceType: "iPhone",
      osVersion: "17.0",
      connection: "connected",
    });
    expect(new Date(device.lastConnectionMs ?? Number.NaN).toISOString()).toBe("2026-09-19T22:21:56.000Z");
  });

  it("reads a jsonVersion 5 device from the properties dictionary", () => {
    const [device] = sweetpadLib.parseDevicectlDevices(
      listing([
        {
          identifier: "ID-1",
          properties: {
            connection: { state: "connected", lastConnectionDate: 811_549_316 },
            hardware: { deviceType: "iPhone", udid: "00008110-001234567890001E" },
            software: { osVersionNumber: { stringValue: "17.0" } },
            state: { name: "iPhone 14 Pro" },
          },
        },
      ]),
    );

    expect(device).toMatchObject({ udid: "00008110-001234567890001E", osVersion: "17.0", connection: "connected" });
    // Core Foundation absolute time, counted from 2001-01-01.
    expect(new Date(device.lastConnectionMs ?? Number.NaN).toISOString()).toBe("2026-09-19T22:21:56.000Z");
  });

  it("leaves out what devicectl leaves out, and the hardware udid of an empty hardware section", () => {
    const [device] = sweetpadLib.parseDevicectlDevices(listing([{ identifier: "ID-1", hardwareProperties: {} }]));

    expect(device.identifier).toBe("ID-1");
    expect(device.udid).toBeUndefined();
    expect(device.name).toBeUndefined();
    expect(device.deviceType).toBeUndefined();
    expect(device.lastConnectionMs).toBeUndefined();
    expect(device.platform).toBe("iOS");
  });

  it("drops the simulators Xcode 27's devicectl lists beside the devices", () => {
    const devices = sweetpadLib.parseDevicectlDevices(
      listing([
        { identifier: "PHONE", properties: { hardware: { deviceType: "iPhone", reality: "physical", udid: "U1" } } },
        { identifier: "SIM", properties: { hardware: { deviceType: "iPhone", reality: "simulated", udid: "SIM" } } },
      ]),
    );

    expect(devices.map((d) => d.identifier)).toEqual(["PHONE"]);
  });
});

describe("narrowing accessors", () => {
  it("keep the device types and states a destination class covers", () => {
    expect(deviceType(createMockDevice())).toBe("iPhone");
    expect(deviceType(createMockDevice({ deviceType: "toaster" }))).toBeUndefined();
    expect(deviceType(createMockDevice({ deviceType: undefined }))).toBeUndefined();
    expect(deviceTunnelState(createMockDevice())).toBe("connected");
    expect(deviceTunnelState(createMockDevice({ connection: "napping" }))).toBeUndefined();
  });
});
