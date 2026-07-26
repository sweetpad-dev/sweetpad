/**
 * "sweetpad.build.destination" stores identity only, so getSelectedXcodeDestinationForBuild
 * takes the display name from the already-scanned destinations. These tests cover what the
 * status bar reads before and after a scan has landed.
 */
import { workspace as vscodeWorkspace } from "../__mocks__/vscode";
import type { WorkspaceStateService } from "../common/workspace-state";
import type { DevicesManager } from "../devices/manager";
import type { SimulatorsManager } from "../simulators/manager";
import { iOSSimulatorDestination } from "../simulators/types";
import { DestinationsManager } from "./manager";

const UDID = "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE";
const FULL_ID = `iossimulator-${UDID}`;

function simulator(name: string, udid = UDID): iOSSimulatorDestination {
  return new iOSSimulatorDestination({
    udid: udid,
    isAvailable: true,
    state: "Shutdown",
    name: name,
    simulatorType: "iPhone",
    os: "iOS",
    osVersion: "17.0",
    rawDeviceTypeIdentifier: "com.apple.CoreSimulator.SimDeviceType.iPhone-16",
    rawRuntime: "com.apple.CoreSimulator.SimRuntime.iOS-17-0",
  });
}

function pinConfig(value: unknown): void {
  vi.mocked(vscodeWorkspace.getConfiguration).mockReturnValue({
    get: (key: string) => (key === "build.destination" ? value : undefined),
  } as any);
}

/**
 * `scanned` stands in for what simctl has already returned; empty means no scan yet.
 * Mutate the returned array to simulate a later scan, then fire `simulatorsUpdated`.
 */
function buildManager(scanned: iOSSimulatorDestination[]): DestinationsManager & { simulateScan: () => void } {
  const simulatorListeners: (() => void)[] = [];
  const manager = new DestinationsManager({
    simulatorsManager: {
      on: vi.fn((_event: string, listener: () => void) => simulatorListeners.push(listener)),
      getCachedSimulators: () => scanned,
    } as unknown as SimulatorsManager,
    devicesManager: {
      on: vi.fn(),
      getCachedDevices: () => [],
    } as unknown as DevicesManager,
    workspaceState: {
      get: vi.fn().mockReturnValue(undefined),
      update: vi.fn(),
    } as unknown as WorkspaceStateService,
  });

  void manager.start();
  return Object.assign(manager, {
    simulateScan: () => {
      for (const listener of simulatorListeners) {
        listener();
      }
    },
  });
}

describe("pinned destination label", () => {
  afterEach(() => {
    vi.mocked(vscodeWorkspace.getConfiguration).mockReset();
  });

  it("expands a bare udid into the full id", () => {
    pinConfig({ id: UDID, type: "iOSSimulator" });

    expect(buildManager([]).getSelectedXcodeDestinationForBuild()).toEqual({
      id: FULL_ID,
      type: "iOSSimulator",
      name: UDID,
    });
  });

  it("shows what was written before any scan has landed", () => {
    pinConfig({ id: UDID, type: "iOSSimulator" });

    expect(buildManager([]).getSelectedXcodeDestinationForBuild()?.name).toBe(UDID);
  });

  it("takes the name from a scanned destination", () => {
    pinConfig({ id: UDID, type: "iOSSimulator" });

    expect(buildManager([simulator("iPhone 16")]).getSelectedXcodeDestinationForBuild()?.name).toBe("iPhone 16");
  });

  it("resolves a fully qualified pin too", () => {
    pinConfig({ id: FULL_ID, type: "iOSSimulator" });

    expect(buildManager([simulator("iPhone 16")]).getSelectedXcodeDestinationForBuild()?.name).toBe("iPhone 16");
  });

  it("follows a rename with no state to invalidate", () => {
    pinConfig({ id: UDID, type: "iOSSimulator" });

    expect(buildManager([simulator("Work Phone")]).getSelectedXcodeDestinationForBuild()?.name).toBe("Work Phone");
  });

  it("picks up a scan that lands after the name was already looked up", () => {
    pinConfig({ id: UDID, type: "iOSSimulator" });
    const scanned: iOSSimulatorDestination[] = [];
    const manager = buildManager(scanned);

    // Populate the lookup while nothing has been scanned yet.
    expect(manager.getSelectedXcodeDestinationForBuild()?.name).toBe(UDID);

    scanned.push(simulator("iPhone 16"));
    manager.simulateScan();

    expect(manager.getSelectedXcodeDestinationForBuild()?.name).toBe("iPhone 16");
  });

  it("drops a name that a later scan removed", () => {
    pinConfig({ id: UDID, type: "iOSSimulator" });
    const scanned = [simulator("iPhone 16")];
    const manager = buildManager(scanned);
    expect(manager.getSelectedXcodeDestinationForBuild()?.name).toBe("iPhone 16");

    scanned.length = 0;
    manager.simulateScan();

    expect(manager.getSelectedXcodeDestinationForBuild()?.name).toBe(UDID);
  });

  it("keeps showing the id when the pinned destination is absent", () => {
    pinConfig({ id: UDID, type: "iOSSimulator" });

    expect(buildManager([simulator("iPad Pro", "other-udid")]).getSelectedXcodeDestinationForBuild()?.name).toBe(UDID);
  });

  it("resolves macOS without a scan", () => {
    pinConfig({ id: "My Mac", type: "macOS" });

    expect(buildManager([]).getSelectedXcodeDestinationForBuild()).toEqual({
      id: "macos-My Mac",
      type: "macOS",
      name: "My Mac",
    });
  });

  it("falls back to workspace state when nothing is pinned", () => {
    pinConfig(undefined);

    expect(buildManager([]).getSelectedXcodeDestinationForBuild()).toBeUndefined();
  });
});
