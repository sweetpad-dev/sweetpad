import { existsSync } from "node:fs";

import type { Mock } from "vitest";
import * as vscode from "vscode";

import { getBspConfigFile } from "../bsp/paths";
import {
  generateBuildServerConfig,
  generateSweetpadBuildServerConfig,
  getSweetpadCliPath,
} from "../common/cli/scripts";
import { isFileExists, readJsonFile } from "../common/files";
import { WorkspaceContextService } from "../common/workspace-context";
import type { WorkspaceStateService } from "../common/workspace-state";
import {
  activateCurrentXcodeWorkspacePath,
  generateBuildServerConfigOnBuild,
  getCurrentXcodeWorkspacePath,
  launchActionToSettings,
  repairStaleBuildServerConfig,
  workspaceFoldersContaining,
} from "./utils";

// `./utils` imports the native `@sweetpad/native` addon at module level; stub it so
// this spec runs without the compiled addon (none of the tested paths touch it).
vi.mock("@sweetpad/native", () => ({}));

vi.mock("../common/cli/scripts", () => ({
  generateBuildServerConfig: vi.fn(),
  generateSweetpadBuildServerConfig: vi.fn(),
  getSweetpadCliPath: vi.fn(),
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

type ArgInput = { argument: string; isEnabled?: boolean };
type EnvInput = { key: string; value?: string; isEnabled?: boolean };

// Build the subset of a parsed scheme (`sweetpadLib.SchemeInfo`) that
// `launchActionToSettings` reads, defaulting each row to enabled.
function launch(over: { args?: ArgInput[]; env?: EnvInput[]; language?: string; region?: string }) {
  return {
    launchArguments: (over.args ?? []).map((a) => ({ argument: a.argument, isEnabled: a.isEnabled ?? true })),
    launchEnvironmentVariables: (over.env ?? []).map((e) => ({
      key: e.key,
      value: e.value,
      isEnabled: e.isEnabled ?? true,
    })),
    launchLanguage: over.language,
    launchRegion: over.region,
  };
}

describe("launchActionToSettings", () => {
  it("returns empty settings for a bare launch action", () => {
    expect(launchActionToSettings(launch({}))).toEqual({ args: [], env: {} });
  });

  it("auto-injects -AppleLanguages and -AppleLocale from language + region", () => {
    expect(launchActionToSettings(launch({ language: "he", region: "IL" })).args).toEqual([
      "-AppleLanguages",
      "(he)",
      "-AppleLocale",
      "he_IL",
    ]);
  });

  it("emits -AppleLanguages but not -AppleLocale when only language is set", () => {
    expect(launchActionToSettings(launch({ language: "ar" })).args).toEqual(["-AppleLanguages", "(ar)"]);
  });

  it("emits no locale flags when only region is set (bare region is not a valid locale id)", () => {
    expect(launchActionToSettings(launch({ region: "JP" })).args).toEqual([]);
  });

  it("tokenizes command-line argument rows on whitespace", () => {
    expect(
      launchActionToSettings(launch({ args: [{ argument: "-AppleLanguages (he)" }, { argument: "--flag" }] })).args,
    ).toEqual(["-AppleLanguages", "(he)", "--flag"]);
  });

  it("skips disabled command-line arguments", () => {
    expect(
      launchActionToSettings(launch({ args: [{ argument: "--keep" }, { argument: "--skip", isEnabled: false }] })).args,
    ).toEqual(["--keep"]);
  });

  it("collects enabled environment variables and drops disabled ones", () => {
    expect(
      launchActionToSettings(
        launch({
          env: [
            { key: "KEEP", value: "1" },
            { key: "SKIP", value: "x", isEnabled: false },
          ],
        }),
      ).env,
    ).toEqual({ KEEP: "1" });
  });

  it("drops environment variables with no value (distinct from empty string)", () => {
    expect(launchActionToSettings(launch({ env: [{ key: "NOVALUE" }, { key: "EMPTY", value: "" }] })).env).toEqual({
      EMPTY: "",
    });
  });

  it("keeps both explicit locale args and language/region attrs (discussion #197)", () => {
    const { args } = launchActionToSettings(
      launch({
        args: [
          { argument: "-AppleLanguages (he)" },
          { argument: "-AppleLocale he_IL" },
          { argument: "-WMFVisualTestBatchRecordMode" },
        ],
        language: "he",
        region: "IL",
      }),
    );
    // The explicit CLI args and the language/region attrs both flow through;
    // Foundation reads the first match at launch.
    expect(args).toContain("-WMFVisualTestBatchRecordMode");
    expect(args.filter((a) => a === "-AppleLanguages")).toHaveLength(2);
    expect(args.filter((a) => a === "-AppleLocale")).toHaveLength(2);
    expect(args).toContain("(he)");
    expect(args).toContain("he_IL");
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

    await repairStaleBuildServerConfig({ workspaceRoot: "/workspace" });

    expect(mockRepair).toHaveBeenCalledTimes(1);
  });

  it("leaves a launcher that still resolves alone", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "sweetpad", argv: ["/usr/local/bin/sweetpad", "bsp", "serve"] });
    mockIsFileExists.mockResolvedValue(true);

    // A path that exists is one somebody meant to point at, even where it isn't
    // the launcher this version ships. Only dangling ones are repaired.
    await repairStaleBuildServerConfig({ workspaceRoot: "/workspace" });

    expect(mockRepair).not.toHaveBeenCalled();
  });

  it("does not touch another server's config", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "xcode build server", argv: ["/gone/xcode-build-server"] });
    mockIsFileExists.mockResolvedValue(false);

    await repairStaleBuildServerConfig({ workspaceRoot: "/workspace" });

    expect(mockRepair).not.toHaveBeenCalled();
  });

  it("writes nothing when there is no config to repair", async () => {
    mockReadJsonFile.mockRejectedValue(new Error("ENOENT"));

    // Creating one from nothing belongs to the build, which knows the project.
    await repairStaleBuildServerConfig({ workspaceRoot: "/workspace" });

    expect(mockRepair).not.toHaveBeenCalled();
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
