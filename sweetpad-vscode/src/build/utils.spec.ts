import type { Mock } from "vitest";
import * as vscode from "vscode";

import {
  generateBuildServerConfig,
  generateSweetpadBuildServerConfig,
  getSweetpadCliPath,
} from "../common/cli/scripts";
import { isFileExists, readJsonFile } from "../common/files";
import type { WorkspaceStateService } from "../common/workspace-state";
import { generateBuildServerConfigOnBuild, launchActionToSettings, repairStaleBuildServerConfig } from "./utils";

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
      scheme: "App",
      xcworkspace: "/workspace/App.xcworkspace",
      workspaceState: { get: vi.fn(), update: vi.fn() } as unknown as WorkspaceStateService,
    });
  }

  it("skips regeneration when the config already names the installed CLI", async () => {
    mockReadJsonFile.mockResolvedValue({
      name: "sweetpad",
      argv: ["/opt/homebrew/bin/sweetpad", "bsp", "serve", "--config", "/state/bsp.json"],
    });
    await run();
    expect(mockGenerate).not.toHaveBeenCalled();
  });

  it("migrates a config an older extension wrote to run its own launcher", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "sweetpad", argv: ["/old-ext/out/bsp-server.js", "--config", "/x"] });
    await run();
    expect(mockGenerate).toHaveBeenCalledTimes(1);
  });

  it("regenerates when the CLI has moved since the config was written", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "sweetpad", argv: ["/usr/local/bin/sweetpad", "bsp", "serve"] });
    await run();
    expect(mockGenerate).toHaveBeenCalledTimes(1);
  });

  it("writes nothing and says so once when the CLI is not installed", async () => {
    mockCliPath.mockResolvedValue(undefined);
    mockReadJsonFile.mockResolvedValue({ name: "sweetpad", argv: ["/old-ext/out/bsp-server.js"] });

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
    mockReadJsonFile.mockResolvedValue({ name: "sweetpad", argv: ["/old-ext/out/bsp-server.js"] });
    mockIsFileExists.mockResolvedValue(false);

    await repairStaleBuildServerConfig();

    expect(mockRepair).toHaveBeenCalledTimes(1);
  });

  it("leaves a launcher that still resolves alone", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "sweetpad", argv: ["/usr/local/bin/sweetpad", "bsp", "serve"] });
    mockIsFileExists.mockResolvedValue(true);

    // A path that exists is one somebody meant to point at, even where it isn't
    // the launcher this version ships. Only dangling ones are repaired.
    await repairStaleBuildServerConfig();

    expect(mockRepair).not.toHaveBeenCalled();
  });

  it("does not touch another server's config", async () => {
    mockReadJsonFile.mockResolvedValue({ name: "xcode build server", argv: ["/gone/xcode-build-server"] });
    mockIsFileExists.mockResolvedValue(false);

    await repairStaleBuildServerConfig();

    expect(mockRepair).not.toHaveBeenCalled();
  });

  it("writes nothing when there is no config to repair", async () => {
    mockReadJsonFile.mockRejectedValue(new Error("ENOENT"));

    // Creating one from nothing belongs to the build, which knows the project.
    await repairStaleBuildServerConfig();

    expect(mockRepair).not.toHaveBeenCalled();
  });
});
