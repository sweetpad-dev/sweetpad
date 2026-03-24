import {
  createOsVersionLookup,
  getOsVersionForDevice,
  createUdidLookup,
  getUdidForDevice,
  createNameLookup,
  getNameForDevice,
  type XcdeviceDevice,
} from "./xcdevice";

const sampleDevices: XcdeviceDevice[] = [
  {
    identifier: "00008110-001234567890001E",
    modelCode: "iPhone15,2",
    name: "Alice's iPhone",
    operatingSystemVersion: "17.0",
    platform: "com.apple.platform.iphoneos",
  },
  {
    identifier: "00008120-ABCDEF012345",
    modelCode: "iPhone14,5",
    name: "Bob's iPhone",
    operatingSystemVersion: "16.7",
    platform: "com.apple.platform.iphoneos",
  },
];

describe("createOsVersionLookup", () => {
  it("creates lookup map from modelCode to OS version", () => {
    const lookup = createOsVersionLookup(sampleDevices);
    expect(lookup.get("iPhone15,2")).toBe("17.0");
    expect(lookup.get("iPhone14,5")).toBe("16.7");
  });

  it("returns empty map for empty array", () => {
    expect(createOsVersionLookup([]).size).toBe(0);
  });

  it("keeps first device for duplicate modelCodes", () => {
    const devices: XcdeviceDevice[] = [
      { ...sampleDevices[0], operatingSystemVersion: "17.0" },
      { ...sampleDevices[0], operatingSystemVersion: "17.1" },
    ];
    const lookup = createOsVersionLookup(devices);
    expect(lookup.get("iPhone15,2")).toBe("17.0");
  });

  it("skips devices without modelCode or OS version", () => {
    const devices: XcdeviceDevice[] = [
      { ...sampleDevices[0], modelCode: "" },
      { ...sampleDevices[1], operatingSystemVersion: "" },
    ];
    const lookup = createOsVersionLookup(devices);
    expect(lookup.size).toBe(0);
  });
});

describe("getOsVersionForDevice", () => {
  it("returns OS version for known productType", () => {
    const lookup = createOsVersionLookup(sampleDevices);
    expect(getOsVersionForDevice(lookup, "iPhone15,2")).toBe("17.0");
  });

  it("returns undefined for unknown productType", () => {
    const lookup = createOsVersionLookup(sampleDevices);
    expect(getOsVersionForDevice(lookup, "iPhone99,9")).toBeUndefined();
  });
});

describe("createUdidLookup", () => {
  it("creates lookup map from modelCode to UDID", () => {
    const lookup = createUdidLookup(sampleDevices);
    expect(lookup.get("iPhone15,2")).toBe("00008110-001234567890001E");
    expect(lookup.get("iPhone14,5")).toBe("00008120-ABCDEF012345");
  });
});

describe("getUdidForDevice", () => {
  it("returns UDID for known productType", () => {
    const lookup = createUdidLookup(sampleDevices);
    expect(getUdidForDevice(lookup, "iPhone15,2")).toBe("00008110-001234567890001E");
  });

  it("returns undefined for unknown productType", () => {
    const lookup = createUdidLookup(sampleDevices);
    expect(getUdidForDevice(lookup, "iPhone99,9")).toBeUndefined();
  });
});

describe("createNameLookup", () => {
  it("creates lookup map from modelCode to name", () => {
    const lookup = createNameLookup(sampleDevices);
    expect(lookup.get("iPhone15,2")).toBe("Alice's iPhone");
    expect(lookup.get("iPhone14,5")).toBe("Bob's iPhone");
  });
});

describe("getNameForDevice", () => {
  it("returns name for known productType", () => {
    const lookup = createNameLookup(sampleDevices);
    expect(getNameForDevice(lookup, "iPhone15,2")).toBe("Alice's iPhone");
  });

  it("returns undefined for unknown productType", () => {
    const lookup = createNameLookup(sampleDevices);
    expect(getNameForDevice(lookup, "iPhone99,9")).toBeUndefined();
  });
});
