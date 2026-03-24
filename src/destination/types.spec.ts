import { macOSDestination } from "./types";

describe("macOSDestination", () => {
  it("has correct type and platform", () => {
    const dest = new macOSDestination({ name: "My Mac", arch: "arm64" });
    expect(dest.type).toBe("macOS");
    expect(dest.typeLabel).toBe("macOS Device");
    expect(dest.platform).toBe("macosx");
  });

  it("generates correct id", () => {
    const dest = new macOSDestination({ name: "My Mac", arch: "arm64" });
    expect(dest.id).toBe("macos-My Mac");
  });

  it("generates correct label", () => {
    const dest = new macOSDestination({ name: "My Mac", arch: "arm64" });
    expect(dest.label).toBe("My Mac");
  });

  it("includes arch in quickPickDetails", () => {
    const arm = new macOSDestination({ name: "My Mac", arch: "arm64" });
    expect(arm.quickPickDetails).toBe("Type: macOS Device, Arch: arm64");

    const intel = new macOSDestination({ name: "My Mac", arch: "x86_64" });
    expect(intel.quickPickDetails).toBe("Type: macOS Device, Arch: x86_64");
  });

  it("returns laptop icon", () => {
    const dest = new macOSDestination({ name: "My Mac", arch: "arm64" });
    expect(dest.icon).toBe("sweetpad-device-laptop");
  });
});
