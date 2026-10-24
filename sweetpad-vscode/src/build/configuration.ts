import { getSchemeConfiguration } from "../common/cli/scripts";
import { getWorkspaceConfig } from "../common/config";
import type { WorkspaceStateService } from "../common/workspace-state";

/** Build and run read one set of configuration picks, tests another, as with schemes. */
export type ConfigurationScope = "build" | "testing";

const PICKS_KEY = {
  build: "build.xcodeConfigurationByScheme",
  testing: "testing.xcodeConfigurationByScheme",
} as const;

const SETTING_KEY = {
  build: "build.configuration",
  testing: "testing.configuration",
} as const;

const SCHEME_ACTION = {
  build: "launch",
  testing: "test",
} as const;

type SchemeKey = {
  workspaceState: WorkspaceStateService;
  scope: ConfigurationScope;
  xcworkspace: string;
  scheme: string;
};

/**
 * The configuration picked for one scheme of one project. Picks are keyed by both because a folder
 * can hold several projects with a same-named scheme whose configurations differ.
 */
export function getPickedConfiguration(options: SchemeKey): string | undefined {
  return options.workspaceState.get(PICKS_KEY[options.scope])?.[options.xcworkspace]?.[options.scheme];
}

/** Record a pick for one scheme, or drop it with `undefined` so the scheme's own configuration applies. */
export function setPickedConfiguration(options: SchemeKey & { configuration: string | undefined }): void {
  const key = PICKS_KEY[options.scope];
  const picks = { ...options.workspaceState.get(key) };
  const forWorkspace = { ...picks[options.xcworkspace] };
  if (options.configuration === undefined) {
    delete forWorkspace[options.scheme];
  } else {
    forWorkspace[options.scheme] = options.configuration;
  }
  if (Object.keys(forWorkspace).length === 0) {
    delete picks[options.xcworkspace];
  } else {
    picks[options.xcworkspace] = forWorkspace;
  }
  options.workspaceState.update(key, Object.keys(picks).length === 0 ? undefined : picks);
}

/**
 * The configuration a build of `scheme` uses without asking: the workspace setting, then the pick for
 * that scheme, then the scheme's own (`LaunchAction` for builds, `TestAction` for tests). Undefined
 * leaves the caller to its fallback (a picker, or Debug where nobody can be asked).
 */
export function resolveConfiguration(options: {
  workspaceState: WorkspaceStateService;
  scope: ConfigurationScope;
  xcworkspace: string | undefined;
  scheme: string | undefined;
}): string | undefined {
  const fromSetting = getWorkspaceConfig(SETTING_KEY[options.scope]);
  if (fromSetting) {
    return fromSetting;
  }
  if (!options.xcworkspace || !options.scheme) {
    return undefined;
  }
  const key = { ...options, xcworkspace: options.xcworkspace, scheme: options.scheme };
  return (
    getPickedConfiguration(key) ??
    getSchemeConfiguration({
      xcworkspace: options.xcworkspace,
      scheme: options.scheme,
      action: SCHEME_ACTION[options.scope],
    })
  );
}
