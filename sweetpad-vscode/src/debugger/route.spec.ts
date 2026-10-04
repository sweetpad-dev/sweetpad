/**
 * Unit tests for picking the debugger a `sweetpad-lldb` session runs on: `sweetpad dap` or
 * CodeLLDB, per `sweetpad.debugger.adapter`, what is installed, and the destination.
 */

import type { Destination } from "../destination/types";
import type { SweetpadCliStatus } from "./cli";
import { chooseDebugRoute, deviceNeedingCodelldb, unavailableRouteError } from "./route";

vi.mock("../common/logger", () => ({
  commonLogger: { log: vi.fn(), debug: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

const READY: SweetpadCliStatus = { kind: "ready", path: "/opt/homebrew/bin/sweetpad" };
const NO_DAP: SweetpadCliStatus = { kind: "no-dap", path: "/usr/local/bin/sweetpad" };
const MISSING: SweetpadCliStatus = { kind: "missing", configuredPath: undefined };
const OLD_DEVICE = { name: "Old iPhone", osVersion: "16.7" };

describe("chooseDebugRoute", () => {
  describe("auto", () => {
    it("uses the CLI when it serves dap", () => {
      expect(
        chooseDebugRoute({ adapter: "auto", cli: READY, codelldbInstalled: true, deviceNeedingCodelldb: undefined }),
      ).toEqual({ kind: "sweetpad", cliPath: "/opt/homebrew/bin/sweetpad" });
    });

    it("uses the CLI without CodeLLDB installed", () => {
      expect(
        chooseDebugRoute({ adapter: "auto", cli: READY, codelldbInstalled: false, deviceNeedingCodelldb: undefined })
          .kind,
      ).toBe("sweetpad");
    });

    it("falls back to CodeLLDB when the CLI is missing", () => {
      expect(
        chooseDebugRoute({ adapter: "auto", cli: MISSING, codelldbInstalled: true, deviceNeedingCodelldb: undefined }),
      ).toEqual({ kind: "codelldb" });
    });

    it("falls back to CodeLLDB when the CLI is too old for dap", () => {
      expect(
        chooseDebugRoute({ adapter: "auto", cli: NO_DAP, codelldbInstalled: true, deviceNeedingCodelldb: undefined }),
      ).toEqual({ kind: "codelldb" });
    });

    it("sends a device without devicectl to CodeLLDB even with the CLI ready", () => {
      expect(
        chooseDebugRoute({ adapter: "auto", cli: READY, codelldbInstalled: true, deviceNeedingCodelldb: OLD_DEVICE }),
      ).toEqual({ kind: "codelldb" });
    });

    it("explains that an old device needs CodeLLDB when it is missing", () => {
      const route = chooseDebugRoute({
        adapter: "auto",
        cli: READY,
        codelldbInstalled: false,
        deviceNeedingCodelldb: OLD_DEVICE,
      });
      expect(route).toEqual({ kind: "unavailable", reason: { why: "device-needs-codelldb", device: OLD_DEVICE } });
    });

    it("fails with both options when neither is installed", () => {
      const route = chooseDebugRoute({
        adapter: "auto",
        cli: MISSING,
        codelldbInstalled: false,
        deviceNeedingCodelldb: undefined,
      });
      expect(route).toEqual({ kind: "unavailable", reason: { why: "nothing-installed", cli: MISSING } });
    });
  });

  describe("sweetpad", () => {
    it("uses the CLI even for a device without devicectl", () => {
      expect(
        chooseDebugRoute({
          adapter: "sweetpad",
          cli: READY,
          codelldbInstalled: true,
          deviceNeedingCodelldb: OLD_DEVICE,
        }).kind,
      ).toBe("sweetpad");
    });

    it.each([
      ["missing", MISSING],
      ["too old", NO_DAP],
    ])("fails when the CLI is %s, without falling back to CodeLLDB", (_label, cli) => {
      expect(
        chooseDebugRoute({ adapter: "sweetpad", cli: cli, codelldbInstalled: true, deviceNeedingCodelldb: undefined }),
      ).toEqual({ kind: "unavailable", reason: { why: "cli-unusable", cli: cli } });
    });

    it("treats a CLI that was not looked up as missing", () => {
      expect(
        chooseDebugRoute({
          adapter: "sweetpad",
          cli: undefined,
          codelldbInstalled: true,
          deviceNeedingCodelldb: undefined,
        }),
      ).toEqual({ kind: "unavailable", reason: { why: "cli-unusable", cli: MISSING } });
    });
  });

  describe("codelldb", () => {
    it("uses CodeLLDB even with the CLI ready", () => {
      expect(
        chooseDebugRoute({
          adapter: "codelldb",
          cli: READY,
          codelldbInstalled: true,
          deviceNeedingCodelldb: undefined,
        }),
      ).toEqual({ kind: "codelldb" });
    });

    it("fails when CodeLLDB is missing", () => {
      expect(
        chooseDebugRoute({
          adapter: "codelldb",
          cli: undefined,
          codelldbInstalled: false,
          deviceNeedingCodelldb: undefined,
        }),
      ).toEqual({ kind: "unavailable", reason: { why: "codelldb-missing" } });
    });
  });
});

function labels(error: ReturnType<typeof unavailableRouteError>): string[] {
  return error.options?.actions?.map((action) => action.label) ?? [];
}

describe("unavailableRouteError", () => {
  it("names both the CLI and CodeLLDB when neither is installed", () => {
    const error = unavailableRouteError({ why: "nothing-installed", cli: MISSING });
    expect(error.message).toContain("brew install sweetpad-dev/tap/sweetpad");
    expect(error.message).toContain("CodeLLDB");
    expect(labels(error)).toEqual(["Install SweetPad CLI", "Install CodeLLDB"]);
  });

  it("offers an update when the CLI is too old", () => {
    const error = unavailableRouteError({ why: "nothing-installed", cli: NO_DAP });
    expect(error.message).toContain("/usr/local/bin/sweetpad");
    expect(error.message).toContain("brew upgrade sweetpad-dev/tap/sweetpad");
    expect(labels(error)).toEqual(["Update SweetPad CLI", "Install CodeLLDB"]);
  });

  it("names a configured CLI path that is not there", () => {
    const error = unavailableRouteError({
      why: "cli-unusable",
      cli: { kind: "missing", configuredPath: "/nowhere/sweetpad" },
    });
    expect(error.message).toContain("sweetpad.debugger.cliPath");
    expect(error.message).toContain("/nowhere/sweetpad");
  });

  it("names the device that needs CodeLLDB", () => {
    const error = unavailableRouteError({ why: "device-needs-codelldb", device: OLD_DEVICE });
    expect(error.message).toContain("Old iPhone (16.7)");
    expect(labels(error)).toEqual(["Install CodeLLDB"]);
  });
});

describe("deviceNeedingCodelldb", () => {
  it("flags a device without devicectl", () => {
    const device = {
      type: "iOSDevice",
      name: "Old iPhone",
      osVersion: "16.7",
      supportsDevicectl: false,
    } as unknown as Destination;
    expect(deviceNeedingCodelldb(device)).toEqual(OLD_DEVICE);
  });

  it("passes a device with devicectl", () => {
    const device = {
      type: "iOSDevice",
      name: "New iPhone",
      osVersion: "18.0",
      supportsDevicectl: true,
    } as unknown as Destination;
    expect(deviceNeedingCodelldb(device)).toBeUndefined();
  });

  it("passes simulators and the Mac", () => {
    expect(deviceNeedingCodelldb({ type: "iOSSimulator" } as unknown as Destination)).toBeUndefined();
    expect(deviceNeedingCodelldb({ type: "macOS" } as unknown as Destination)).toBeUndefined();
  });
});
