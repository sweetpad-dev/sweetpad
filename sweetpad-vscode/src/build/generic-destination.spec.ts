import { GENERIC_DESTINATIONS, GenericDestination } from "../destination/types";
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
    expect(anyIOSSim.type).toBe("Generic");
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
