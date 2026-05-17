import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DestinationsManager } from "../../core/destination/manager";
import type { Destination } from "../../core/destination/types";
import { createDestinationsListMethod } from "./destinations-list";

const SIM: Destination = {
  id: "ios-sim-A",
  label: "iPhone 15",
  type: "iOSSimulator",
  platform: "iphonesimulator",
} as unknown as Destination;

const DEVICE: Destination = {
  id: "ios-dev-A",
  label: "Bob's iPhone",
  type: "iOSDevice",
  platform: "iphoneos",
} as unknown as Destination;

const MAC: Destination = {
  id: "macos-myMac",
  label: "My Mac",
  type: "macOS",
  platform: "macosx",
} as unknown as Destination;

function makeHarness(options?: { destinations?: Destination[] }) {
  const destinationsManager = {
    getDestinations: vi.fn().mockResolvedValue(options?.destinations ?? [SIM, DEVICE, MAC]),
    refresh: vi.fn().mockResolvedValue(undefined),
  } as unknown as DestinationsManager;

  const method = createDestinationsListMethod({ destinationsManager });
  return { method, destinationsManager };
}

describe("destinations.list method", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns every destination mapped to wire shape", async () => {
    const h = makeHarness();
    const result = await h.method({});

    expect(result.destinations).toHaveLength(3);
    expect(result.destinations[0]).toEqual({
      id: "ios-sim-A",
      kind: "iOSSimulator",
      label: "iPhone 15",
      platform: "iphonesimulator",
    });
  });

  it("requests most-used sorting from the destinations manager", async () => {
    const h = makeHarness();
    await h.method({});
    expect(h.destinationsManager.getDestinations).toHaveBeenCalledWith({ mostUsedSort: true });
  });

  it("filters server-side when kind is provided", async () => {
    const h = makeHarness();
    const result = await h.method({ kind: "iOSDevice" });

    expect(result.destinations).toEqual([
      { id: "ios-dev-A", kind: "iOSDevice", label: "Bob's iPhone", platform: "iphoneos" },
    ]);
  });

  it("returns an empty array when no destination matches the kind", async () => {
    const h = makeHarness({ destinations: [SIM] });
    const result = await h.method({ kind: "macOS" });
    expect(result.destinations).toEqual([]);
  });

  it("calls refresh() when --refresh is set", async () => {
    const h = makeHarness();
    await h.method({ refresh: true });
    expect(h.destinationsManager.refresh).toHaveBeenCalledOnce();
  });

  it("does not call refresh() by default", async () => {
    const h = makeHarness();
    await h.method({});
    expect(h.destinationsManager.refresh).not.toHaveBeenCalled();
  });

  it("rejects invalid kind values with INVALID_ARGUMENT", async () => {
    const h = makeHarness();
    await expect(h.method({ kind: "carPlay" })).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });

  it("rejects non-boolean refresh with INVALID_ARGUMENT", async () => {
    const h = makeHarness();
    await expect(h.method({ refresh: "yes" })).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });
});
