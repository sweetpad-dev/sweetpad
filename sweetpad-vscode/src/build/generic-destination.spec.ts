import {
  GENERIC_DESTINATIONS,
  GenericDestination,
  macOSDestination,
  normalizeDestinationId,
} from "../destination/types";
import {
  assertDestinationSupportsAction,
  assertRunnableDestination,
  filterDestinationsForAction,
  findDestinationForTaskInput,
} from "../destination/utils";
import { getXcodeBuildDestinationString } from "./utils";

describe("generic build-only destinations", () => {
  it("serialize to xcodebuild's device-less `generic/platform=…`", () => {
    const anyIOS = new GenericDestination({ name: "Any iOS Device", platform: "iphoneos", platformArg: "iOS" });
    const anyIOSSim = new GenericDestination({
      name: "Any iOS Simulator Device",
      platform: "iphonesimulator",
      platformArg: "iOS Simulator",
    });

    expect(getXcodeBuildDestinationString({ destination: anyIOS })).toBe("generic/platform=iOS");
    expect(getXcodeBuildDestinationString({ destination: anyIOSSim })).toBe("generic/platform=iOS Simulator");
    // No id/arch is appended — that's what makes it device-less/build-only.
    expect(getXcodeBuildDestinationString({ destination: anyIOS })).not.toContain("id=");
  });

  it("build a stable, prefix-matched id from the platform arg", () => {
    const anyIOSSim = new GenericDestination({
      name: "Any iOS Simulator Device",
      platform: "iphonesimulator",
      platformArg: "iOS Simulator",
    });
    expect(anyIOSSim.id).toBe("generic-ios-simulator");
    expect(anyIOSSim.type).toBe("generic");
  });

  it("every catalog entry round-trips its id through the -destination string", () => {
    for (const destination of GENERIC_DESTINATIONS) {
      expect(getXcodeBuildDestinationString({ destination })).toBe(`generic/platform=${destination.platformArg}`);
      expect(destination.id.startsWith("generic-")).toBe(true);
    }
    // The two the issue explicitly asked for are present.
    const names = GENERIC_DESTINATIONS.map((d) => d.name);
    expect(names).toContain("Any iOS Device");
    expect(names).toContain("Any iOS Simulator Device");
  });
});

describe("build-only destinations and what an action can use", () => {
  const anyIOS = new GenericDestination({ name: "Any iOS Device", platform: "iphoneos", platformArg: "iOS" });
  const myMac = new macOSDestination({ name: "My Mac", arch: "arm64" });

  it.each(["build", "clean"] as const)("offers them to %s, which needs no device", (action) => {
    expect(filterDestinationsForAction([myMac, anyIOS], action)).toEqual([myMac, anyIOS]);
  });

  it.each(["run", "launch", "test"] as const)("keeps them out of the %s picker", (action) => {
    expect(filterDestinationsForAction([myMac, anyIOS], action)).toEqual([myMac]);
  });

  it("refuses one that reaches a run, launch or test path anyway", () => {
    // A pinned or task-named destination skips the picker, so the filter alone isn't enough.
    expect(() => assertRunnableDestination(anyIOS, "test")).toThrow(
      '"Any iOS Device" is a build-only destination. Pick a simulator or device to test.',
    );
    expect(() => assertRunnableDestination(myMac, "test")).not.toThrow();
  });

  it.each(["build", "clean"] as const)("lets one through on %s", (action) => {
    expect(() => assertDestinationSupportsAction(anyIOS, action)).not.toThrow();
  });
});

describe("resolving a hand-written generic id", () => {
  it.each(["iOS Simulator", "ios-simulator", "IOS SIMULATOR", "generic-ios-simulator"])(
    "folds %s onto the catalog id",
    (written) => {
      expect(normalizeDestinationId(written, "generic")).toBe("generic-ios-simulator");
    },
  );
});

describe("resolving the destination a task named", () => {
  const anyIOS = new GenericDestination({ name: "Any iOS Device", platform: "iphoneos", platformArg: "iOS" });
  const anyMac = new GenericDestination({ name: "Any Mac", platform: "macosx", platformArg: "macOS" });
  const myMac = new macOSDestination({ name: "My Mac", arch: "arm64" });
  // My Mac is scanned before the generics, so it gets first refusal on every string.
  const destinations = [myMac, anyIOS, anyMac];

  it.each(["generic/platform=macOS", "generic/platform=macos", " generic/platform=macOS "])(
    "reads %s as Any Mac, not this Mac",
    (destination) => {
      expect(findDestinationForTaskInput(destinations, { destination })).toBe(anyMac);
    },
  );

  it("still reads a plain macOS destination as this Mac", () => {
    expect(findDestinationForTaskInput(destinations, { destination: "platform=macOS,arch=arm64" })).toBe(myMac);
  });

  it("reads a device-less string for any other platform too", () => {
    expect(findDestinationForTaskInput(destinations, { destination: "generic/platform=iOS" })).toBe(anyIOS);
  });

  it("still accepts a generic destination named by its id", () => {
    expect(findDestinationForTaskInput(destinations, { destinationId: "generic-ios" })).toBe(anyIOS);
  });

  it("matches nothing when the platform isn't one we offer", () => {
    expect(findDestinationForTaskInput(destinations, { destination: "generic/platform=DriverKit" })).toBeUndefined();
  });
});
