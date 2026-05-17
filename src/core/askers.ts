import type { UserAsker } from "./asker/types";
import { type XcodeCliDeps, type XcodeConfiguration, getBuildConfigurations } from "./cli/scripts";

export const DEFAULT_DEBUG_CONFIGURATION = "Debug";
export const DEFAULT_RELEASE_CONFIGURATION = "Release";

/**
 * Base function to ask user to select configuration. Doesn't cache nor store the selection.
 *
 * "Debug" is preferred when present, to match Xcode's default LaunchAction.
 */
export async function askConfigurationBase(
  deps: XcodeCliDeps & { asker: UserAsker },
  options: { xcworkspace: string },
): Promise<string> {
  const configurations = await getBuildConfigurations(deps, {
    xcworkspace: options.xcworkspace,
  });

  if (configurations.length === 0) {
    return DEFAULT_DEBUG_CONFIGURATION;
  }

  if (configurations.length === 1) {
    return configurations[0].name;
  }

  // Most common case. When new project is created by default Xcode creates Debug and Release
  // configurations, and "Debug" is mostly set as default for launching. To avoid confusion just
  // use "Debug" and don't ask.
  if (
    configurations.length === 2 &&
    configurations.some((c) => c.name === DEFAULT_DEBUG_CONFIGURATION) &&
    configurations.some((c) => c.name === DEFAULT_RELEASE_CONFIGURATION)
  ) {
    return DEFAULT_DEBUG_CONFIGURATION;
  }

  return await showConfigurationPicker(deps.asker, configurations);
}

/**
 * Show a picker with all configurations and return the selected name.
 */
export async function showConfigurationPicker(asker: UserAsker, configurations: XcodeConfiguration[]): Promise<string> {
  const selected = await asker.pick({
    title: "Select configuration",
    items: configurations.map((configuration) => ({
      label: configuration.name,
      context: { configuration },
    })),
  });
  return selected.context.configuration.name;
}

export async function showYesNoQuestion(asker: UserAsker, options: { title: string }): Promise<boolean> {
  const selected = await asker.pick<boolean>({
    title: options.title,
    items: [
      { label: "Yes", context: true },
      { label: "No", context: false },
    ],
  });
  return selected.context;
}
