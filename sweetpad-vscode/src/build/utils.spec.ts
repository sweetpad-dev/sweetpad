import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync } from "node:fs";
import os from "node:os";
import path from "node:path";

import * as sweetpadLib from "@sweetpad/native";
import type { Mock } from "vitest";
import * as vscode from "vscode";

import { getBspConfigFile } from "../bsp/paths";
import {
  findSchemeFile,
  generateBuildServerConfig,
  generateSweetpadBuildServerConfig,
  getBuildSettingsList,
  getSweetpadCliPath,
} from "../common/cli/scripts";
import { isFileExists, readJsonFile } from "../common/files";
import { WorkspaceContextService } from "../common/workspace-context";
import type { WorkspaceStateService } from "../common/workspace-state";
import {
  XCODEBUILD_VALUE_FLAGS,
  XcodeCommandBuilder,
  activateCurrentXcodeWorkspacePath,
  detectXcodeWorkspacesPaths,
  findXcodeWorkspaceInDirectory,
  generateBuildServerConfigOnBuild,
  getCurrentXcodeWorkspacePath,
  getSchemeLaunchSettings,
  prepareDerivedDataPath,
  repairStaleBuildServerConfig,
  workspaceFoldersContaining,
  xcodeContainerArgs,
} from "./utils";

// `./utils` imports the native `@sweetpad/native` addon at module level; stub it so
// this spec runs without the compiled addon. Container discovery and the scheme launch settings
// are the addon's.
const scheme = vi.hoisted(() => ({ launchReferencesSettings: false }));
vi.mock("@sweetpad/native", () => ({
  discoverContainers: vi.fn(),
  parseScheme: vi.fn(() => scheme),
  schemeLaunchSettings: vi.fn(),
}));

vi.mock("../common/cli/scripts", () => ({
  generateBuildServerConfig: vi.fn(),
  generateSweetpadBuildServerConfig: vi.fn(),
  getSweetpadCliPath: vi.fn(),
  findSchemeFile: vi.fn(),
  getBuildSettingsList: vi.fn(),
  getXcodeBuildCommand: vi.fn(() => "xcodebuild"),
  SWEETPAD_CLI_MISSING_MESSAGE: "cli missing",
}));

vi.mock("../common/files", () => ({
  isFileExists: vi.fn(),
  readJsonFile: vi.fn(),
}));

// `./utils` reads `existsSync` to resolve relative configured paths across workspace folders.
vi.mock("node:fs", async (importOriginal) => {
  const original = await importOriginal<typeof import("node:fs")>();
  return { ...original, existsSync: vi.fn(original.existsSync) };
});

const launchOptions = {
  workspaceRoot: "/w",
  xcworkspace: "/w/App.xcodeproj",
  scheme: "App",
  configuration: "Debug",
  sdk: "iphonesimulator",
  destination: "platform=iOS Simulator,id=SIM",
};

// The launch rows are turned into argv and env by the native addon (checked
// against xcodebuild in sweetpad-lib's scheme tests); this side only finds the
// scheme file and resolves build settings when a row refers to one.
describe("getSchemeLaunchSettings", () => {
  beforeEach(() => {
    scheme.launchReferencesSettings = false;
    (findSchemeFile as Mock).mockResolvedValue("/w/App.xcodeproj/xcshareddata/xcschemes/App.xcscheme");
    (sweetpadLib.schemeLaunchSettings as Mock).mockReturnValue({ args: ["-Flag", "a b"], env: { KEY: "value" } });
  });

  it("returns empty settings when the scheme has no file", async () => {
    (findSchemeFile as Mock).mockResolvedValue(undefined);
    expect(await getSchemeLaunchSettings(launchOptions)).toEqual({ args: [], env: {} });
    expect(sweetpadLib.schemeLaunchSettings).not.toHaveBeenCalled();
  });

  it("skips resolving build settings when no row refers to one", async () => {
    expect(await getSchemeLaunchSettings(launchOptions)).toEqual({ args: ["-Flag", "a b"], env: { KEY: "value" } });
    expect(getBuildSettingsList).not.toHaveBeenCalled();
    expect(sweetpadLib.schemeLaunchSettings).toHaveBeenCalledWith(
      "/w/App.xcodeproj/xcshareddata/xcschemes/App.xcscheme",
      [],
    );
  });

  it("passes the resolved build settings when a row refers to one", async () => {
    scheme.launchReferencesSettings = true;
    (getBuildSettingsList as Mock).mockResolvedValue([{ target: "App", settings: { PRODUCT_NAME: "App" } }]);
    await getSchemeLaunchSettings(launchOptions);
    expect(getBuildSettingsList).toHaveBeenCalledWith(launchOptions);
    expect(sweetpadLib.schemeLaunchSettings).toHaveBeenCalledWith(
      "/w/App.xcodeproj/xcshareddata/xcschemes/App.xcscheme",
      [{ target: "App", settings: { PRODUCT_NAME: "App" } }],
    );
  });

  it("launches without the scheme's settings when they can't be read", async () => {
    (sweetpadLib.schemeLaunchSettings as Mock).mockImplementation(() => {
      throw new Error("invalid scheme");
    });
    expect(await getSchemeLaunchSettings(launchOptions)).toEqual({ args: [], env: {} });
  });
});

// `sweetpad.build.args` joins the command the extension assembles for its own builds.
describe("XcodeCommandBuilder.addAdditionalArgs", () => {
  function extensionCommand(): XcodeCommandBuilder {
    const command = new XcodeCommandBuilder();
    command.addBuildSettings("ONLY_ACTIVE_ARCH", "YES");
    command.addParameters("-scheme", "App");
    command.addParameters("-configuration", "Debug");
    command.addParameters("-destination", "platform=macOS,arch=arm64");
    command.addParameters("-derivedDataPath", "/w/dd");
    command.addOption("-allowProvisioningUpdates");
    command.addAction("build");
    return command;
  }

  it("keeps a setting's value whole past its first '='", () => {
    const command = new XcodeCommandBuilder();
    command.addAdditionalArgs(["OTHER_SWIFT_FLAGS=-D A=1", "EMPTY="]);
    expect(command.build()).toEqual(["xcodebuild", "OTHER_SWIFT_FLAGS=-D A=1", "EMPTY="]);
  });

  it("keeps every copy of a flag xcodebuild reads more than once, in order", () => {
    const command = new XcodeCommandBuilder();
    command.addAdditionalArgs([
      "-skip-testing",
      "AppTests/A",
      "-only-testing:AppTests/B",
      "-skip-testing",
      "AppTests/C",
      "-only-testing:AppTests/D",
      "-arch",
      "arm64",
      "-arch",
      "x86_64",
    ]);
    expect(command.build()).toEqual([
      "xcodebuild",
      "-skip-testing",
      "AppTests/A",
      "-only-testing:AppTests/B",
      "-skip-testing",
      "AppTests/C",
      "-only-testing:AppTests/D",
      "-arch",
      "arm64",
      "-arch",
      "x86_64",
    ]);
  });

  it("keeps the last copy of a flag xcodebuild takes once", () => {
    const command = new XcodeCommandBuilder();
    command.addAdditionalArgs(["-jobs", "2", "-quiet", "-jobs", "4", "-quiet"]);
    expect(command.build()).toEqual(["xcodebuild", "-jobs", "4", "-quiet"]);
  });

  it("replaces the extension's own copy of a flag the user gives", () => {
    const command = extensionCommand();
    command.addAdditionalArgs([
      "-derivedDataPath",
      "custom",
      "-destination",
      "platform=iOS Simulator,name=A",
      "-destination",
      "platform=iOS Simulator,name=B",
      "ONLY_ACTIVE_ARCH=NO",
      "-skipMacroValidation",
    ]);
    expect(command.build()).toEqual([
      "xcodebuild",
      "ONLY_ACTIVE_ARCH=NO",
      "-scheme",
      "App",
      "-configuration",
      "Debug",
      "-allowProvisioningUpdates",
      "-derivedDataPath",
      "custom",
      "-destination",
      "platform=iOS Simulator,name=A",
      "-destination",
      "platform=iOS Simulator,name=B",
      "-skipMacroValidation",
      "build",
    ]);
  });

  it("leaves the extension's command alone without user args", () => {
    const command = extensionCommand();
    const before = command.build();
    command.addAdditionalArgs([]);
    expect(command.build()).toEqual(before);
  });

  // xcodebuild reads the argument after a value flag as its value, dashes and all: `-xcconfig -quiet` reads a
  // file named '-quiet'.
  it("reads the argument after a value flag as its value, even one spelled as a flag", () => {
    const command = extensionCommand();
    command.addAdditionalArgs(["-xcconfig", "-derivedDataPath", "-jobs", "-quiet"]);
    expect(command.build()).toEqual([
      "xcodebuild",
      "ONLY_ACTIVE_ARCH=YES",
      "-scheme",
      "App",
      "-configuration",
      "Debug",
      "-destination",
      "platform=macOS,arch=arm64",
      "-derivedDataPath",
      "/w/dd",
      "-allowProvisioningUpdates",
      "-xcconfig",
      "-derivedDataPath",
      "-jobs",
      "-quiet",
      "build",
    ]);
  });

  it("keeps a switch apart from the setting or action after it", () => {
    const command = extensionCommand();
    command.addAdditionalArgs(["-quiet", "ONLY_ACTIVE_ARCH=NO", "-skipMacroValidation", "build"]);
    expect(command.build()).toEqual([
      "xcodebuild",
      "ONLY_ACTIVE_ARCH=NO",
      "-scheme",
      "App",
      "-configuration",
      "Debug",
      "-destination",
      "platform=macOS,arch=arm64",
      "-derivedDataPath",
      "/w/dd",
      "-allowProvisioningUpdates",
      "-quiet",
      "-skipMacroValidation",
      "build",
    ]);
  });

  it("keeps the value of a flag outside the value-flag list", () => {
    const command = new XcodeCommandBuilder();
    command.addAdditionalArgs(["-flagFromNewerXcode", "YES", "-destination", "platform=macOS", "-quiet"]);
    expect(command.build()).toEqual([
      "xcodebuild",
      "-flagFromNewerXcode",
      "YES",
      "-destination",
      "platform=macOS",
      "-quiet",
    ]);
  });

  // A value spelled like a setting or an action is still the flag's: a registry URL with a query, or a
  // directory named `build`.
  it("reads the value of a testing or package flag as its value", () => {
    const command = new XcodeCommandBuilder();
    command.addAdditionalArgs([
      "-enableCodeCoverage",
      "YES",
      "-defaultPackageRegistryURL",
      "https://registry.example.com/?region=eu",
      "-enableCodesizeProfile",
      "YES",
      "-codesizeProfileOutputDir",
      "build",
      "-only-testing",
      "AppTests/Slow",
    ]);
    expect(command.build()).toEqual([
      "xcodebuild",
      "-enableCodeCoverage",
      "YES",
      "-defaultPackageRegistryURL",
      "https://registry.example.com/?region=eu",
      "-enableCodesizeProfile",
      "YES",
      "-codesizeProfileOutputDir",
      "build",
      "-only-testing",
      "AppTests/Slow",
    ]);
  });
});

// The BSP server reads the `buildArgs` in bsp.json with sweetpad-core's `VALUE_FLAGS`, so the index and the
// builds only agree on which argument is a flag's value while the two lists match.
describe("XCODEBUILD_VALUE_FLAGS", () => {
  it("matches sweetpad-core's VALUE_FLAGS", () => {
    const source = readFileSync(path.resolve(__dirname, "../../../sweetpad-core/src/xcodebuild_args.rs"), "utf8");
    const list = source.match(/pub const VALUE_FLAGS: \[&str; \d+\] = \[([^\]]*)\];/);
    expect(list).not.toBeNull();
    const coreFlags = [...(list?.[1] ?? "").matchAll(/"([^"]+)"/g)].map((match) => match[1]);
    expect(coreFlags.length).toBeGreaterThan(0);
    expect([...XCODEBUILD_VALUE_FLAGS].toSorted()).toEqual(coreFlags.toSorted());
  });
});

// Builds, the app locator and the BSP index all read DerivedData through `prepareDerivedDataPath`, so a
// `-derivedDataPath` in `sweetpad.build.args` has to move all of them.
describe("prepareDerivedDataPath", () => {
  const mockGetConfiguration = vscode.workspace.getConfiguration as Mock;

  function mockConfig(values: Record<string, unknown>) {
    mockGetConfiguration.mockReturnValue({
      get: vi.fn((key: string) => values[key]),
    });
  }

  afterEach(() => {
    mockGetConfiguration.mockReset();
  });

  it("leaves the location to xcodebuild when nothing sets it", () => {
    mockConfig({});
    expect(prepareDerivedDataPath({ workspaceRoot: "/w" })).toBeNull();
  });

  it("resolves the setting against the workspace folder", () => {
    mockConfig({ "build.derivedDataPath": ".build/dd" });
    expect(prepareDerivedDataPath({ workspaceRoot: "/w" })).toBe("/w/.build/dd");
  });

  it("takes the last -derivedDataPath in the build args over the setting", () => {
    mockConfig({
      "build.derivedDataPath": "/setting/dd",
      "build.args": ["-derivedDataPath", "dd-a", "-quiet", "-derivedDataPath", "/abs/dd-b", "-derivedDataPath"],
    });
    expect(prepareDerivedDataPath({ workspaceRoot: "/w" })).toBe("/abs/dd-b");
  });

  it("reads the build args' flag values the way xcodebuild does", () => {
    // The '-derivedDataPath' here is the xcconfig file's name.
    mockConfig({ "build.derivedDataPath": "/setting/dd", "build.args": ["-xcconfig", "-derivedDataPath", "dd"] });
    expect(prepareDerivedDataPath({ workspaceRoot: "/w" })).toBe("/setting/dd");

    mockConfig({ "build.derivedDataPath": "/setting/dd", "build.args": ["-derivedDataPath", "-dd"] });
    expect(prepareDerivedDataPath({ workspaceRoot: "/w" })).toBe("/w/-dd");
  });

  it("names the directory the build's own command line does", () => {
    const buildArgs = ["-derivedDataPath", "dd-a", "-derivedDataPath", "dd-b"];
    mockConfig({ "build.derivedDataPath": "/setting/dd", "build.args": buildArgs });
    const derivedDataPath = prepareDerivedDataPath({ workspaceRoot: "/w" });

    const command = new XcodeCommandBuilder();
    command.addParameters("-derivedDataPath", derivedDataPath ?? "");
    command.addAdditionalArgs(buildArgs);
    const parts = command.build();
    const built = parts[parts.lastIndexOf("-derivedDataPath") + 1];
    // xcodebuild runs in the workspace folder, so it reads a relative path against it.
    expect(path.resolve("/w", built)).toBe(derivedDataPath);
    expect(derivedDataPath).toBe("/w/dd-b");
  });

  // xcodebuild reads a relative path against the physical directory it runs in, and the CLI joins the
  // project's standardized directory for that reason. A folder opened through a symlink has to name the same
  // directory the same way.
  it("resolves a relative path against the directory a symlinked folder points at", () => {
    const root = mkdtempSync(path.join(os.tmpdir(), "sweetpad-dd-link-"));
    try {
      mkdirSync(path.join(root, "real", "app"), { recursive: true });
      symlinkSync(path.join(root, "real", "app"), path.join(root, "link"));
      const link = path.join(root, "link");
      const real = path.join(root, "real", "app");

      mockConfig({ "build.derivedDataPath": "dd" });
      const inside = prepareDerivedDataPath({ workspaceRoot: link });
      expect(inside).toBe(prepareDerivedDataPath({ workspaceRoot: real }));
      expect(inside).toMatch(/\/real\/app\/dd$/);

      // `..` leaves the real directory, not the symlink.
      mockConfig({ "build.args": ["-derivedDataPath", "../dd"] });
      const beside = prepareDerivedDataPath({ workspaceRoot: link });
      expect(beside).toBe(prepareDerivedDataPath({ workspaceRoot: real }));
      expect(beside).toMatch(/\/real\/dd$/);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("drops the /private that a root symlink adds", () => {
    mockConfig({ "build.derivedDataPath": "dd" });
    expect(prepareDerivedDataPath({ workspaceRoot: "/private/tmp" })).toBe("/tmp/dd");
    expect(prepareDerivedDataPath({ workspaceRoot: "/tmp" })).toBe("/tmp/dd");
    // An absolute path is the user's spelling.
    mockConfig({ "build.derivedDataPath": "/private/tmp/dd" });
    expect(prepareDerivedDataPath({ workspaceRoot: "/tmp" })).toBe("/private/tmp/dd");
  });
});

describe("Xcode container discovery", () => {
  const mockDiscover = sweetpadLib.discoverContainers as Mock;
  const mockExists = isFileExists as Mock;

  beforeEach(() => {
    vi.clearAllMocks();
    (vscode.workspace as { workspaceFolders?: unknown }).workspaceFolders = [{ uri: { fsPath: "/repo" } }];
  });

  it("addresses a project through its embedded workspace, and a bare one by itself (issue #339)", async () => {
    mockDiscover.mockResolvedValue([
      { path: "/repo/App.xcodeproj", kind: "project", depth: 0 },
      { path: "/repo/Pkg/Package.swift", kind: "package", depth: 1 },
      { path: "/repo/Tool/Tool.xcodeproj", kind: "project", depth: 1 },
    ]);
    mockExists.mockImplementation(async (p: string) => p === "/repo/App.xcodeproj/project.xcworkspace");

    expect(await detectXcodeWorkspacesPaths()).toEqual([
      "/repo/App.xcodeproj/project.xcworkspace",
      "/repo/Pkg/Package.swift",
      "/repo/Tool/Tool.xcodeproj",
    ]);
    // Four levels down, the depth the picker has always searched.
    expect(mockDiscover).toHaveBeenCalledWith("/repo", 4);
  });

  it("offers a project reached from two nested folders once", async () => {
    (vscode.workspace as { workspaceFolders?: unknown }).workspaceFolders = [
      { uri: { fsPath: "/repo" } },
      { uri: { fsPath: "/repo/ios" } },
    ];
    mockDiscover.mockImplementation(async (root: string) =>
      root === "/repo"
        ? [{ path: "/repo/ios/App.xcworkspace", kind: "workspace", depth: 1 }]
        : [{ path: "/repo/ios/App.xcworkspace", kind: "workspace", depth: 0 }],
    );
    mockExists.mockResolvedValue(false);

    expect(await detectXcodeWorkspacesPaths()).toEqual(["/repo/ios/App.xcworkspace"]);
  });

  // The walk puts the nearest container first, and a workspace ahead of the project beside it, so a
  // CocoaPods checkout opens its workspace rather than whichever entry `readdir` listed first.
  it("opens the first container the walk finds in a directory", async () => {
    mockDiscover.mockResolvedValue([
      { path: "/wt/App.xcworkspace", kind: "workspace", depth: 0 },
      { path: "/wt/App.xcodeproj", kind: "project", depth: 0 },
    ]);
    mockExists.mockResolvedValue(true);

    expect(await findXcodeWorkspaceInDirectory("/wt")).toBe("/wt/App.xcworkspace");
    mockDiscover.mockResolvedValue([]);
    expect(await findXcodeWorkspaceInDirectory("/wt")).toBeUndefined();
  });

  it("passes a bare project to xcodebuild as a project", () => {
    expect(xcodeContainerArgs("/repo/Tool.xcodeproj")).toEqual(["-project", "/repo/Tool.xcodeproj"]);
    expect(xcodeContainerArgs("/repo/App.xcodeproj/project.xcworkspace")).toEqual([
      "-workspace",
      "/repo/App.xcodeproj/project.xcworkspace",
    ]);
    expect(xcodeContainerArgs("/repo/App.xcworkspace")).toEqual(["-workspace", "/repo/App.xcworkspace"]);
  });
});

describe("generateBuildServerConfigOnBuild (sweetpad provider)", () => {
  const mockGetConfiguration = vscode.workspace.getConfiguration as Mock;
  const mockGenerate = generateBuildServerConfig as Mock;
  const mockCliPath = getSweetpadCliPath as Mock;
  const mockReadJsonFile = readJsonFile as Mock;
  const mockIsFileExists = isFileExists as Mock;

  beforeEach(() => {
    vi.clearAllMocks();
    (vscode.workspace as { workspaceFolders?: unknown }).workspaceFolders = [{ uri: { fsPath: "/workspace" } }];
    mockGetConfiguration.mockReturnValue({
      get: vi.fn(
        (key: string) =>
          ({
            "buildServer.provider": "sweetpad",
            "build.autoGenerateBuildServerConfig": true,
            // Skip the LSP restart so the test stays on the regeneration logic.
            "build.autoRestartSwiftLSP": false,
          })[key as never],
      ),
      // These cases are about a workspace that asked for our server, so the
      // provider reads as chosen rather than inherited from the manifest —
      // otherwise the resolver would go looking for a config on disk to defer
      // to, which is a different test.
      inspect: vi.fn((key: string) =>
        key === "buildServer.provider" ? { defaultValue: "sweetpad", workspaceValue: "sweetpad" } : undefined,
      ),
    });
    mockCliPath.mockResolvedValue("/opt/homebrew/bin/sweetpad");
  });

  function run() {
    return generateBuildServerConfigOnBuild({
      workspaceRoot: "/workspace",
      scheme: "App",
      xcworkspace: "/workspace/App.xcworkspace",
      workspaceState: { get: vi.fn(), update: vi.fn() } as unknown as WorkspaceStateService,
    });
  }

  it("skips regeneration when the config already names the installed CLI", async () => {
    mockReadJsonFile.mockResolvedValue({
      name: "sweetpad",
      argv: ["/opt/homebrew/bin/sweetpad", "bsp", "serve", "--config", getBspConfigFile("/workspace")],
    });
    await run();
    expect(mockGenerate).not.toHaveBeenCalled();
  });

  it("leaves a standalone config alone even though it shares our name", async () => {
    mockReadJsonFile.mockResolvedValue({
      name: "sweetpad",
      argv: ["/opt/homebrew/bin/sweetpad", "bsp", "serve", "--project", "/workspace/App.xcodeproj"],
    });
    mockIsFileExists.mockResolvedValue(true);

    await run();

    // `sweetpad bsp init` writes our name too, so only the absence of
    // `--config` marks this as a setup the workspace made rather than one we
    // maintain. Reading it as ours would overwrite it on the next build.
    expect(mockGenerate).not.toHaveBeenCalled();
  });

  it("migrates a config an older extension wrote to run its own launcher", async () => {
    mockReadJsonFile.mockResolvedValue({
      name: "sweetpad",
      argv: ["/old-ext/out/bsp-server.js", "--config", getBspConfigFile("/workspace")],
    });
    await run();
    expect(mockGenerate).toHaveBeenCalledTimes(1);
  });

  it("regenerates when the CLI has moved since the config was written", async () => {
    mockReadJsonFile.mockResolvedValue({
      name: "sweetpad",
      argv: ["/usr/local/bin/sweetpad", "bsp", "serve", "--config", getBspConfigFile("/workspace")],
    });
    mockIsFileExists.mockResolvedValue(true);
    await run();
    expect(mockGenerate).toHaveBeenCalledTimes(1);
  });

  it("writes nothing and says so once when the CLI is not installed", async () => {
    mockCliPath.mockResolvedValue(undefined);
    mockReadJsonFile.mockResolvedValue({
      name: "sweetpad",
      argv: ["/old-ext/out/bsp-server.js", "--config", getBspConfigFile("/workspace")],
    });
    mockIsFileExists.mockResolvedValue(false);

    await run();

    // Pointing buildServer.json at a launcher that isn't there would leave
    // sourcekit-lsp failing to spawn it with nothing said about why.
    expect(mockGenerate).not.toHaveBeenCalled();
    expect(vscode.window.showWarningMessage).toHaveBeenCalledTimes(1);
  });

  it("leaves a config the sweetpad CLI wrote alone", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "sweetpad-lib", argv: ["/opt/homebrew/bin/sweetpad", "bsp", "serve"] });
    mockIsFileExists.mockResolvedValue(true);

    await run();

    // `sweetpad bsp init` reaches the same server through the CLI binary. That
    // launcher is the one the workspace set up, so swapping ours in would move
    // the project onto something it never asked for.
    expect(mockGenerate).not.toHaveBeenCalled();
  });

  it("replaces a CLI config whose binary has gone", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "sweetpad-lib", argv: ["/opt/homebrew/bin/sweetpad", "bsp", "serve"] });
    mockIsFileExists.mockResolvedValue(false);

    await run();

    // Nothing can start from here, so this is a repair rather than a takeover.
    expect(mockGenerate).toHaveBeenCalledTimes(1);
  });

  it("regenerates when switching in from another provider's config", async () => {
    mockReadJsonFile.mockResolvedValue({
      name: "xcode build server",
      argv: ["/opt/homebrew/bin/xcode-build-server"],
    });
    mockIsFileExists.mockResolvedValue(true);
    await run();
    expect(mockGenerate).toHaveBeenCalledTimes(1);
  });

  it("regenerates when buildServer.json is missing or unreadable", async () => {
    mockReadJsonFile.mockRejectedValue(new Error("ENOENT"));
    await run();
    expect(mockGenerate).toHaveBeenCalledTimes(1);
  });
});

describe("repairStaleBuildServerConfig", () => {
  const mockGetConfiguration = vscode.workspace.getConfiguration as Mock;
  const mockRepair = generateSweetpadBuildServerConfig as Mock;
  const mockReadJsonFile = readJsonFile as Mock;
  const mockIsFileExists = isFileExists as Mock;
  const state = { get: vi.fn(), update: vi.fn() } as unknown as WorkspaceStateService;
  const stateRemembering = (xcworkspace: string) =>
    ({
      get: vi.fn((key: string) => (key === "build.xcodeWorkspacePath" ? xcworkspace : undefined)),
      update: vi.fn(),
    }) as unknown as WorkspaceStateService;

  beforeEach(() => {
    vi.clearAllMocks();
    (vscode.workspace as { workspaceFolders?: unknown }).workspaceFolders = [{ uri: { fsPath: "/workspace" } }];
    mockGetConfiguration.mockReturnValue({
      get: vi.fn((key: string) => ({ "buildServer.provider": "sweetpad" })[key as never]),
      inspect: vi.fn((key: string) =>
        key === "buildServer.provider" ? { defaultValue: "sweetpad", workspaceValue: "sweetpad" } : undefined,
      ),
    });
  });

  it("rewrites a config of ours whose launcher an update deleted", async () => {
    mockReadJsonFile.mockResolvedValue({
      name: "sweetpad",
      argv: ["/old-ext/out/bsp-server.js", "--config", getBspConfigFile("/workspace")],
    });
    mockIsFileExists.mockResolvedValue(false);

    await repairStaleBuildServerConfig({ workspaceRoot: "/workspace", workspaceState: state });

    expect(mockRepair).toHaveBeenCalledTimes(1);
  });

  it("leaves a launcher that still resolves alone", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "sweetpad", argv: ["/usr/local/bin/sweetpad", "bsp", "serve"] });
    mockIsFileExists.mockResolvedValue(true);

    // A path that exists is one somebody meant to point at, even where it isn't
    // the launcher this version ships. Only dangling ones are repaired.
    await repairStaleBuildServerConfig({ workspaceRoot: "/workspace", workspaceState: state });

    expect(mockRepair).not.toHaveBeenCalled();
  });

  it("does not touch another server's config", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "xcode build server", argv: ["/gone/xcode-build-server"] });
    mockIsFileExists.mockResolvedValue(false);

    await repairStaleBuildServerConfig({ workspaceRoot: "/workspace", workspaceState: state });

    expect(mockRepair).not.toHaveBeenCalled();
  });

  it("writes nothing when there is no config to repair", async () => {
    mockReadJsonFile.mockRejectedValue(new Error("ENOENT"));

    // Creating one from nothing belongs to the build, which knows the project.
    await repairStaleBuildServerConfig({ workspaceRoot: "/workspace", workspaceState: state });

    expect(mockRepair).not.toHaveBeenCalled();
  });

  it("names the remembered project so a missing bsp.json is filled in", async () => {
    mockReadJsonFile.mockResolvedValue({
      name: "sweetpad",
      argv: ["/old-ext/out/bsp-server.js", "--config", getBspConfigFile("/workspace")],
    });
    mockIsFileExists.mockResolvedValue(false);

    await repairStaleBuildServerConfig({
      workspaceRoot: "/workspace",
      workspaceState: stateRemembering("/workspace/App.xcworkspace"),
    });

    expect(mockRepair).toHaveBeenCalledWith(expect.objectContaining({ xcworkspace: "/workspace/App.xcworkspace" }));
  });

  it("names no project when the remembered one is a Swift package", async () => {
    mockReadJsonFile.mockResolvedValue({
      name: "sweetpad",
      argv: ["/old-ext/out/bsp-server.js", "--config", getBspConfigFile("/workspace")],
    });
    mockIsFileExists.mockResolvedValue(false);

    // Packages go to sourcekit-lsp's own SwiftPM support, so a bsp.json naming
    // Package.swift points the BSP server at a project it cannot open. The
    // launcher is still rewritten.
    await repairStaleBuildServerConfig({
      workspaceRoot: "/workspace",
      workspaceState: stateRemembering("/workspace/Package.swift"),
    });

    expect(mockRepair).toHaveBeenCalledWith(expect.objectContaining({ xcworkspace: undefined }));
  });

  it("logs a failed rewrite instead of rejecting", async () => {
    mockReadJsonFile.mockResolvedValue({
      name: "sweetpad",
      argv: ["/old-ext/out/bsp-server.js", "--config", getBspConfigFile("/workspace")],
    });
    mockIsFileExists.mockResolvedValue(false);
    mockRepair.mockRejectedValueOnce(new Error("EROFS"));

    // Activation fires this without awaiting it, so a rejection has nobody to
    // catch it.
    await expect(
      repairStaleBuildServerConfig({ workspaceRoot: "/workspace", workspaceState: state }),
    ).resolves.toBeUndefined();
  });
});

describe("multi-root workspace path resolution", () => {
  const mockGetConfiguration = vscode.workspace.getConfiguration as Mock;
  let workspaceContext: WorkspaceContextService;
  const mockExistsSync = existsSync as Mock;

  function setFolders(paths: string[]) {
    (vscode.workspace as { workspaceFolders?: unknown }).workspaceFolders = paths.map((p) => ({
      uri: { fsPath: p },
    }));
  }

  // Invoke the listener `WorkspaceContextService.start` handed to VS Code, standing in for the
  // host firing onDidChangeWorkspaceFolders.
  function fireWorkspaceFoldersChanged() {
    const register = vscode.workspace.onDidChangeWorkspaceFolders as Mock;
    for (const [listener] of register.mock.calls) {
      (listener as () => void)();
    }
  }

  function mockConfig(values: Record<string, unknown>) {
    mockGetConfiguration.mockReturnValue({
      get: vi.fn((key: string) => values[key]),
    });
  }

  beforeEach(() => {
    vi.clearAllMocks();
    mockConfig({});
    mockExistsSync.mockReturnValue(false);
    // A fresh context per case, so the selection cannot leak between them. Every case below
    // reuses the same two folder names, which only holds because of this.
    workspaceContext = new WorkspaceContextService();
  });

  it("defaults to the first workspace folder before any project is selected", () => {
    setFolders(["/root-1", "/root-2"]);
    expect(workspaceContext.root).toBe("/root-1");
  });

  it("follows the folder containing the selected xcworkspace", () => {
    setFolders(["/root-1", "/root-2"]);
    workspaceContext.setActiveFolder("/root-2/App/App.xcworkspace");
    expect(workspaceContext.root).toBe("/root-2");
  });

  it("picks the innermost folder when workspace folders nest", () => {
    setFolders(["/root-1", "/root-1/ios"]);
    workspaceContext.setActiveFolder("/root-1/ios/App.xcworkspace");
    expect(workspaceContext.root).toBe("/root-1/ios");
  });

  it("keeps the current folder when the xcworkspace is outside every workspace folder", () => {
    setFolders(["/root-1", "/root-2"]);
    workspaceContext.setActiveFolder("/root-2/App.xcworkspace");
    // e.g. a git worktree next to the repo
    workspaceContext.setActiveFolder("/elsewhere/App.xcworkspace");
    expect(workspaceContext.root).toBe("/root-2");
  });

  it("falls back to the first folder when the active folder leaves the workspace", () => {
    setFolders(["/root-1", "/root-2"]);
    workspaceContext.setActiveFolder("/root-2/App.xcworkspace");
    setFolders(["/root-3"]);
    expect(workspaceContext.root).toBe("/root-3");
  });

  it("activates the folder of a cached xcworkspace from workspace state", () => {
    setFolders(["/root-1", "/root-2"]);
    const state = {
      get: vi.fn(() => "/root-2/App.xcworkspace"),
      update: vi.fn(),
    } as unknown as WorkspaceStateService;

    expect(activateCurrentXcodeWorkspacePath({ workspaceState: state, workspaceContext: workspaceContext })).toBe(
      "/root-2/App.xcworkspace",
    );
    expect(workspaceContext.root).toBe("/root-2");
  });

  it("resolves a relative configured path against the folder where it exists", () => {
    setFolders(["/root-1", "/root-2"]);
    mockConfig({ "build.xcodeWorkspacePath": "App.xcworkspace" });
    mockExistsSync.mockImplementation((p: string) => p === "/root-2/App.xcworkspace");
    const state = { get: vi.fn(), update: vi.fn() } as unknown as WorkspaceStateService;

    expect(activateCurrentXcodeWorkspacePath({ workspaceState: state, workspaceContext: workspaceContext })).toBe(
      "/root-2/App.xcworkspace",
    );
    expect(workspaceContext.root).toBe("/root-2");
  });

  // Reporting what is selected must not decide what is selected: `workspace detect`, the doctor and
  // the worktree picker all ask this only to display it.
  it("reports the selection without moving the active folder or clearing the cache", () => {
    setFolders(["/root-1", "/root-2"]);
    mockConfig({ "build.xcodeWorkspacePath": "App.xcworkspace" });
    mockExistsSync.mockImplementation((p: string) => p === "/root-2/App.xcworkspace");
    const state = { get: vi.fn(), update: vi.fn() } as unknown as WorkspaceStateService;

    expect(getCurrentXcodeWorkspacePath(state)).toBe("/root-2/App.xcworkspace");
    expect(workspaceContext.root).toBe("/root-1");
    expect(state.update).not.toHaveBeenCalled();
  });

  // `getWorkspaceRelativePath` anchors what it stores to the first folder, so a *bare* relative
  // path names a project in that folder and nowhere else. Resolving it against the active folder
  // instead would hand back the wrong one of two checkouts as soon as the active folder moved.
  it("resolves a bare relative path against the first folder, not the active one", () => {
    setFolders(["/root-1", "/root-2"]);
    mockConfig({ "build.xcodeWorkspacePath": "App.xcworkspace" });
    mockExistsSync.mockImplementation(
      (p: string) => p === "/root-1/App.xcworkspace" || p === "/root-2/App.xcworkspace",
    );
    const state = { get: vi.fn(), update: vi.fn() } as unknown as WorkspaceStateService;
    workspaceContext.setActiveFolder("/root-2/App.xcworkspace");

    expect(getCurrentXcodeWorkspacePath(state)).toBe("/root-1/App.xcworkspace");
    expect(activateCurrentXcodeWorkspacePath({ workspaceState: state, workspaceContext: workspaceContext })).toBe(
      "/root-1/App.xcworkspace",
    );
    expect(workspaceContext.root).toBe("/root-1");
  });

  // The other half of that contract: a project outside the first folder is stored with the
  // "../root-2/" prefix precisely so the reader lands on exactly one file.
  it("round-trips a path the writer anchored past the first folder", () => {
    setFolders(["/root-1", "/root-2"]);
    mockConfig({ "build.xcodeWorkspacePath": "../root-2/App.xcworkspace" });
    mockExistsSync.mockImplementation((p: string) => p === "/root-2/App.xcworkspace");
    const state = { get: vi.fn(), update: vi.fn() } as unknown as WorkspaceStateService;

    expect(activateCurrentXcodeWorkspacePath({ workspaceState: state, workspaceContext: workspaceContext })).toBe(
      "/root-2/App.xcworkspace",
    );
    expect(workspaceContext.root).toBe("/root-2");
  });

  it("activates the folder of an absolute configured path", () => {
    setFolders(["/root-1", "/root-2"]);
    mockConfig({ "build.xcodeWorkspacePath": "/root-2/App.xcworkspace" });
    const state = { get: vi.fn(), update: vi.fn() } as unknown as WorkspaceStateService;

    expect(activateCurrentXcodeWorkspacePath({ workspaceState: state, workspaceContext: workspaceContext })).toBe(
      "/root-2/App.xcworkspace",
    );
    expect(workspaceContext.root).toBe("/root-2");
  });

  // Long-lived state derived from the active folder — a BSP socket, a registry key — is only right
  // for the folder it was built from, so subscribers need to hear about every move exactly once.
  describe("WorkspaceContextService.onDidChange", () => {
    it("fires with the new folder when the active project moves", () => {
      setFolders(["/root-1", "/root-2"]);
      const seen: string[] = [];
      workspaceContext.onDidChange((folder) => seen.push(folder));

      workspaceContext.setActiveFolder("/root-2/App.xcworkspace");

      expect(seen).toEqual(["/root-2"]);
    });

    it("stays quiet when the folder does not actually change", () => {
      setFolders(["/root-1", "/root-2"]);
      workspaceContext.setActiveFolder("/root-2/App.xcworkspace");
      const seen: string[] = [];
      workspaceContext.onDidChange((folder) => seen.push(folder));

      // A second project in the same folder, and one outside every folder, both leave it put.
      workspaceContext.setActiveFolder("/root-2/Other/Other.xcworkspace");
      workspaceContext.setActiveFolder("/elsewhere/App.xcworkspace");

      expect(seen).toEqual([]);
    });

    // Removing the folder that holds the current project moves the resolved root back to the first
    // folder without any call to setActiveWorkspaceFolder, so the folder list is a second input
    // subscribers have to hear about.
    it("fires when the active folder leaves the workspace", () => {
      setFolders(["/root-1", "/root-2"]);
      workspaceContext.setActiveFolder("/root-2/App.xcworkspace");
      workspaceContext.start();
      const seen: string[] = [];
      workspaceContext.onDidChange((folder) => seen.push(folder));

      setFolders(["/root-1"]);
      fireWorkspaceFoldersChanged();

      expect(seen).toEqual(["/root-1"]);
      expect(workspaceContext.root).toBe("/root-1");
      workspaceContext.dispose();
    });

    it("stays quiet when the folder list changes without moving the resolved root", () => {
      setFolders(["/root-1", "/root-2"]);
      workspaceContext.setActiveFolder("/root-2/App.xcworkspace");
      workspaceContext.start();
      const seen: string[] = [];
      workspaceContext.onDidChange((folder) => seen.push(folder));

      // A third folder joins; the project's folder is untouched, so the root stays put.
      setFolders(["/root-1", "/root-2", "/root-3"]);
      fireWorkspaceFoldersChanged();

      expect(seen).toEqual([]);
      workspaceContext.dispose();
    });

    it("stops delivering once the subscription is disposed", () => {
      setFolders(["/root-1", "/root-2"]);
      const seen: string[] = [];
      const subscription = workspaceContext.onDidChange((folder) => seen.push(folder));

      subscription.dispose();
      workspaceContext.setActiveFolder("/root-2/App.xcworkspace");

      expect(seen).toEqual([]);
    });
  });

  // The generators run in the folder holding their spec, so the folder itself is the answer these
  // callers need — a plain "does one of them have it" would send the generate to the wrong place.
  describe("workspaceFoldersContaining", () => {
    const mockIsFileExists = isFileExists as Mock;

    async function rootsContaining(...fileNames: string[]) {
      const folders = await workspaceFoldersContaining(...fileNames);
      return folders.map((folder) => folder.uri.fsPath);
    }

    it("returns only the folders holding the file, in workspace folder order", async () => {
      setFolders(["/root-1", "/root-2", "/root-3"]);
      mockIsFileExists.mockImplementation(async (p: string) => p !== "/root-2/project.yml");

      expect(await rootsContaining("project.yml")).toEqual(["/root-1", "/root-3"]);
    });

    it("matches a folder holding any one of several files", async () => {
      setFolders(["/root-1", "/root-2"]);
      mockIsFileExists.mockImplementation(async (p: string) => p === "/root-2/Workspace.swift");

      expect(await rootsContaining("Project.swift", "Workspace.swift")).toEqual(["/root-2"]);
    });

    it("returns nothing when no folder holds the file", async () => {
      setFolders(["/root-1", "/root-2"]);
      mockIsFileExists.mockResolvedValue(false);

      expect(await rootsContaining("project.yml")).toEqual([]);
    });
  });

  // Two folders holding the same layout — two checkouts of one repo — are the case where a bare
  // relative path names both. `getWorkspaceRelativePath` anchors to the first folder so the stored
  // value keeps its "../" prefix; this is the read half of that contract.
  it("resolves a folder-prefixed relative path even when the first folder also matches", () => {
    setFolders(["/root-1", "/root-2"]);
    mockConfig({ "build.xcodeWorkspacePath": "../root-2/App.xcworkspace" });
    mockExistsSync.mockImplementation(
      (p: string) => p === "/root-1/App.xcworkspace" || p === "/root-2/App.xcworkspace",
    );
    const state = { get: vi.fn(), update: vi.fn() } as unknown as WorkspaceStateService;

    expect(activateCurrentXcodeWorkspacePath({ workspaceState: state, workspaceContext: workspaceContext })).toBe(
      "/root-2/App.xcworkspace",
    );
    expect(workspaceContext.root).toBe("/root-2");
  });
});
