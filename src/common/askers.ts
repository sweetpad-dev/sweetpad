import { type XcodeConfiguration, getBuildConfigurations } from "./cli/scripts";
import { showQuickPick } from "./quick-pick";

export const DEFAULT_DEBUG_CONFIGURATION = "Debug";
export const DEFAULT_RELEASE_CONFIGURATION = "Release";

/**
 * Base function to ask user to select configuration it doesn't use cache nor store selected configuration
 *
 * "Debug" configuration is used as default
 */
export async function askConfigurationBase(options: {
  xcworkspace: string;
}) {
  // Fetch all configurations
  const configurations = await getBuildConfigurations({
    xcworkspace: options.xcworkspace,
  });

  // Use default configuration if no configurations found
  // todo: we can try to parse configurations from schemes files or ask user to enter configuration name manually
  if (configurations.length === 0) {
    return DEFAULT_DEBUG_CONFIGURATION;
  }

  // When we have only one configuration let's use it as default
  if (configurations.length === 1) {
    return configurations[0].name;
  }

  // This is most common case. When new project is created by default Xcode creates Debug and Release configurations
  // and "Debug" is mostly set as default configuration for launching application. To avoid confusion we will just use
  // "Debug" configuration as default and don't ask user to select configuration.
  if (
    configurations.length === 2 &&
    configurations.some((c) => c.name === DEFAULT_DEBUG_CONFIGURATION) &&
    configurations.some((c) => c.name === DEFAULT_RELEASE_CONFIGURATION)
  ) {
    return DEFAULT_DEBUG_CONFIGURATION;
  }

  // todo: we can try to find configuration in schemes files for "launch" action
  // File: MyApp.xcodeproj/xcuserdata/YourName.xcuserdatad/xcschemes/MyApp.xcscheme
  // Path: /Scheme/BuildAction[buildConfiguration]
  // Path: /Scheme/LaunchAction[buildConfiguration]

  // Give user a choice to select configuration if we don't know wich one to use
  return await showConfigurationPicker(configurations);
}

/**
 * Just show quick pick with all configurations and return selected configuration name
 */
export async function showConfigurationPicker(configurations: XcodeConfiguration[]): Promise<string> {
  const selected = await showQuickPick({
    title: "Select configuration",
    items: configurations.map((configuration) => {
      return {
        label: configuration.name,
        context: {
          configuration,
        },
      };
    }),
  });
  return selected.context.configuration.name;
}

export async function showYesNoQuestion(options: {
  title: string;
}): Promise<boolean> {
  const selected = await showQuickPick({
    title: options.title,
    items: [
      {
        label: "Yes",
        context: true,
      },
      {
        label: "No",
        context: false,
      },
    ],
  });
  return selected.context;
}
