import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import type { Mock } from "vitest";

import { getWorkspacePath } from "../build/utils";
import { getWorkspaceConfig, isWorkspaceConfigSetByUser } from "../common/config";
import { getBuildServerProvider, isPreservingForeignBuildServer, isSweetpadBuildServerActive } from "./commands";

vi.mock("../common/config", () => ({
  getWorkspaceConfig: vi.fn(),
  isWorkspaceConfigSetByUser: vi.fn(),
  updateWorkspaceConfig: vi.fn(),
}));
vi.mock("../build/utils", () => ({ getWorkspacePath: vi.fn() }));

const mockConfig = getWorkspaceConfig as Mock;
const mockSetByUser = isWorkspaceConfigSetByUser as Mock;
const mockWorkspacePath = getWorkspacePath as Mock;

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
  mockWorkspacePath.mockReturnValue(workspace);
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

    it("claims a workspace the CLI set up, which runs the same server", () => {
      noChoiceMade();
      writeBuildServerConfig(
        JSON.stringify({ name: "sweetpad-lib", argv: ["/opt/homebrew/bin/sweetpad", "bsp", "serve"] }),
      );

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
      mockWorkspacePath.mockImplementation(() => {
        throw new Error("no workspace open");
      });

      expect(getBuildServerProvider()).toBe("sweetpad");
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
    writeBuildServerConfig(JSON.stringify({ name: "sweetpad", argv: ["/path/to/server"] }));

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
    writeBuildServerConfig(JSON.stringify({ name: "sweetpad" }));

    await expect(isSweetpadBuildServerActive(workspace)).resolves.toBe(false);
  });
});
