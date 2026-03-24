import { parseDeviceTypeIdentifier, parseSimulatorRuntime } from "./utils";

describe("parseDeviceTypeIdentifier", () => {
  it("parses iPhone identifier", () => {
    expect(parseDeviceTypeIdentifier("com.apple.CoreSimulator.SimDeviceType.iPhone-8-Plus")).toBe("iPhone");
  });

  it("parses iPhone SE identifier", () => {
    expect(parseDeviceTypeIdentifier("com.apple.CoreSimulator.SimDeviceType.iPhone-SE-3rd-generation")).toBe("iPhone");
  });

  it("parses iPad identifier", () => {
    expect(parseDeviceTypeIdentifier("com.apple.CoreSimulator.SimDeviceType.iPad-Pro-11-inch-3rd-generation")).toBe(
      "iPad",
    );
  });

  it("parses iPod identifier", () => {
    expect(parseDeviceTypeIdentifier("com.apple.CoreSimulator.SimDeviceType.iPod-touch--7th-generation-")).toBe("iPod");
  });

  it("parses Apple TV identifier", () => {
    expect(parseDeviceTypeIdentifier("com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-3rd-generation-4")).toBe(
      "AppleTV",
    );
  });

  it("parses Apple Watch identifier", () => {
    expect(parseDeviceTypeIdentifier("com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-5-40mm")).toBe(
      "AppleWatch",
    );
  });

  it("parses Apple Vision identifier", () => {
    expect(parseDeviceTypeIdentifier("com.apple.CoreSimulator.SimDeviceType.Apple-Vision-Pro")).toBe("AppleVision");
  });

  it("returns null for invalid prefix", () => {
    expect(parseDeviceTypeIdentifier("invalid.prefix.iPhone")).toBeNull();
  });

  it("returns null for empty string", () => {
    expect(parseDeviceTypeIdentifier("")).toBeNull();
  });

  it("returns null for prefix only", () => {
    expect(parseDeviceTypeIdentifier("com.apple.CoreSimulator.SimDeviceType.")).toBeNull();
  });

  it("returns null for unknown device type", () => {
    expect(parseDeviceTypeIdentifier("com.apple.CoreSimulator.SimDeviceType.Unknown-Device")).toBeNull();
  });

  it("returns null for null-ish input", () => {
    expect(parseDeviceTypeIdentifier(undefined as any)).toBeNull();
    expect(parseDeviceTypeIdentifier(null as any)).toBeNull();
  });
});

describe("parseSimulatorRuntime", () => {
  it("parses iOS runtime", () => {
    expect(parseSimulatorRuntime("com.apple.CoreSimulator.SimRuntime.iOS-15-2")).toEqual({
      os: "iOS",
      version: "15.2",
    });
  });

  it("parses iOS 18.0 runtime", () => {
    expect(parseSimulatorRuntime("com.apple.CoreSimulator.SimRuntime.iOS-18-0")).toEqual({
      os: "iOS",
      version: "18.0",
    });
  });

  it("parses tvOS runtime", () => {
    expect(parseSimulatorRuntime("com.apple.CoreSimulator.SimRuntime.tvOS-18-0")).toEqual({
      os: "tvOS",
      version: "18.0",
    });
  });

  it("parses watchOS runtime", () => {
    expect(parseSimulatorRuntime("com.apple.CoreSimulator.SimRuntime.watchOS-8-5")).toEqual({
      os: "watchOS",
      version: "8.5",
    });
  });

  it("parses xrOS runtime", () => {
    expect(parseSimulatorRuntime("com.apple.CoreSimulator.SimRuntime.xrOS-2-0")).toEqual({
      os: "xrOS",
      version: "2.0",
    });
  });

  it("returns null for invalid prefix", () => {
    expect(parseSimulatorRuntime("invalid.prefix.iOS-15-2")).toBeNull();
  });

  it("returns null for empty string", () => {
    expect(parseSimulatorRuntime("")).toBeNull();
  });

  it("returns null for prefix only", () => {
    expect(parseSimulatorRuntime("com.apple.CoreSimulator.SimRuntime.")).toBeNull();
  });

  it("returns null for malformed version", () => {
    expect(parseSimulatorRuntime("com.apple.CoreSimulator.SimRuntime.iOS-15")).toBeNull();
  });

  it("returns null for unknown OS", () => {
    expect(parseSimulatorRuntime("com.apple.CoreSimulator.SimRuntime.androidOS-15-2")).toBeNull();
  });

  it("returns null for null-ish input", () => {
    expect(parseSimulatorRuntime(undefined as any)).toBeNull();
    expect(parseSimulatorRuntime(null as any)).toBeNull();
  });
});
