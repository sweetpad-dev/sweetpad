import {
  iOSSimulatorDestination,
  watchOSSimulatorDestination,
  tvOSSimulatorDestination,
  visionOSSimulatorDestination,
} from "./types";

function createiOSSimulator(overrides?: Partial<ConstructorParameters<typeof iOSSimulatorDestination>[0]>) {
  return new iOSSimulatorDestination({
    udid: "10D6D4A3-3A3D-4D3D-8D3D-3D3D3D3D3D3D",
    isAvailable: true,
    state: "Shutdown",
    name: "iPhone 14 Pro",
    simulatorType: "iPhone",
    os: "iOS",
    osVersion: "17.0",
    rawDeviceTypeIdentifier: "com.apple.CoreSimulator.SimDeviceType.iPhone-14-Pro",
    rawRuntime: "com.apple.CoreSimulator.SimRuntime.iOS-17-0",
    ...overrides,
  });
}

describe("iOSSimulatorDestination", () => {
  it("has correct type and platform", () => {
    const sim = createiOSSimulator();
    expect(sim.type).toBe("iOSSimulator");
    expect(sim.typeLabel).toBe("iOS Simulator");
    expect(sim.platform).toBe("iphonesimulator");
  });

  it("generates correct id", () => {
    const sim = createiOSSimulator();
    expect(sim.id).toBe("iossimulator-10D6D4A3-3A3D-4D3D-8D3D-3D3D3D3D3D3D");
  });

  it("generates correct label", () => {
    const sim = createiOSSimulator();
    expect(sim.label).toBe("iPhone 14 Pro (17.0)");
  });

  it("reports isBooted correctly", () => {
    expect(createiOSSimulator({ state: "Booted" }).isBooted).toBe(true);
    expect(createiOSSimulator({ state: "Shutdown" }).isBooted).toBe(false);
  });

  it("returns correct icon based on state", () => {
    expect(createiOSSimulator({ state: "Booted" }).icon).toBe("sweetpad-device-mobile");
    expect(createiOSSimulator({ state: "Shutdown" }).icon).toBe("sweetpad-device-mobile-pause");
  });

  it("generates quickPickDetails", () => {
    const sim = createiOSSimulator();
    expect(sim.quickPickDetails).toBe(
      "Type: iOS Simulator, Version: 17.0, ID: 10d6d4a3-3a3d-4d3d-8d3d-3d3d3d3d3d3d",
    );
  });
});

describe("watchOSSimulatorDestination", () => {
  function createWatchSim(overrides?: Partial<ConstructorParameters<typeof watchOSSimulatorDestination>[0]>) {
    return new watchOSSimulatorDestination({
      udid: "AAAA-BBBB",
      isAvailable: true,
      state: "Shutdown",
      name: "Apple Watch Series 9 - 45mm",
      os: "watchOS",
      osVersion: "10.2",
      rawDeviceTypeIdentifier: "com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-9-45mm",
      rawRuntime: "com.apple.CoreSimulator.SimRuntime.watchOS-10-2",
      ...overrides,
    });
  }

  it("has correct type and platform", () => {
    const sim = createWatchSim();
    expect(sim.type).toBe("watchOSSimulator");
    expect(sim.typeLabel).toBe("watchOS");
    expect(sim.platform).toBe("watchsimulator");
  });

  it("generates correct id", () => {
    expect(createWatchSim().id).toBe("watchossimulator-AAAA-BBBB");
  });

  it("returns correct icon based on state", () => {
    expect(createWatchSim({ state: "Booted" }).icon).toBe("sweetpad-device-watch");
    expect(createWatchSim({ state: "Shutdown" }).icon).toBe("sweetpad-device-watch-pause");
  });
});

describe("tvOSSimulatorDestination", () => {
  function createTVSim(overrides?: Partial<ConstructorParameters<typeof tvOSSimulatorDestination>[0]>) {
    return new tvOSSimulatorDestination({
      udid: "CCCC-DDDD",
      isAvailable: true,
      state: "Shutdown",
      name: "Apple TV 4K",
      os: "tvOS",
      osVersion: "17.0",
      rawDeviceTypeIdentifier: "com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-3rd-gen",
      rawRuntime: "com.apple.CoreSimulator.SimRuntime.tvOS-17-0",
      ...overrides,
    });
  }

  it("has correct type and platform", () => {
    const sim = createTVSim();
    expect(sim.type).toBe("tvOSSimulator");
    expect(sim.typeLabel).toBe("tvOS");
    expect(sim.platform).toBe("appletvsimulator");
  });

  it("generates correct id", () => {
    expect(createTVSim().id).toBe("tvossimulator-CCCC-DDDD");
  });

  it("always returns tv icon regardless of state", () => {
    expect(createTVSim({ state: "Booted" }).icon).toBe("sweetpad-device-tv-old");
    expect(createTVSim({ state: "Shutdown" }).icon).toBe("sweetpad-device-tv-old");
  });
});

describe("visionOSSimulatorDestination", () => {
  function createVisionSim(overrides?: Partial<ConstructorParameters<typeof visionOSSimulatorDestination>[0]>) {
    return new visionOSSimulatorDestination({
      udid: "EEEE-FFFF",
      isAvailable: true,
      state: "Shutdown",
      name: "Apple Vision Pro",
      os: "xrOS",
      osVersion: "2.0",
      rawDeviceTypeIdentifier: "com.apple.CoreSimulator.SimDeviceType.Apple-Vision-Pro",
      rawRuntime: "com.apple.CoreSimulator.SimRuntime.xrOS-2-0",
      ...overrides,
    });
  }

  it("has correct type and platform", () => {
    const sim = createVisionSim();
    expect(sim.type).toBe("visionOSSimulator");
    expect(sim.typeLabel).toBe("Apple Vision");
    expect(sim.platform).toBe("xrsimulator");
  });

  it("generates correct id", () => {
    expect(createVisionSim().id).toBe("visionsimulator-EEEE-FFFF");
  });

  it("always returns cardboards icon", () => {
    expect(createVisionSim({ state: "Booted" }).icon).toBe("sweetpad-cardboards");
    expect(createVisionSim({ state: "Shutdown" }).icon).toBe("sweetpad-cardboards");
  });
});
