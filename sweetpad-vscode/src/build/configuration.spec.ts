import type { Mock } from "vitest";

import { getSchemeConfiguration } from "../common/cli/scripts";
import { getWorkspaceConfig } from "../common/config";
import type { WorkspaceStateService } from "../common/workspace-state";
import { getPickedConfiguration, resolveConfiguration, setPickedConfiguration } from "./configuration";

vi.mock("../common/config", () => ({ getWorkspaceConfig: vi.fn() }));
vi.mock("../common/cli/scripts", () => ({ getSchemeConfiguration: vi.fn() }));

const mockSetting = getWorkspaceConfig as Mock;
const mockSchemeConfiguration = getSchemeConfiguration as Mock;

function memoryState(initial: Record<string, unknown> = {}): WorkspaceStateService {
  const values = new Map(Object.entries(initial));
  return {
    get: (key: string) => values.get(key),
    update: (key: string, value: unknown) => {
      if (value === undefined) {
        values.delete(key);
      } else {
        values.set(key, value);
      }
    },
    rawKeys: () => [...values.keys()],
  } as unknown as WorkspaceStateService;
}

const APP = "/work/App.xcworkspace";
const OTHER = "/work/Other.xcodeproj";

describe("resolveConfiguration", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  // #342: each scheme builds with the configuration its own .xcscheme names, so switching from
  // "App TEST" to "App UAT" switches the configuration with it.
  it("uses each scheme's own launch configuration when nothing is picked", () => {
    const state = memoryState();
    mockSchemeConfiguration.mockImplementation(({ scheme }) => (scheme === "App UAT" ? "UAT" : "TEST-Debug"));

    expect(resolveConfiguration({ workspaceState: state, scope: "build", xcworkspace: APP, scheme: "App UAT" })).toBe(
      "UAT",
    );
    expect(resolveConfiguration({ workspaceState: state, scope: "build", xcworkspace: APP, scheme: "App TEST" })).toBe(
      "TEST-Debug",
    );
    expect(mockSchemeConfiguration).toHaveBeenCalledWith({ xcworkspace: APP, scheme: "App UAT", action: "launch" });
  });

  it("reads the scheme's test action for testing", () => {
    mockSchemeConfiguration.mockReturnValue("TEST-Debug");

    expect(
      resolveConfiguration({ workspaceState: memoryState(), scope: "testing", xcworkspace: APP, scheme: "App" }),
    ).toBe("TEST-Debug");
    expect(mockSchemeConfiguration).toHaveBeenCalledWith({ xcworkspace: APP, scheme: "App", action: "test" });
  });

  it("prefers a pick for the scheme over the scheme's own configuration", () => {
    const state = memoryState();
    mockSchemeConfiguration.mockReturnValue("UAT");
    setPickedConfiguration({
      workspaceState: state,
      scope: "build",
      xcworkspace: APP,
      scheme: "App UAT",
      configuration: "Release",
    });

    expect(resolveConfiguration({ workspaceState: state, scope: "build", xcworkspace: APP, scheme: "App UAT" })).toBe(
      "Release",
    );
  });

  it("lets the workspace setting override every scheme", () => {
    const state = memoryState();
    mockSetting.mockImplementation((key: string) => (key === "build.configuration" ? "Staging" : undefined));
    setPickedConfiguration({
      workspaceState: state,
      scope: "build",
      xcworkspace: APP,
      scheme: "App",
      configuration: "Release",
    });

    expect(resolveConfiguration({ workspaceState: state, scope: "build", xcworkspace: APP, scheme: "App" })).toBe(
      "Staging",
    );
    expect(
      resolveConfiguration({ workspaceState: state, scope: "build", xcworkspace: undefined, scheme: undefined }),
    ).toBe("Staging");
  });

  it("answers nothing without a scheme, leaving the caller its fallback", () => {
    expect(
      resolveConfiguration({ workspaceState: memoryState(), scope: "build", xcworkspace: APP, scheme: undefined }),
    ).toBeUndefined();
    expect(mockSchemeConfiguration).not.toHaveBeenCalled();
  });

  // The single workspace-wide pick older versions stored is the value that pinned every scheme to
  // one configuration, so it is not read at all.
  it("ignores the workspace-wide pick older versions stored", () => {
    const state = memoryState({ "build.xcodeConfiguration": "TEST-Debug" });
    mockSchemeConfiguration.mockReturnValue("UAT");

    expect(resolveConfiguration({ workspaceState: state, scope: "build", xcworkspace: APP, scheme: "App UAT" })).toBe(
      "UAT",
    );
  });
});

describe("setPickedConfiguration", () => {
  it("keeps picks apart for same-named schemes of different projects", () => {
    const state = memoryState();
    setPickedConfiguration({
      workspaceState: state,
      scope: "build",
      xcworkspace: APP,
      scheme: "App",
      configuration: "UAT",
    });
    setPickedConfiguration({
      workspaceState: state,
      scope: "build",
      xcworkspace: OTHER,
      scheme: "App",
      configuration: "Beta",
    });

    expect(getPickedConfiguration({ workspaceState: state, scope: "build", xcworkspace: APP, scheme: "App" })).toBe(
      "UAT",
    );
    expect(getPickedConfiguration({ workspaceState: state, scope: "build", xcworkspace: OTHER, scheme: "App" })).toBe(
      "Beta",
    );
  });

  it("keeps build and testing picks apart", () => {
    const state = memoryState();
    setPickedConfiguration({
      workspaceState: state,
      scope: "testing",
      xcworkspace: APP,
      scheme: "App",
      configuration: "TEST",
    });

    expect(
      getPickedConfiguration({ workspaceState: state, scope: "build", xcworkspace: APP, scheme: "App" }),
    ).toBeUndefined();
  });

  it("drops a pick, and the state key once nothing is left", () => {
    const state = memoryState();
    setPickedConfiguration({
      workspaceState: state,
      scope: "build",
      xcworkspace: APP,
      scheme: "App",
      configuration: "UAT",
    });
    setPickedConfiguration({
      workspaceState: state,
      scope: "build",
      xcworkspace: APP,
      scheme: "App",
      configuration: undefined,
    });

    expect(
      getPickedConfiguration({ workspaceState: state, scope: "build", xcworkspace: APP, scheme: "App" }),
    ).toBeUndefined();
    expect(state.get("build.xcodeConfigurationByScheme")).toBeUndefined();
  });
});
