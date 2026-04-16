import { getWorkspaceConfig } from "../common/config";
import {
  buildDefaultPymobiledevice3Regex,
  buildPymobiledevice3Args,
  escapeRegex,
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
  const defaultRegex = buildDefaultPymobiledevice3Regex(base.processName);

  it("uses SweetPad defaults when extras are empty", () => {
    const result = buildPymobiledevice3Args({ ...base, rawExtraArgs: [] });
    expect(result).toEqual({
      kind: "ok",
      args: ["syslog", "live", "--label", "--process-name", "pulse_2050", "--regex", defaultRegex],
      hasProcessNameOverride: false,
      hasRegexOverride: false,
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
      "--regex",
      defaultRegex,
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
      "--regex",
      buildDefaultPymobiledevice3Regex("MyApp"),
      "--process-name",
      "MyApp",
    ]);
  });

  it("replaces --regex when overridden", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--regex", "com.example.custom"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasRegexOverride).toBe(true);
    expect(result.args).toEqual([
      "syslog",
      "live",
      "--label",
      "--process-name",
      "pulse_2050",
      "--regex",
      "com.example.custom",
    ]);
  });

  it("accepts short aliases -p and -e", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["-p", "Other", "-e", "foo"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasProcessNameOverride).toBe(true);
    expect(result.hasRegexOverride).toBe(true);
    expect(result.args).toEqual(["syslog", "live", "--label", "-p", "Other", "-e", "foo"]);
  });

  it("suppresses --regex when value is null", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--regex", null],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasRegexOverride).toBe(true);
    expect(result.args).toEqual(["syslog", "live", "--label", "--process-name", "pulse_2050"]);
  });

  it("suppresses --process-name when value is null", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--process-name", null],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasProcessNameOverride).toBe(true);
    expect(result.args).toEqual(["syslog", "live", "--label", "--regex", defaultRegex]);
  });

  it("suppresses both with null values", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--process-name", null, "--regex", null],
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
      "--regex",
      buildDefaultPymobiledevice3Regex("Explicit"),
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
    expect(result.args).toEqual(["syslog", "live", "--label"]);
  });

  it("escapes regex metacharacters in the default regex", () => {
    const result = buildPymobiledevice3Args({
      processName: "pulse_2050+beta",
      rawExtraArgs: [],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.args).toEqual([
      "syslog",
      "live",
      "--label",
      "--process-name",
      "pulse_2050+beta",
      "--regex",
      "pulse_2050\\+beta\\{pulse_2050\\+beta(\\.debug\\.dylib)?\\}\\[",
    ]);
  });

  it("drops trailing flag with no value", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--regex"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.hasRegexOverride).toBe(true);
    expect(result.args).toEqual(["syslog", "live", "--label", "--process-name", "pulse_2050"]);
  });

  it("preserves extras declared after a null-suppressed flag", () => {
    const result = buildPymobiledevice3Args({
      ...base,
      rawExtraArgs: ["--regex", null, "--verbose"],
    });
    if (result.kind !== "ok") throw new Error("expected ok");
    expect(result.args).toEqual(["syslog", "live", "--label", "--process-name", "pulse_2050", "--verbose"]);
  });
});

describe("buildDefaultPymobiledevice3Regex", () => {
  it("matches the main executable and optional debug dylib image name", () => {
    expect(buildDefaultPymobiledevice3Regex("pulse_2050")).toBe("pulse_2050\\{pulse_2050(\\.debug\\.dylib)?\\}\\[");
  });
});

describe("escapeRegex", () => {
  it("escapes regex metacharacters", () => {
    expect(escapeRegex("pulse_2050+beta")).toBe("pulse_2050\\+beta");
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

  it("quotes regex args with shell metacharacters", () => {
    expect(formatCommandLine("pymobiledevice3", ["--regex", "pulse_2050\\{pulse_2050(\\.debug\\.dylib)?\\}\\["])).toBe(
      "pymobiledevice3 --regex 'pulse_2050\\{pulse_2050(\\.debug\\.dylib)?\\}\\['",
    );
  });
});
