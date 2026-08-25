import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import type { Mock } from "vitest";

import { getWorkspaceFolderPaths } from "../build/utils";
import { getWorkspaceConfig, isWorkspaceConfigSetByUser } from "../common/config";
import {
  getBuildServerProvider,
  isPreservingForeignBuildServer,
  isSweetpadBuildServerActive,
  preparedCheck,
} from "./commands";
import { getBspConfigFile } from "./paths";
import type { BspStatusSnapshot } from "./service";

/** argv for the config the extension maintains: project comes from bsp.json. */
const managedArgv = () => ["/opt/homebrew/bin/sweetpad", "bsp", "serve", "--config", getBspConfigFile(workspace)];

/** argv `sweetpad bsp init` writes: project fixed on the command line. */
const standaloneArgv = () => ["/opt/homebrew/bin/sweetpad", "bsp", "serve", "--project", "/w/App.xcodeproj"];

vi.mock("../common/config", () => ({
  getWorkspaceConfig: vi.fn(),
  isWorkspaceConfigSetByUser: vi.fn(),
  updateWorkspaceConfig: vi.fn(),
}));
vi.mock("../build/utils", () => ({ getWorkspaceFolderPaths: vi.fn() }));

const mockConfig = getWorkspaceConfig as Mock;
const mockSetByUser = isWorkspaceConfigSetByUser as Mock;
const mockWorkspaceFolders = getWorkspaceFolderPaths as Mock;

/**
 * Nobody has chosen a provider. VS Code still hands back the default this
 * extension contributes, which is exactly why the code can't read the absence
 * of a choice off the value.
 */
function noChoiceMade(): void {
  mockSetByUser.mockReturnValue(false);
  mockConfig.mockReturnValue("sweetpad");
}

function chose(provider: "sweetpad" | "xcode-build-server"): void {
  mockSetByUser.mockReturnValue(true);
  mockConfig.mockReturnValue(provider);
}

let workspace: string;

/** Put a `buildServer.json` in the workspace, verbatim. */
function writeBuildServerConfig(contents: string): void {
  writeFileSync(path.join(workspace, "buildServer.json"), contents, "utf8");
}

beforeEach(() => {
  vi.resetAllMocks();
  workspace = mkdtempSync(path.join(os.tmpdir(), "sweetpad-bsp-"));
  mockWorkspaceFolders.mockReturnValue([workspace]);
});

afterEach(() => {
  rmSync(workspace, { recursive: true, force: true });
});

describe("getBuildServerProvider", () => {
  describe("with no provider configured", () => {
    it("uses the provider declared as the default in package.json", () => {
      noChoiceMade();

      const manifest = JSON.parse(readFileSync(path.resolve(__dirname, "../../package.json"), "utf8"));
      const declared = manifest.contributes.configuration.properties["sweetpad.buildServer.provider"].default;

      // The code fallback and the contributed default are two separate
      // spellings of one decision, and only the manifest reaches the settings
      // UI. Left to drift they disagree about what a workspace that never
      // touched the setting gets.
      expect(getBuildServerProvider()).toBe(declared);
      expect(declared).toBe("sweetpad");
    });

    it("keeps a workspace on the server that already owns buildServer.json", () => {
      noChoiceMade();
      writeBuildServerConfig(JSON.stringify({ name: "xcode build server", scheme: "App" }));

      // Nobody set this setting while xcode-build-server was the default, so
      // the file is the only record that a working setup exists here. Taking
      // it over would swap a build server out from under the project.
      expect(getBuildServerProvider()).toBe("xcode-build-server");
    });

    it("claims a workspace whose config is one SweetPad wrote", () => {
      noChoiceMade();
      writeBuildServerConfig(JSON.stringify({ name: "sweetpad", argv: ["/path/to/server"] }));

      expect(getBuildServerProvider()).toBe("sweetpad");
    });

    it("claims a workspace an older CLI set up under the previous name", () => {
      noChoiceMade();
      writeBuildServerConfig(JSON.stringify({ name: "sweetpad-lib", argv: standaloneArgv() }));

      // `sweetpad bsp init` writes this. Reading it as somebody else's setup
      // would route the workspace to xcode-build-server and start asking it to
      // install a tool it has no use for.
      expect(getBuildServerProvider()).toBe("sweetpad");
      expect(isPreservingForeignBuildServer()).toBe(false);
    });

    it("treats a config too broken to read as nothing to defer to", () => {
      noChoiceMade();
      writeBuildServerConfig("{ not json");

      expect(getBuildServerProvider()).toBe("sweetpad");
    });

    it("survives having no workspace at all", () => {
      noChoiceMade();
      mockWorkspaceFolders.mockReturnValue([]);

      expect(getBuildServerProvider()).toBe("sweetpad");
    });

    it("defers to another server's config in any folder of the window", () => {
      noChoiceMade();
      const second = mkdtempSync(path.join(os.tmpdir(), "sweetpad-bsp-"));
      mockWorkspaceFolders.mockReturnValue([workspace, second]);
      writeFileSync(path.join(second, "buildServer.json"), JSON.stringify({ name: "xcode build server" }), "utf8");

      try {
        // One setting covers every folder, so a setup in the folder nobody is on is still a setup
        // the next build would otherwise replace.
        expect(getBuildServerProvider()).toBe("xcode-build-server");
      } finally {
        rmSync(second, { recursive: true, force: true });
      }
    });
  });

  describe("with a provider configured", () => {
    it("honours an explicit choice", () => {
      chose("xcode-build-server");

      expect(getBuildServerProvider()).toBe("xcode-build-server");
    });

    it("lets an explicit choice override the file on disk", () => {
      chose("sweetpad");
      writeBuildServerConfig(JSON.stringify({ name: "xcode build server" }));

      // Asking for our server is how a workspace opts in, so the leftover file
      // must not veto it — the next build rewrites it.
      expect(getBuildServerProvider()).toBe("sweetpad");
    });
  });
});

describe("isPreservingForeignBuildServer", () => {
  it("is true when another server's config is the only reason we're on it", () => {
    noChoiceMade();
    writeBuildServerConfig(JSON.stringify({ name: "xcode build server" }));

    expect(isPreservingForeignBuildServer()).toBe(true);
  });

  it("is false once the workspace has asked for xcode-build-server itself", () => {
    chose("xcode-build-server");
    writeBuildServerConfig(JSON.stringify({ name: "xcode build server" }));

    // Same provider, but nothing was preserved on the workspace's behalf, so
    // there is nothing to tell it about.
    expect(isPreservingForeignBuildServer()).toBe(false);
  });

  it("is false when there is no foreign config to preserve", () => {
    noChoiceMade();

    expect(isPreservingForeignBuildServer()).toBe(false);
  });
});

describe("isSweetpadBuildServerActive", () => {
  it("is active when the provider is ours and so is the config", async () => {
    chose("sweetpad");
    writeBuildServerConfig(JSON.stringify({ name: "sweetpad", argv: managedArgv() }));

    await expect(isSweetpadBuildServerActive(workspace)).resolves.toBe(true);
  });

  it("is inactive when another server's config is what sourcekit-lsp will launch", async () => {
    chose("sweetpad");
    writeBuildServerConfig(JSON.stringify({ name: "xcode build server", scheme: "App" }));

    // The setting says ours, the file says otherwise, and the file is what
    // sourcekit-lsp acts on.
    await expect(isSweetpadBuildServerActive(workspace)).resolves.toBe(false);
  });

  it("is inactive when there is no config at all", async () => {
    chose("sweetpad");

    await expect(isSweetpadBuildServerActive(workspace)).resolves.toBe(false);
  });

  it("is inactive when the provider is not ours", async () => {
    chose("xcode-build-server");
    writeBuildServerConfig(JSON.stringify({ name: "sweetpad", argv: managedArgv() }));

    await expect(isSweetpadBuildServerActive(workspace)).resolves.toBe(false);
  });

  it("is inactive for a standalone config that shares our name", async () => {
    chose("sweetpad");
    writeBuildServerConfig(JSON.stringify({ name: "sweetpad", argv: standaloneArgv() }));

    // `sweetpad bsp init` writes the same name now, so only argv separates the
    // two. That server reads neither our bsp.json nor our socket, and calling
    // it active would have us write and report against something we don't run.
    await expect(isSweetpadBuildServerActive(workspace)).resolves.toBe(false);
  });
});

const snapshot = (phase: string | null, phaseDetail: string | null = null): BspStatusSnapshot => ({
  bspConnected: true,
  scheme: "App",
  configuration: "Debug",
  logLevel: "info",
  phase,
  phaseDetail,
});

describe("preparedCheck", () => {
  it("reports a failed prepare with the server's detail", () => {
    const check = preparedCheck(snapshot("failed", "App: exit=65: clang: error: no such file"));
    expect(check.ok).toBe(false);
    expect(check.detail).toBe("App: exit=65: clang: error: no such file");
    expect(check.hint).toBeDefined();
  });

  it("does not call a prepare that has not run a failure", () => {
    const check = preparedCheck(snapshot(null));
    expect(check.ok).toBe(true);
    expect(check.detail).toBe("not run yet");
  });

  it("distinguishes a prepare still running from a finished one", () => {
    expect(preparedCheck(snapshot("preparing")).detail).toBe("in progress");
    expect(preparedCheck(snapshot("ready")).detail).toBe("ready");
  });
});
