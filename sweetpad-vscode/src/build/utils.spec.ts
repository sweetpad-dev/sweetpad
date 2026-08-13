import { existsSync } from "node:fs";

import type { Mock } from "vitest";
import * as vscode from "vscode";

import { generateBuildServerConfig, getSweetpadBspServerPath } from "../common/cli/scripts";
import { isFileExists, readJsonFile } from "../common/files";
import type { WorkspaceStateService } from "../common/workspace-state";
import {
  generateBuildServerConfigOnBuild,
  getCurrentXcodeWorkspacePath,
  getWorkspacePath,
  launchActionToSettings,
  setActiveWorkspaceFolder,
} from "./utils";

// `./utils` imports the native `@sweetpad/native` addon at module level; stub it so
// this spec runs without the compiled addon (none of the tested paths touch it).
vi.mock("@sweetpad/native", () => ({}));

vi.mock("../common/cli/scripts", () => ({
  generateBuildServerConfig: vi.fn(),
  getSweetpadBspServerPath: vi.fn(),
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
  const mockBspServerPath = getSweetpadBspServerPath as Mock;
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
    });
    mockBspServerPath.mockReturnValue("/ext/out/bsp-server.js");
  });

  function run() {
    return generateBuildServerConfigOnBuild({
      scheme: "App",
      xcworkspace: "/workspace/App.xcworkspace",
      workspaceState: {} as unknown as WorkspaceStateService,
    });
  }

  it("skips regeneration when the config is ours and the launcher path is current", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "sweetpad", argv: ["/ext/out/bsp-server.js"] });
    mockIsFileExists.mockResolvedValue(true);
    await run();
    expect(mockGenerate).not.toHaveBeenCalled();
  });

  it("regenerates when argv[0] points into a stale (old extension version) dir", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "sweetpad", argv: ["/old-ext/out/bsp-server.js"] });
    mockIsFileExists.mockResolvedValue(true);
    await run();
    expect(mockGenerate).toHaveBeenCalledTimes(1);
  });

  it("regenerates when argv[0] no longer exists on disk", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "sweetpad", argv: ["/ext/out/bsp-server.js"] });
    mockIsFileExists.mockResolvedValue(false);
    await run();
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

describe("multi-root workspace path resolution", () => {
  const mockGetConfiguration = vscode.workspace.getConfiguration as Mock;
  const mockExistsSync = existsSync as Mock;

  function setFolders(paths: string[]) {
    (vscode.workspace as { workspaceFolders?: unknown }).workspaceFolders = paths.map((p) => ({
      uri: { fsPath: p },
    }));
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
  });

  it("defaults to the first workspace folder before any project is selected", () => {
    setFolders(["/root-a", "/root-b"]);
    expect(getWorkspacePath()).toBe("/root-a");
  });

  it("follows the folder containing the selected xcworkspace", () => {
    setFolders(["/root-c", "/root-d"]);
    setActiveWorkspaceFolder("/root-d/App/App.xcworkspace");
    expect(getWorkspacePath()).toBe("/root-d");
  });

  it("keeps the current folder when the xcworkspace is outside every workspace folder", () => {
    setFolders(["/root-e", "/root-f"]);
    setActiveWorkspaceFolder("/root-f/App.xcworkspace");
    // e.g. a git worktree next to the repo
    setActiveWorkspaceFolder("/elsewhere/App.xcworkspace");
    expect(getWorkspacePath()).toBe("/root-f");
  });

  it("falls back to the first folder when the active folder leaves the workspace", () => {
    setFolders(["/root-g", "/root-h"]);
    setActiveWorkspaceFolder("/root-h/App.xcworkspace");
    setFolders(["/root-i"]);
    expect(getWorkspacePath()).toBe("/root-i");
  });

  it("activates the folder of a cached xcworkspace from workspace state", () => {
    setFolders(["/root-j", "/root-k"]);
    const state = {
      get: vi.fn(() => "/root-k/App.xcworkspace"),
      update: vi.fn(),
    } as unknown as WorkspaceStateService;

    expect(getCurrentXcodeWorkspacePath(state)).toBe("/root-k/App.xcworkspace");
    expect(getWorkspacePath()).toBe("/root-k");
  });

  it("resolves a relative configured path against the folder where it exists", () => {
    setFolders(["/root-l", "/root-m"]);
    mockConfig({ "build.xcodeWorkspacePath": "App.xcworkspace" });
    mockExistsSync.mockImplementation((p: string) => p === "/root-m/App.xcworkspace");
    const state = { get: vi.fn(), update: vi.fn() } as unknown as WorkspaceStateService;

    expect(getCurrentXcodeWorkspacePath(state)).toBe("/root-m/App.xcworkspace");
    expect(getWorkspacePath()).toBe("/root-m");
  });

  it("activates the folder of an absolute configured path", () => {
    setFolders(["/root-n", "/root-o"]);
    mockConfig({ "build.xcodeWorkspacePath": "/root-o/App.xcworkspace" });
    const state = { get: vi.fn(), update: vi.fn() } as unknown as WorkspaceStateService;

    expect(getCurrentXcodeWorkspacePath(state)).toBe("/root-o/App.xcworkspace");
    expect(getWorkspacePath()).toBe("/root-o");
  });
});
