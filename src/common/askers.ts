import { getBuildConfigurations, type XcodeConfiguration } from "./cli/scripts";
import { showInputBox, showQuickPick } from "./quick-pick";

export const DEFAULT_CONFIGURATION = "Debug";

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
  if (configurations.length === 0) {
    return DEFAULT_CONFIGURATION;
  }

  // Use default configuration if it exists
  if (configurations.some((configuration) => configuration.name === DEFAULT_CONFIGURATION)) {
    return DEFAULT_CONFIGURATION;
  }

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