import { iOSSimulatorDestination, tvOSSimulatorDestination } from "../simulators/types";
import { splitSupportedDestinatinos } from "./utils";
import { macOSDestination } from "./types";

function createiOSSim(udid: string) {
  return new iOSSimulatorDestination({
    udid,
    isAvailable: true,
    state: "Shutdown",
    name: "iPhone 14",
    simulatorType: "iPhone",
    os: "iOS",
    osVersion: "17.0",
    rawDeviceTypeIdentifier: "com.apple.CoreSimulator.SimDeviceType.iPhone-14",
    rawRuntime: "com.apple.CoreSimulator.SimRuntime.iOS-17-0",
  });
}

function createTVSim(udid: string) {
  return new tvOSSimulatorDestination({
    udid,
    isAvailable: true,
    state: "Shutdown",
    name: "Apple TV",
    os: "tvOS",
    osVersion: "17.0",
    rawDeviceTypeIdentifier: "com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K",
    rawRuntime: "com.apple.CoreSimulator.SimRuntime.tvOS-17-0",
  });
}

describe("splitSupportedDestinatinos", () => {
  const iosSim = createiOSSim("aaa");
  const tvSim = createTVSim("bbb");
  const mac = new macOSDestination({ name: "My Mac", arch: "arm64" });

  it("returns all destinations as supported when supportedPlatforms is undefined", () => {
    const result = splitSupportedDestinatinos({
      destinations: [iosSim, tvSim, mac],
      supportedPlatforms: undefined,
    });
    expect(result.supported).toHaveLength(3);
    expect(result.unsupported).toHaveLength(0);
  });

  it("splits destinations by platform", () => {
    const result = splitSupportedDestinatinos({
      destinations: [iosSim, tvSim, mac],
      supportedPlatforms: ["iphonesimulator"],
    });
    expect(result.supported).toEqual([iosSim]);
    expect(result.unsupported).toEqual([tvSim, mac]);
  });

  it("supports multiple platforms", () => {
    const result = splitSupportedDestinatinos({
      destinations: [iosSim, tvSim, mac],
      supportedPlatforms: ["iphonesimulator", "macosx"],
    });
    expect(result.supported).toEqual([iosSim, mac]);
    expect(result.unsupported).toEqual([tvSim]);
  });

  it("handles empty destinations", () => {
    const result = splitSupportedDestinatinos({
      destinations: [],
      supportedPlatforms: ["iphonesimulator"],
    });
    expect(result.supported).toHaveLength(0);
    expect(result.unsupported).toHaveLength(0);
  });

  it("handles no matching platforms", () => {
    const result = splitSupportedDestinatinos({
      destinations: [iosSim],
      supportedPlatforms: ["macosx"],
    });
    expect(result.supported).toHaveLength(0);
    expect(result.unsupported).toEqual([iosSim]);
  });
});
