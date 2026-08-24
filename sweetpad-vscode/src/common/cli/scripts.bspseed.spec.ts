import { promises as fs } from "node:fs";

import type { Mock } from "vitest";

import { assembleBspConfig, hasBspConfig, writeBspConfig } from "../../bsp/write";
import { generateSweetpadBuildServerConfig } from "./scripts";

vi.mock("@sweetpad/native", () => ({}));
vi.mock("../../bsp/commands", () => ({ getBuildServerProvider: vi.fn(() => "sweetpad") }));
vi.mock("../../bsp/write", () => ({
  assembleBspConfig: vi.fn((parts: unknown) => parts),
  hasBspConfig: vi.fn(),
  writeBspConfig: vi.fn(),
}));
vi.mock("../../build/utils", () => ({
  detectWorkspaceType: vi.fn(() => "xcode"),
  getSwiftPMDirectory: vi.fn(),
  prepareDerivedDataPath: vi.fn(() => null),
}));
vi.mock("../exec", () => ({ exec: vi.fn(async () => "/opt/homebrew/bin/sweetpad\n") }));
vi.mock("../tasks/shell-env", () => ({
  getShellDeveloperDir: vi.fn(async () => "/Applications/Xcode.app/Contents/Developer"),
}));
vi.mock("../config", () => ({ getWorkspaceConfig: vi.fn(() => undefined) }));
vi.mock("../logger", () => ({
  commonLogger: { warn: vi.fn(), log: vi.fn(), debug: vi.fn(), error: vi.fn() },
}));

const mockHasBspConfig = hasBspConfig as Mock;
const mockWriteBspConfig = writeBspConfig as Mock;
const mockAssemble = assembleBspConfig as Mock;

// `buildServer.json` names a `bsp.json` the BSP server opens on startup. A
// pointer written while that file is absent is what issue #326 reported: the
// server exits and sourcekit-lsp reports only a closed connection.
describe("generateSweetpadBuildServerConfig — bsp.json comes first", () => {
  let order: string[];
  let writeFile: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    vi.clearAllMocks();
    order = [];
    mockWriteBspConfig.mockImplementation(async () => {
      order.push("bsp.json");
    });
    writeFile = vi.spyOn(fs, "writeFile").mockImplementation(async () => {
      order.push("buildServer.json");
    });
  });

  afterEach(() => {
    writeFile.mockRestore();
  });

  it("writes bsp.json before the buildServer.json that points at it", async () => {
    mockHasBspConfig.mockResolvedValue(false);

    await generateSweetpadBuildServerConfig({
      workspaceRoot: "/workspace",
      xcworkspace: "/workspace/App.xcworkspace",
      scheme: "App",
      configuration: "Release",
    });

    expect(order).toEqual(["bsp.json", "buildServer.json"]);
    // The caller's selection, not a default: this is what the server reads
    // until BspService next rewrites it.
    expect(mockAssemble).toHaveBeenCalledWith(
      expect.objectContaining({
        workspacePath: "/workspace",
        xcworkspace: "/workspace/App.xcworkspace",
        scheme: "App",
        configuration: "Release",
      }),
    );
  });

  it("leaves an existing bsp.json alone", async () => {
    mockHasBspConfig.mockResolvedValue(true);

    await generateSweetpadBuildServerConfig({
      workspaceRoot: "/workspace",
      xcworkspace: "/workspace/App.xcworkspace",
      scheme: "App",
      configuration: "Debug",
    });

    // BspService keeps that file current; overwriting it here would drop the
    // live scheme and configuration back to defaults.
    expect(mockWriteBspConfig).not.toHaveBeenCalled();
    expect(order).toEqual(["buildServer.json"]);
  });

  it("still writes buildServer.json when the project is unknown", async () => {
    mockHasBspConfig.mockResolvedValue(false);

    await generateSweetpadBuildServerConfig({
      workspaceRoot: "/workspace",
      xcworkspace: undefined,
      scheme: undefined,
      configuration: undefined,
    });

    // No project to name, so nothing to seed. The pointer is still written:
    // BspService fills the gap once it sees it.
    expect(mockWriteBspConfig).not.toHaveBeenCalled();
    expect(order).toEqual(["buildServer.json"]);
  });

  it("still writes buildServer.json when the seed fails", async () => {
    mockHasBspConfig.mockResolvedValue(false);
    mockWriteBspConfig.mockRejectedValue(new Error("EROFS"));

    await generateSweetpadBuildServerConfig({
      workspaceRoot: "/workspace",
      xcworkspace: "/workspace/App.xcworkspace",
      scheme: "App",
      configuration: "Debug",
    });

    // A state home we cannot write costs code intelligence until the next
    // write; dropping the pointer with it costs the whole setup.
    expect(order).toEqual(["buildServer.json"]);
  });
});
