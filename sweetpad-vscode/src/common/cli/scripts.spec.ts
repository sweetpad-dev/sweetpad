import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import * as sweetpadLib from "@sweetpad/native";
import type { Mock } from "vitest";
import * as vscode from "vscode";

import { ExtensionError } from "../errors";
import { exec } from "../exec";
import { getShellDeveloperDir } from "../tasks/shell-env";
import {
  findSchemeFile,
  getBuildSettingsList,
  getSchemes,
  getSimulatorAppPath,
  getSupportedPlatforms,
  getTargets,
  getXcodeBuildCommand,
  locateBuiltApp,
  parseCliJsonOutput,
} from "./scripts";

vi.mock("../exec", () => ({ exec: vi.fn() }));
vi.mock("../tasks/shell-env", () => ({ getShellDeveloperDir: vi.fn() }));
vi.mock("@sweetpad/native", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@sweetpad/native")>()),
  buildSettings: vi.fn(),
  locateApp: vi.fn(),
  pickApp: vi.fn(),
  supportedPlatforms: vi.fn(),
  schemes: vi.fn(),
  targets: vi.fn(),
  locateScheme: vi.fn(),
}));

const mockGetConfiguration = vscode.workspace.getConfiguration as Mock;
const mockExec = exec as Mock;
const mockGetShellDeveloperDir = getShellDeveloperDir as Mock;
const mockBuildSettings = sweetpadLib.buildSettings as Mock;
const mockLocateApp = sweetpadLib.locateApp as Mock;
const mockPickApp = sweetpadLib.pickApp as Mock;
const mockSupportedPlatforms = sweetpadLib.supportedPlatforms as Mock;
const mockSchemes = sweetpadLib.schemes as Mock;
const mockTargets = sweetpadLib.targets as Mock;
const mockLocateScheme = sweetpadLib.locateScheme as Mock;

/** `getWorkspaceConfig` reads `getConfiguration("sweetpad").get(key)`. */
function mockConfig(values: Record<string, unknown>) {
  mockGetConfiguration.mockReturnValue({
    get: vi.fn((key: string) => values[key]),
  });
}

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
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("resolves Xcode projects with the in-process resolver, not xcodebuild", async () => {
    mockConfig({});
    mockBuildSettings.mockReturnValue([{ target: "App", settings: { PRODUCT_NAME: "App" } }]);

    const settings = await getBuildSettingsList({
      workspaceRoot: "/test/workspace",
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
      workspaceRoot: "/test/workspace",
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
      workspaceRoot: "/test/workspace",
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
      workspaceRoot: "/test/workspace",
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
      workspaceRoot: "/test/workspace",
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
        workspaceRoot: "/test/workspace",
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
      workspaceRoot: "/test/workspace",
      scheme: "App",
      configuration: "Debug",
      sdk: undefined,
      xcworkspace: "/proj/App.xcworkspace",
    });

    expect(settings[0].target).toBe("App");
    expect(mockExec).toHaveBeenCalledTimes(1);
  });

  // The build runs xcodebuild in the workspace root with build.args on its command line, so the settings are
  // the ones those arguments give.
  it("gives the resolver sweetpad.build.args and the directory the build runs in", async () => {
    mockConfig({ "build.args": ["PRODUCT_NAME=Other", "-configuration", "Release"] });
    mockBuildSettings.mockReturnValue([{ target: "App", settings: {} }]);

    await getBuildSettingsList({
      workspaceRoot: "/test/workspace",
      scheme: "App",
      configuration: "Debug",
      sdk: undefined,
      xcworkspace: "/proj/App.xcodeproj",
    });

    expect(mockBuildSettings).toHaveBeenCalledWith(
      expect.objectContaining({
        buildArgs: ["PRODUCT_NAME=Other", "-configuration", "Release"],
        workingDirectory: "/test/workspace",
      }),
    );
  });

  it("adds sweetpad.build.args to the xcodebuild query as the build adds them", async () => {
    mockConfig({
      "build.xcodebuildCommand": "/usr/local/bin/xcodebuild-wrapper",
      "build.args": ["PRODUCT_NAME=Other", "-configuration", "Release"],
    });
    mockExec.mockResolvedValue(xcodebuildJson("App"));

    await getBuildSettingsList({
      workspaceRoot: "/test/workspace",
      scheme: "App",
      configuration: "Debug",
      sdk: undefined,
      xcworkspace: "/proj/App.xcodeproj",
    });

    const call = mockExec.mock.calls[0][0] as { command: string; args: string[] };
    expect(call.command).toBe("/usr/local/bin/xcodebuild-wrapper");
    expect(call.args).toContain("PRODUCT_NAME=Other");
    expect(call.args).toContain("-showBuildSettings");
    // The typed configuration replaces the picked one, as on the build's command line.
    expect(call.args[call.args.indexOf("-configuration") + 1]).toBe("Release");
    expect(call.args).not.toContain("Debug");
  });

  it("keeps using xcodebuild for SPM packages, from the package directory", async () => {
    mockConfig({});
    mockExec.mockResolvedValue(xcodebuildJson("MyPackage"));

    const settings = await getBuildSettingsList({
      workspaceRoot: "/test/workspace",
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

describe("locateBuiltApp", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  const located = {
    target: "App",
    path: "/dd/Build/Products/Debug/App.app",
    bundleId: "com.example.app",
    executable: "/dd/Build/Products/Debug/App.app/Contents/MacOS/App",
    settings: { EXECUTABLE_NAME: "App", ENABLE_DEBUG_DYLIB: "YES" },
  };

  it("asks the shared locator, with the build's own arguments", async () => {
    mockConfig({ "build.args": ["PRODUCT_NAME=Other"] });
    mockLocateApp.mockReturnValue(located);

    const app = await locateBuiltApp({
      workspaceRoot: "/test/workspace",
      scheme: "App",
      configuration: "Debug",
      sdk: "macosx",
      xcworkspace: "/proj/App.xcworkspace",
      destination: "platform=macOS",
    });

    expect(mockLocateApp).toHaveBeenCalledWith(
      expect.objectContaining({
        workspace: "/proj/App.xcworkspace",
        scheme: "App",
        configuration: "Debug",
        destination: "platform=macOS",
        buildArgs: ["PRODUCT_NAME=Other"],
        workingDirectory: "/test/workspace",
      }),
    );
    expect(mockBuildSettings).not.toHaveBeenCalled();
    expect(app.appPath).toBe("/dd/Build/Products/Debug/App.app");
    expect(app.executablePath).toBe("/dd/Build/Products/Debug/App.app/Contents/MacOS/App");
    expect(app.bundleIdentifier).toBe("com.example.app");
    expect(app.appName).toBe("App.app");
    expect(app.executableName).toBe("App");
    expect(app.enableDebugDylib).toBe(true);
  });

  it("picks among xcodebuild's settings by the same rules on the xcodebuild route", async () => {
    mockConfig({ "build.xcodebuildCommand": "/usr/local/bin/xcodebuild-wrapper" });
    mockExec.mockResolvedValue(xcodebuildJson("App"));
    mockPickApp.mockReturnValue(located);

    const app = await locateBuiltApp({
      workspaceRoot: "/test/workspace",
      scheme: "App",
      configuration: "Debug",
      sdk: "iphonesimulator",
      xcworkspace: "/proj/App.xcodeproj",
      destination: "platform=iOS Simulator,id=U",
    });

    expect(mockLocateApp).not.toHaveBeenCalled();
    expect(mockPickApp).toHaveBeenCalledWith(
      expect.objectContaining({
        targets: [{ target: "App", settings: { PRODUCT_NAME: "App" } }],
        container: "/proj/App.xcodeproj",
        scheme: "App",
        destination: "platform=iOS Simulator,id=U",
        sdk: "iphonesimulator",
      }),
    );
    const args = mockExec.mock.calls[0][0].args as string[];
    expect(args[args.indexOf("-destination") + 1]).toBe("platform=iOS Simulator,id=U");
    expect(app.target).toBe("App");
  });

  it("runs a Swift package's first target, which builds no .app", async () => {
    mockConfig({});
    mockExec.mockResolvedValue(
      JSON.stringify([
        {
          action: "build",
          target: "Tool",
          buildSettings: { TARGET_BUILD_DIR: "/dd/Debug", EXECUTABLE_PATH: "Tool", FULL_PRODUCT_NAME: "Tool" },
        },
      ]),
    );

    const app = await locateBuiltApp({
      workspaceRoot: "/test/workspace",
      scheme: "Tool",
      configuration: "Debug",
      sdk: "macosx",
      xcworkspace: "/proj/Package.swift",
      destination: "platform=macOS",
    });

    expect(mockLocateApp).not.toHaveBeenCalled();
    expect(mockPickApp).not.toHaveBeenCalled();
    expect(app.executablePath).toBe("/dd/Debug/Tool");
  });
});

describe("getSupportedPlatforms", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("reads the scheme's platforms through the CLI's filter", () => {
    mockSupportedPlatforms.mockReturnValue(["iphoneos", "iphonesimulator"]);

    expect(
      getSupportedPlatforms({ scheme: "App", configuration: "Debug", xcworkspace: "/proj/App.xcodeproj" }),
    ).toEqual(["iphoneos", "iphonesimulator"]);
    expect(mockSupportedPlatforms).toHaveBeenCalledWith("/proj/App.xcodeproj", "App", "Debug");
  });

  it("filters nothing when the platforms can't be told", () => {
    mockSupportedPlatforms.mockReturnValue(null);
    expect(
      getSupportedPlatforms({ scheme: "App", configuration: "Debug", xcworkspace: "/proj/Package.swift" }),
    ).toBeUndefined();

    mockSupportedPlatforms.mockImplementation(() => {
      throw new Error("unreadable project");
    });
    expect(
      getSupportedPlatforms({ scheme: "App", configuration: "Debug", xcworkspace: "/proj/App.xcodeproj" }),
    ).toBeUndefined();
  });
});

describe("findSchemeFile", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  // The addon reads the file `xcodebuild` reads for this container: never another user's
  // `xcuserdata`, never a same-named scheme outside the container.
  it("asks the addon for the container's own scheme file", async () => {
    mockLocateScheme.mockReturnValue("/proj/App.xcodeproj/xcshareddata/xcschemes/App.xcscheme");
    expect(await findSchemeFile("/proj/App.xcworkspace", "App")).toBe(
      "/proj/App.xcodeproj/xcshareddata/xcschemes/App.xcscheme",
    );
    expect(mockLocateScheme).toHaveBeenCalledWith("/proj/App.xcworkspace", "App");
  });

  it("has no file for an autocreated scheme", async () => {
    mockLocateScheme.mockReturnValue(null);
    expect(await findSchemeFile("/proj/App.xcodeproj", "App")).toBeUndefined();
  });
});

describe("package names", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  // The addon reads the package the way `xcodebuild -list` does, scheme files
  // included, so nothing runs `swift` in the package directory from here.
  it("reads a Swift package's schemes through the addon, with the shell's toolchain", async () => {
    mockConfig({ "build.swiftCommand": "/opt/swift/bin/swift" });
    mockGetShellDeveloperDir.mockResolvedValue("/Applications/Xcode-beta.app/Contents/Developer");
    mockSchemes.mockResolvedValue(["alpha", "B10Multi-Package", "Beta"]);

    const schemes = await getSchemes({ xcworkspace: "/proj/Package.swift" });

    expect(schemes).toEqual([{ name: "alpha" }, { name: "B10Multi-Package" }, { name: "Beta" }]);
    expect(mockSchemes).toHaveBeenCalledWith("/proj/Package.swift", {
      swift: "/opt/swift/bin/swift",
      developerDir: "/Applications/Xcode-beta.app/Contents/Developer",
    });
    expect(mockExec).not.toHaveBeenCalled();
  });

  it("reads a Swift package's targets through the addon", async () => {
    mockConfig({});
    mockGetShellDeveloperDir.mockResolvedValue(undefined);
    mockTargets.mockResolvedValue(["Zeta", "ZetaTests"]);

    const targets = await getTargets({ xcworkspace: "/proj/Package.swift" });

    expect(targets).toEqual(["Zeta", "ZetaTests"]);
    expect(mockTargets).toHaveBeenCalledWith("/proj/Package.swift", { swift: undefined, developerDir: undefined });
    expect(mockExec).not.toHaveBeenCalled();
  });

  it("offers no schemes when the manifest doesn't evaluate", async () => {
    mockConfig({});
    mockSchemes.mockRejectedValue(new Error("swift package dump-package exited with exit status: 1"));

    expect(await getSchemes({ xcworkspace: "/proj/Package.swift" })).toEqual([]);
  });

  it("hands an Xcode container's package manifests the same toolchain", async () => {
    mockConfig({});
    mockGetShellDeveloperDir.mockResolvedValue("/Applications/Xcode.app/Contents/Developer");
    mockSchemes.mockResolvedValue(["App"]);

    await getSchemes({ xcworkspace: "/proj/App.xcworkspace" });

    expect(mockSchemes).toHaveBeenCalledWith("/proj/App.xcworkspace", {
      swift: undefined,
      developerDir: "/Applications/Xcode.app/Contents/Developer",
    });
  });
});

describe("getSimulatorAppPath", () => {
  let xcode: string;

  beforeEach(async () => {
    vi.resetAllMocks();
    xcode = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-xcode-"));
    mockGetShellDeveloperDir.mockResolvedValue(path.join(xcode, "Contents", "Developer"));
  });

  afterEach(async () => {
    await fs.rm(xcode, { recursive: true, force: true });
  });

  it("finds Simulator.app inside the developer dir (Xcode 26 and earlier)", async () => {
    const app = path.join(xcode, "Contents", "Developer", "Applications", "Simulator.app");
    await fs.mkdir(app, { recursive: true });
    await expect(getSimulatorAppPath({ workspaceRoot: "/workspace" })).resolves.toBe(app);
  });

  it("finds DeviceHub.app beside the developer dir (Xcode 27)", async () => {
    const app = path.join(xcode, "Contents", "Applications", "DeviceHub.app");
    await fs.mkdir(app, { recursive: true });
    await expect(getSimulatorAppPath({ workspaceRoot: "/workspace" })).resolves.toBe(app);
  });

  it("falls back to the bare name when the install matches neither layout", async () => {
    await expect(getSimulatorAppPath({ workspaceRoot: "/workspace" })).resolves.toBe("Simulator");
  });
});
