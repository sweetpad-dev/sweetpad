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
  const base = { processName: "pulse_2050" };

  it("uses SweetPad defaults when extras are empty", () => {
    const result = buildPymobiledevice3Args({ ...base, rawExtraArgs: [] });
    expect(result).toEqual({
      kind: "ok",
      args: ["--no-color", "syslog", "live", "--label", "--process-name", "pulse_2050"],
      hasProcessNameOverride: false,
    });
  });

  it("passes through unrelated extras in order", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--verbose", "--image-offset"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.args).toEqual([
      "--no-color",
      "syslog",
      "live",
      "--label",
      "--process-name",
      "pulse_2050",
      "--verbose",
      "--image-offset",
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
      "--no-color",
      "syslog",
      "live",
      "--label",
      "--process-name",
      "MyApp",
    ]);
  });

  it("accepts short alias -p", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["-p", "Other"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasProcessNameOverride).toBe(true);
    expect(result.args).toEqual(["--no-color", "syslog", "live", "--label", "-p", "Other"]);
  });

  it("suppresses --process-name when value is null", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--process-name", null],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasProcessNameOverride).toBe(true);
    expect(result.args).toEqual(["--no-color", "syslog", "live", "--label"]);
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
      "--no-color",
      "syslog",
      "live",
      "--label",
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
    expect(result.args).toEqual(["--no-color", "syslog", "live", "--label"]);
  });

  it("drops trailing --process-name flag with no value", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--process-name"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasProcessNameOverride).toBe(true);
    expect(result.args).toEqual(["--no-color", "syslog", "live", "--label"]);
  });

  it("preserves extras declared after a null-suppressed flag", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--process-name", null, "--verbose"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.args).toEqual(["--no-color", "syslog", "live", "--label", "--verbose"]);
  });

  it("does not inject any server-side message filter (--match / --regex)", () => {
    const result = buildPymobiledevice3Args({ ...base, rawExtraArgs: [] });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.args).not.toContain("--match");
    expect(result.args).not.toContain("--regex");
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
    expect(formatCommandLine("pymobiledevice3", ["--process-name", "a b"])).toBe(
      "pymobiledevice3 --process-name 'a b'",
    );
  });
});
