import { promises as fs } from "node:fs";

import type { Mock } from "vitest";

import { getBuildServerProvider } from "../../bsp/commands";
import { isFileExists } from "../files";
import { commonLogger } from "../logger";
import { generateBuildServerConfig } from "./scripts";

vi.mock("../../bsp/commands", () => ({ getBuildServerProvider: vi.fn() }));
vi.mock("../files", () => ({ isFileExists: vi.fn(), readJsonFile: vi.fn() }));
vi.mock("../logger", () => ({
  commonLogger: { warn: vi.fn(), log: vi.fn(), debug: vi.fn(), error: vi.fn() },
}));

const mockProvider = getBuildServerProvider as Mock;
const mockIsFileExists = isFileExists as Mock;
const mockWarn = commonLogger.warn as Mock;
const mockLog = commonLogger.log as Mock;

// `detectWorkspaceType` / `getSwiftPMDirectory` are pure path helpers, so the
// real ones run here — only the provider, filesystem probe, and logger are mocked.
describe("generateBuildServerConfig — Swift package (sweetpad provider)", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mockProvider.mockReturnValue("sweetpad");
  });

  it("writes no buildServer.json for a package (native sourcekit-lsp route)", async () => {
    const writeFile = vi.spyOn(fs, "writeFile");
    mockIsFileExists.mockResolvedValue(false);

    await generateBuildServerConfig({ xcworkspace: "/pkg/Package.swift", scheme: "MyLib" });

    expect(writeFile).not.toHaveBeenCalled();
    expect(mockLog).toHaveBeenCalled();
    expect(mockWarn).not.toHaveBeenCalled();
    writeFile.mockRestore();
  });

  it("warns about a stale buildServer.json in the package directory", async () => {
    const writeFile = vi.spyOn(fs, "writeFile");
    mockIsFileExists.mockResolvedValue(true);

    await generateBuildServerConfig({ xcworkspace: "/pkg/Package.swift", scheme: "MyLib" });

    expect(mockIsFileExists).toHaveBeenCalledWith("/pkg/buildServer.json");
    expect(mockWarn).toHaveBeenCalled();
    expect(writeFile).not.toHaveBeenCalled();
    writeFile.mockRestore();
  });
});
