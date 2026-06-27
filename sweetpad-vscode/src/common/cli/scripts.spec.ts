import * as sweetpadLib from "@sweetpad/native";
import type { Mock } from "vitest";
import * as vscode from "vscode";

import { ExtensionError } from "../errors";
import { exec } from "../exec";
import { getShellDeveloperDir } from "../tasks/shell-env";
import { getBuildSettingsList, getXcodeBuildCommand, parseCliJsonOutput } from "./scripts";

vi.mock("../exec", () => ({ exec: vi.fn() }));
vi.mock("../tasks/shell-env", () => ({ getShellDeveloperDir: vi.fn() }));
vi.mock("@sweetpad/native", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@sweetpad/native")>()),
  buildSettings: vi.fn(),
}));

const mockGetConfiguration = vscode.workspace.getConfiguration as Mock;
const mockExec = exec as Mock;
const mockGetShellDeveloperDir = getShellDeveloperDir as Mock;
const mockBuildSettings = sweetpadLib.buildSettings as Mock;

describe("getXcodeBuildCommand", () => {
  const originalEnv = process.env;

  beforeEach(() => {
    vi.resetAllMocks();
    process.env = { ...originalEnv };
  });

  afterAll(() => {
    process.env = originalEnv;
  });

  it("returns default 'xcodebuild' when no config is set", () => {
    mockGetConfiguration.mockReturnValue({
      get: vi.fn().mockReturnValue(undefined),
    });
    expect(getXcodeBuildCommand()).toBe("xcodebuild");
  });

  it("returns default 'xcodebuild' when config is null", () => {
    mockGetConfiguration.mockReturnValue({
      get: vi.fn().mockReturnValue(null),
    });
    expect(getXcodeBuildCommand()).toBe("xcodebuild");
  });

  it("returns default 'xcodebuild' when config is empty string", () => {
    mockGetConfiguration.mockReturnValue({
      get: vi.fn().mockReturnValue(""),
    });
    expect(getXcodeBuildCommand()).toBe("xcodebuild");
  });

  it("returns custom command when configured with absolute path", () => {
    mockGetConfiguration.mockReturnValue({
      get: vi.fn().mockReturnValue("/Applications/Xcode-beta.app/Contents/Developer/usr/bin/xcodebuild"),
    });
    expect(getXcodeBuildCommand()).toBe("/Applications/Xcode-beta.app/Contents/Developer/usr/bin/xcodebuild");
  });

  it("expands environment variable in config value", () => {
    process.env.CUSTOM_XCODEBUILD = "/custom/path/to/xcodebuild";
    mockGetConfiguration.mockReturnValue({
      get: vi.fn().mockReturnValue("${env:CUSTOM_XCODEBUILD}"),
    });
    expect(getXcodeBuildCommand()).toBe("/custom/path/to/xcodebuild");
  });

  it("expands environment variable with additional path components", () => {
    process.env.XCODE_PATH = "/Applications/Xcode-beta.app";
    mockGetConfiguration.mockReturnValue({
      get: vi.fn().mockReturnValue("${env:XCODE_PATH}/Contents/Developer/usr/bin/xcodebuild"),
    });
    expect(getXcodeBuildCommand()).toBe("/Applications/Xcode-beta.app/Contents/Developer/usr/bin/xcodebuild");
  });

  it("keeps original placeholder when environment variable is not set", () => {
    process.env.NONEXISTENT_VAR = undefined;
    mockGetConfiguration.mockReturnValue({
      get: vi.fn().mockReturnValue("${env:NONEXISTENT_VAR}"),
    });
    expect(getXcodeBuildCommand()).toBe("${env:NONEXISTENT_VAR}");
  });

  it("expands empty environment variable to empty string", () => {
    process.env.EMPTY_VAR = "";
    mockGetConfiguration.mockReturnValue({
      get: vi.fn().mockReturnValue("prefix${env:EMPTY_VAR}suffix"),
    });
    expect(getXcodeBuildCommand()).toBe("prefixsuffix");
  });
});

describe("parseCliJsonOutput", () => {
  it("simple", async () => {
    const input = `{"key1":"value1","key2":2}`;
    const obj = parseCliJsonOutput(input);
    expect(obj).toEqual({ key1: "value1", key2: 2 });
  });

  it("with noise", async () => {
    const input = `Some initial noise
{"key1":"value1","key2":2}
Some trailing noise`;
    const obj = parseCliJsonOutput(input);
    expect(obj).toEqual({ key1: "value1", key2: 2 });
  });

  it("multiple json objects", async () => {
    const input = `Noise before
{"key1":"value1"}
Some noise in between
{"key2":2}
Noise after`;
    expect(() => parseCliJsonOutput(input)).toThrow(ExtensionError);
  });

  it("no valid json", async () => {
    const input = `Just some random text
No JSON here!`;
    expect(() => parseCliJsonOutput(input)).toThrow(ExtensionError);
  });

  it("malformed json", async () => {
    const input = `Noise
{"key1":"value1", "key2":2
More noise`;
    expect(() => parseCliJsonOutput(input)).toThrow(ExtensionError);
  });

  it("json array", async () => {
    const input = `Noise
["item1", "item2", "item3"]
More noise`;
    const obj = parseCliJsonOutput(input);
    expect(obj).toEqual(["item1", "item2", "item3"]);
  });
  it("json array with nose and objects", async () => {
    const input = `Noise
[{"key1":"value1"}, {"key2":2}]
More noise`;
    const obj = parseCliJsonOutput(input);
    expect(obj).toEqual([{ key1: "value1" }, { key2: 2 }]);
  });
});

const xcodebuildJson = (target: string) =>
  JSON.stringify([{ action: "build", target, buildSettings: { PRODUCT_NAME: target } }]);

describe("getBuildSettingsList", () => {
  /** `getWorkspaceConfig` reads `getConfiguration("sweetpad").get(key)`. */
  function mockConfig(values: Record<string, unknown>) {
    mockGetConfiguration.mockReturnValue({
      get: vi.fn((key: string) => values[key]),
    });
  }

  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("resolves Xcode projects with the in-process resolver, not xcodebuild", async () => {
    mockConfig({});
    mockBuildSettings.mockReturnValue([{ target: "App", settings: { PRODUCT_NAME: "App" } }]);

    const settings = await getBuildSettingsList({
      scheme: "App",
      configuration: "Debug",
      sdk: undefined,
      xcworkspace: "/proj/App.xcworkspace",
    });

    expect(settings).toHaveLength(1);
    expect(settings[0].target).toBe("App");
    expect(mockBuildSettings).toHaveBeenCalledWith(expect.objectContaining({ workspace: "/proj/App.xcworkspace" }));
    expect(mockExec).not.toHaveBeenCalled();
  });

  it("passes the login shell's DEVELOPER_DIR to the resolver as `xcode`", async () => {
    mockConfig({});
    mockGetShellDeveloperDir.mockResolvedValue("/Applications/Xcode-beta.app/Contents/Developer");
    mockBuildSettings.mockReturnValue([{ target: "App", settings: {} }]);

    await getBuildSettingsList({
      scheme: "App",
      configuration: "Debug",
      sdk: undefined,
      xcworkspace: "/proj/App.xcworkspace",
    });

    expect(mockBuildSettings).toHaveBeenCalledWith(
      expect.objectContaining({ xcode: "/Applications/Xcode-beta.app/Contents/Developer" }),
    );
  });

  it("passes a bare .xcodeproj as `project` to the resolver", async () => {
    mockConfig({});
    mockBuildSettings.mockReturnValue([{ target: "App", settings: {} }]);

    await getBuildSettingsList({
      scheme: "App",
      configuration: "Debug",
      sdk: undefined,
      xcworkspace: "/proj/App.xcodeproj",
    });

    expect(mockBuildSettings).toHaveBeenCalledWith(expect.objectContaining({ project: "/proj/App.xcodeproj" }));
  });

  it("routes through a customized build.xcodebuildCommand instead of the resolver", async () => {
    mockConfig({ "build.xcodebuildCommand": "/usr/local/bin/xcodebuild-wrapper" });
    mockExec.mockResolvedValue(xcodebuildJson("App"));

    const settings = await getBuildSettingsList({
      scheme: "App",
      configuration: "Debug",
      sdk: undefined,
      xcworkspace: "/proj/App.xcworkspace",
    });

    expect(settings[0].target).toBe("App");
    expect(mockBuildSettings).not.toHaveBeenCalled();
    expect(mockExec).toHaveBeenCalledWith(
      expect.objectContaining({
        command: "/usr/local/bin/xcodebuild-wrapper",
        args: expect.arrayContaining(["-showBuildSettings", "-workspace", "/proj/App.xcworkspace"]),
      }),
    );
  });

  it("uses -project for a bare .xcodeproj on the xcodebuild path", async () => {
    mockConfig({ "build.xcodebuildCommand": "/usr/local/bin/xcodebuild-wrapper" });
    mockExec.mockResolvedValue(xcodebuildJson("App"));

    await getBuildSettingsList({
      scheme: "App",
      configuration: "Debug",
      sdk: undefined,
      xcworkspace: "/proj/App.xcodeproj",
    });

    const args = mockExec.mock.calls[0][0].args as string[];
    expect(args).toContain("-project");
    expect(args).not.toContain("-workspace");
  });

  it("throws an ExtensionError with a hint when the resolver fails and fallback is off", async () => {
    mockConfig({});
    mockBuildSettings.mockImplementation(() => {
      throw new Error("unparseable pbxproj");
    });

    await expect(
      getBuildSettingsList({
        scheme: "App",
        configuration: "Debug",
        sdk: undefined,
        xcworkspace: "/proj/App.xcworkspace",
      }),
    ).rejects.toThrow(/Failed to resolve build settings: unparseable pbxproj/);
    expect(mockExec).not.toHaveBeenCalled();
  });

  it("falls back to xcodebuild on resolver failure when system.xcodebuildFallback is on", async () => {
    mockConfig({ "system.xcodebuildFallback": true });
    mockBuildSettings.mockImplementation(() => {
      throw new Error("unparseable pbxproj");
    });
    mockExec.mockResolvedValue(xcodebuildJson("App"));

    const settings = await getBuildSettingsList({
      scheme: "App",
      configuration: "Debug",
      sdk: undefined,
      xcworkspace: "/proj/App.xcworkspace",
    });

    expect(settings[0].target).toBe("App");
    expect(mockExec).toHaveBeenCalledTimes(1);
  });

  it("keeps using xcodebuild for SPM packages, from the package directory", async () => {
    mockConfig({});
    mockExec.mockResolvedValue(xcodebuildJson("MyPackage"));

    const settings = await getBuildSettingsList({
      scheme: "MyPackage",
      configuration: "Debug",
      sdk: undefined,
      xcworkspace: "/proj/Package.swift",
    });

    expect(settings[0].target).toBe("MyPackage");
    expect(mockBuildSettings).not.toHaveBeenCalled();
    expect(mockExec).toHaveBeenCalledWith(expect.objectContaining({ cwd: "/proj" }));
  });
});
