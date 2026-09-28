import type { Mock } from "vitest";

import type { BuildManager } from "../build/manager";
import { restartSwiftLSP } from "../build/utils";
import { onDidChangeConfiguration } from "../common/config";
import type { WorkspaceContextService } from "../common/workspace-context";
import type { WorkspaceStateService } from "../common/workspace-state";
import type { DestinationsManager } from "../destination/manager";
import { isSweetpadBuildServerActive } from "./commands";
import { buildBspResolvedConfig } from "./config";
import { BspService } from "./service";
import { type BspResolvedConfig, readBspConfig, writeBspConfig } from "./write";

vi.mock("../build/utils", () => ({ restartSwiftLSP: vi.fn() }));
vi.mock("../cli-server/registry", () => ({ unregisterBspConfig: vi.fn(async () => {}) }));
vi.mock("../common/config", () => ({ getWorkspaceConfig: vi.fn(), onDidChangeConfiguration: vi.fn() }));
vi.mock("./bridge", () => ({
  BSP_LOG_LEVELS: ["info"],
  BspBridge: class {
    connect() {}
    disconnect() {}
    setLogLevel() {}
    dispose() {}
  },
}));
vi.mock("./commands", () => ({
  getBuildServerProvider: vi.fn(() => "sweetpad"),
  isSweetpadBuildServerActive: vi.fn(),
}));
vi.mock("./config", () => ({ buildBspResolvedConfig: vi.fn() }));
vi.mock("./write", () => ({ readBspConfig: vi.fn(), writeBspConfig: vi.fn() }));

function bspConfig(derivedDataPath: string | null): BspResolvedConfig {
  return {
    workspacePath: "/w",
    projectPath: "/w/App.xcodeproj",
    developerDir: null,
    scheme: "App",
    configuration: "Debug",
    destinationPlatform: null,
    derivedDataPath: derivedDataPath,
    logPath: "/state/bsp.log",
    socket: "/state/bsp.sock",
    buildArgs: [],
  };
}

type ConfigListener = (event: { affectsConfiguration: (section: string) => boolean }) => void;

/** Start a service and hand back what it listens to configuration and destination changes with. */
async function startService(): Promise<{
  service: BspService;
  changeConfig: (section: string) => Promise<void>;
  changeDestination: () => Promise<void>;
}> {
  let listener: ConfigListener | undefined;
  const destinationListeners = new Map<string, () => void>();
  (onDidChangeConfiguration as Mock).mockImplementation((l: ConfigListener) => {
    listener = l;
    return { dispose() {} };
  });
  const service = new BspService({
    workspaceContext: { root: "/w", onDidChange: () => ({ dispose() {} }) } as unknown as WorkspaceContextService,
    buildManager: {
      on: vi.fn(),
      removeAllListeners: vi.fn(),
    } as unknown as BuildManager,
    destinationsManager: {
      on: (event: string, l: () => void) => destinationListeners.set(event, l),
    } as unknown as DestinationsManager,
    workspaceState: {} as WorkspaceStateService,
  });
  await service.start();
  await settle();
  (writeBspConfig as Mock).mockClear();
  return {
    service: service,
    changeConfig: async (section: string) => {
      listener?.({ affectsConfiguration: (s) => s === section });
      await settle();
    },
    changeDestination: async () => {
      destinationListeners.get("xcodeDestinationForBuildUpdated")?.();
      await settle();
    },
  };
}

/** Let the service's fire-and-forget writes finish. */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

beforeEach(() => {
  vi.clearAllMocks();
  (isSweetpadBuildServerActive as Mock).mockResolvedValue(true);
});

// The server reads bsp.json's derivedDataPath only at startup, so moving DerivedData has to restart the
// language server that launched it.
describe("BspService", () => {
  it("rewrites bsp.json when either setting that moves DerivedData changes", async () => {
    (buildBspResolvedConfig as Mock).mockResolvedValue(bspConfig("/w/dd"));
    (readBspConfig as Mock).mockResolvedValue(bspConfig("/w/dd"));
    const { service, changeConfig } = await startService();

    await changeConfig("sweetpad.build.derivedDataPath");
    expect(writeBspConfig).toHaveBeenCalledTimes(1);
    await changeConfig("sweetpad.build.args");
    expect(writeBspConfig).toHaveBeenCalledTimes(2);
    await changeConfig("sweetpad.build.xcbeautifyEnabled");
    expect(writeBspConfig).toHaveBeenCalledTimes(2);
    expect(restartSwiftLSP).not.toHaveBeenCalled();
    service.dispose();
  });

  it("restarts the language server when the DerivedData in bsp.json moves", async () => {
    (buildBspResolvedConfig as Mock).mockResolvedValue(bspConfig(null));
    (readBspConfig as Mock).mockResolvedValue(bspConfig(null));
    const { service, changeConfig } = await startService();

    (buildBspResolvedConfig as Mock).mockResolvedValue(bspConfig("/w/dd"));
    await changeConfig("sweetpad.build.derivedDataPath");
    expect(writeBspConfig).toHaveBeenCalledWith(bspConfig("/w/dd"));
    expect(restartSwiftLSP).toHaveBeenCalledTimes(1);
    service.dispose();
  });

  // A file several targets compile is read as the target for the selected destination's platform.
  it("rewrites bsp.json when the destination for builds changes", async () => {
    (buildBspResolvedConfig as Mock).mockResolvedValue(bspConfig("/w/dd"));
    (readBspConfig as Mock).mockResolvedValue(bspConfig("/w/dd"));
    const { service, changeDestination } = await startService();

    await changeDestination();
    expect(writeBspConfig).toHaveBeenCalledTimes(1);
    expect(restartSwiftLSP).not.toHaveBeenCalled();
    service.dispose();
  });

  it("leaves the language server alone when no bsp.json was there to read", async () => {
    (buildBspResolvedConfig as Mock).mockResolvedValue(bspConfig("/w/dd"));
    (readBspConfig as Mock).mockResolvedValue(undefined);
    const { service, changeConfig } = await startService();

    await changeConfig("sweetpad.build.derivedDataPath");
    expect(writeBspConfig).toHaveBeenCalledTimes(1);
    expect(restartSwiftLSP).not.toHaveBeenCalled();
    service.dispose();
  });
});
