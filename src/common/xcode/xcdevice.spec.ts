/**
 * Unit tests for listDevicesWithXcdevice.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { createMockContext } from "../../__mocks__/devices";
import { exec } from "../exec";
import { listDevicesWithXcdevice } from "./xcdevice";

jest.mock("../exec", () => ({
  exec: jest.fn(),
}));

jest.mock("../logger", () => ({
  commonLogger: {
    debug: jest.fn(),
    error: jest.fn(),
  },
}));

const fixturesDir = path.join(__dirname, "../../../tests/xcdevice-data");

function loadFixture(name: string): string {
  return fs.readFileSync(path.join(fixturesDir, name), "utf8");
}

describe("listDevicesWithXcdevice", () => {
  const mockContext = createMockContext();

  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("returns iOS devices from xcdevice list output", async () => {
    (exec as jest.Mock).mockResolvedValue(loadFixture("xcdevice-ios-devices.json"));

    const devices = await listDevicesWithXcdevice(mockContext);

    expect(devices).toHaveLength(3);
    expect(devices[0].name).toBe("iPhone 14 Pro");
    expect(devices[0].modelCode).toBe("iPhone15,2");
  });

  it("keeps devices from iphoneos, watchos, appletvos and xros platforms; drops simulators", async () => {
    (exec as jest.Mock).mockResolvedValue(loadFixture("xcdevice-mixed-platforms.json"));

    const devices = await listDevicesWithXcdevice(mockContext);

    // Fixture has 4 real devices + 1 simulator; simulator must be filtered out.
    expect(devices).toHaveLength(4);
    const platforms = devices.map((d) => d.platform).sort();
    expect(platforms).toEqual([
      "com.apple.platform.appletvos",
      "com.apple.platform.iphoneos",
      "com.apple.platform.iphoneos",
      "com.apple.platform.watchos",
    ]);
  });

  it("retains all supported physical-device platforms in mixed input", async () => {
    (exec as jest.Mock).mockResolvedValue(loadFixture("xcdevice-mixed-devices.json"));

    const devices = await listDevicesWithXcdevice(mockContext);

    // Fixture has iphoneos × 2, watchos, appletvos, xros — all physical, all retained.
    expect(devices).toHaveLength(5);
    const platformSet = new Set(devices.map((d) => d.platform));
    expect(platformSet).toEqual(
      new Set([
        "com.apple.platform.iphoneos",
        "com.apple.platform.watchos",
        "com.apple.platform.appletvos",
        "com.apple.platform.xros",
      ]),
    );
  });

  it("retains entries with available:false / error so callers can render them as unavailable", async () => {
    (exec as jest.Mock).mockResolvedValue(loadFixture("xcdevice-unavailable.json"));

    const devices = await listDevicesWithXcdevice(mockContext);

    expect(devices).toHaveLength(1);
    expect(devices[0].available).toBe(false);
    expect(devices[0].error?.code).toBe(-9);
  });

  it("returns empty array when no devices found", async () => {
    (exec as jest.Mock).mockResolvedValue(loadFixture("xcdevice-empty.json"));

    const devices = await listDevicesWithXcdevice(mockContext);

    expect(devices).toEqual([]);
  });

  it("returns empty array and logs error for malformed JSON", async () => {
    (exec as jest.Mock).mockResolvedValue(loadFixture("xcdevice-malformed.json"));

    const devices = await listDevicesWithXcdevice(mockContext);

    expect(devices).toEqual([]);
  });

  it("returns empty array when exec throws", async () => {
    (exec as jest.Mock).mockRejectedValue(new Error("Command failed"));

    const devices = await listDevicesWithXcdevice(mockContext);

    expect(devices).toEqual([]);
  });

  it("returns empty array when xcdevice command not found (ENOENT)", async () => {
    const error: any = new Error("Command not found");
    error.code = "ENOENT";
    (exec as jest.Mock).mockRejectedValue(error);

    const devices = await listDevicesWithXcdevice(mockContext);

    expect(devices).toEqual([]);
  });
});
