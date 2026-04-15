import { getWorkspaceConfig } from "../common/config";
import {
  buildPymobiledevice3Args,
  formatCommandLine,
  getDeviceLaunchEnvExtras,
  resolveDeviceLogBackend,
  shellQuote,
} from "./device-log-backend";

jest.mock("../common/config", () => ({
  getWorkspaceConfig: jest.fn(),
}));

const mockedGetConfig = getWorkspaceConfig as jest.MockedFunction<typeof getWorkspaceConfig>;

describe("resolveDeviceLogBackend", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("defaults to off when config is missing", () => {
    mockedGetConfig.mockReturnValue(undefined);
    expect(resolveDeviceLogBackend()).toBe("off");
  });

  it("returns explicit off", () => {
    mockedGetConfig.mockReturnValue("off");
    expect(resolveDeviceLogBackend()).toBe("off");
  });

  it("returns explicit osActivityDtMode", () => {
    mockedGetConfig.mockReturnValue("osActivityDtMode");
    expect(resolveDeviceLogBackend()).toBe("osActivityDtMode");
  });

  it("returns explicit pymobiledevice3", () => {
    mockedGetConfig.mockReturnValue("pymobiledevice3");
    expect(resolveDeviceLogBackend()).toBe("pymobiledevice3");
  });
});

describe("getDeviceLaunchEnvExtras", () => {
  it("returns OS_ACTIVITY_DT_MODE for osActivityDtMode", () => {
    expect(getDeviceLaunchEnvExtras("osActivityDtMode")).toEqual({ OS_ACTIVITY_DT_MODE: "enable" });
  });

  it("returns empty for off", () => {
    expect(getDeviceLaunchEnvExtras("off")).toEqual({});
  });

  it("returns empty for pymobiledevice3", () => {
    expect(getDeviceLaunchEnvExtras("pymobiledevice3")).toEqual({});
  });
});

describe("buildPymobiledevice3Args", () => {
  const base = { processName: "pulse_2050", bundleIdentifier: "dev.tuist.pulse-2050" };

  it("uses SweetPad defaults when extras are empty", () => {
    const result = buildPymobiledevice3Args({ ...base, rawExtraArgs: [] });
    expect(result).toEqual({
      kind: "ok",
      args: ["syslog", "live", "--label", "--process-name", "pulse_2050", "--match", "dev.tuist.pulse-2050"],
      hasProcessNameOverride: false,
      hasMatchOverride: false,
    });
  });

  it("passes through unrelated extras in order", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--verbose", "--color"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.args).toEqual([
      "syslog",
      "live",
      "--label",
      "--process-name",
      "pulse_2050",
      "--match",
      "dev.tuist.pulse-2050",
      "--verbose",
      "--color",
    ]);
  });

  it("replaces --process-name when overridden", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--process-name", "MyApp"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasProcessNameOverride).toBe(true);
    expect(result.args).toEqual([
      "syslog",
      "live",
      "--label",
      "--match",
      "dev.tuist.pulse-2050",
      "--process-name",
      "MyApp",
    ]);
  });

  it("replaces --match when overridden", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--match", "com.example.custom"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasMatchOverride).toBe(true);
    expect(result.args).toEqual([
      "syslog",
      "live",
      "--label",
      "--process-name",
      "pulse_2050",
      "--match",
      "com.example.custom",
    ]);
  });

  it("accepts short aliases -p and -m", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["-p", "Other", "-m", "foo"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasProcessNameOverride).toBe(true);
    expect(result.hasMatchOverride).toBe(true);
    expect(result.args).toEqual(["syslog", "live", "--label", "-p", "Other", "-m", "foo"]);
  });

  it("suppresses --match when value is null", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--match", null],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasMatchOverride).toBe(true);
    expect(result.args).toEqual(["syslog", "live", "--label", "--process-name", "pulse_2050"]);
  });

  it("suppresses --process-name when value is null", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--process-name", null],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasProcessNameOverride).toBe(true);
    expect(result.args).toEqual(["syslog", "live", "--label", "--match", "dev.tuist.pulse-2050"]);
  });

  it("suppresses both with null values", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--process-name", null, "--match", null],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.args).toEqual(["syslog", "live", "--label"]);
  });

  it("returns missingProcessName when process name is undefined and no override", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      processName: undefined,
      rawExtraArgs: [],
    });
    expect(result).toEqual({ kind: "missingProcessName" });
  });

  it("accepts missing process name when overridden", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      processName: undefined,
      rawExtraArgs: ["--process-name", "Explicit"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.args).toEqual([
      "syslog",
      "live",
      "--label",
      "--match",
      "dev.tuist.pulse-2050",
      "--process-name",
      "Explicit",
    ]);
  });

  it("accepts missing process name when --process-name is null-suppressed", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      processName: undefined,
      rawExtraArgs: ["--process-name", null],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.args).toEqual(["syslog", "live", "--label", "--match", "dev.tuist.pulse-2050"]);
  });

  it("drops trailing flag with no value", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--match"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasMatchOverride).toBe(true);
    expect(result.args).toEqual(["syslog", "live", "--label", "--process-name", "pulse_2050"]);
  });

  it("preserves extras declared after a null-suppressed flag", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--match", null, "--verbose"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.args).toEqual(["syslog", "live", "--label", "--process-name", "pulse_2050", "--verbose"]);
  });
});

describe("shellQuote", () => {
  it("leaves bare identifiers unquoted", () => {
    expect(shellQuote("pulse_2050")).toBe("pulse_2050");
    expect(shellQuote("com.example.app")).toBe("com.example.app");
    expect(shellQuote("/usr/bin/foo")).toBe("/usr/bin/foo");
  });

  it("single-quotes values with spaces", () => {
    expect(shellQuote("hello world")).toBe("'hello world'");
  });

  it("single-quotes empty string", () => {
    expect(shellQuote("")).toBe("''");
  });

  it("escapes embedded single quotes", () => {
    expect(shellQuote("it's")).toBe("'it'\\''s'");
  });

  it("quotes shell metacharacters", () => {
    expect(shellQuote("a|b")).toBe("'a|b'");
    expect(shellQuote("$foo")).toBe("'$foo'");
  });
});

describe("formatCommandLine", () => {
  it("joins command and args with spaces", () => {
    expect(formatCommandLine("pymobiledevice3", ["syslog", "live"])).toBe("pymobiledevice3 syslog live");
  });

  it("quotes args that need it", () => {
    expect(formatCommandLine("pymobiledevice3", ["--match", "a b"])).toBe("pymobiledevice3 --match 'a b'");
  });
});
