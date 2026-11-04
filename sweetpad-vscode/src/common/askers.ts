import path from "node:path";

import * as vscode from "vscode";

import {
  type XcodeConfiguration,
  findSchemeFile,
  getBuildConfigurations,
  getProjectDefaultConfiguration,
} from "./cli/scripts";
import { type QuickPickItem, showInputBox, showQuickPick } from "./quick-pick";

export const DEFAULT_DEBUG_CONFIGURATION = "Debug";
export const DEFAULT_RELEASE_CONFIGURATION = "Release";

/**
 * Base function to ask user to select configuration it doesn't use cache nor store selected configuration
 *
 * "Debug" configuration is used as default
 */
export async function askConfigurationBase(options: { xcworkspace: string }) {
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

  const projectDefaultConfigration = getProjectDefaultConfiguration(options.xcworkspace);

  // Give user a choice to select configuration if we don't know wich one to use
  return await showConfigurationPicker(configurations, {
    projectDefault: projectDefaultConfigration,
  });
}

/**
 * Show quick pick with all configurations and return selected configuration name.
 */
async function showConfigurationPicker(
  configurations: XcodeConfiguration[],
  options: { projectDefault: string | undefined },
): Promise<string> {
  const selected = await showQuickPick({
    title: "Select configuration",
    items: configurationRows(configurations, { projectDefault: options.projectDefault, current: undefined }),
  });
  return selected.context.configuration;
}

/**
 * One row per configuration, the one in effect with a check. `projectDefault` is the project's
 * "Use … for command-line builds" configuration: it goes first, so it is the highlighted row.
 */
function configurationRows(
  configurations: XcodeConfiguration[],
  options: { projectDefault: string | undefined; current: string | undefined },
) {
  const first = configurations.filter((c) => c.name === options.projectDefault);
  const rest = configurations.filter((c) => c.name !== options.projectDefault);
  return [...first, ...rest].map((configuration) => ({
    label: `${configuration.name === options.current ? "$(check) " : ""}${configuration.name}`,
    description: configuration.name === options.projectDefault ? "Xcode command-line default" : undefined,
    context: { configuration: configuration.name },
  }));
}

/**
 * The picker behind "Select configuration" for one scheme. Resolves to the picked name, or to
 * `{ configuration: undefined }` when the user picks the scheme's own configuration back, and to
 * undefined when nothing was picked. A project that lists no configurations asks for a name instead.
 */
export async function askConfigurationForScheme(options: {
  xcworkspace: string;
  scheme: string;
  /** The scheme's own configuration, offered as the first row so a pick can be undone. */
  schemeDefault: string | undefined;
  /** The configuration picked or set for this scheme, checked in the list; undefined when the scheme's own applies. */
  current: string | undefined;
}): Promise<{ configuration: string | undefined } | undefined> {
  const configurations = await getBuildConfigurations({ xcworkspace: options.xcworkspace });
  if (configurations.length === 0) {
    const entered = await showInputBox({
      title: "No configurations found. Please enter configuration name manually",
    });
    return entered ? { configuration: entered } : undefined;
  }

  // Xcode's command-line default only applies to builds without a scheme, so it is a hint only
  // when this scheme names no configuration of its own.
  const rows = configurationRows(configurations, {
    projectDefault: options.schemeDefault ? undefined : getProjectDefaultConfiguration(options.xcworkspace),
    current: options.current,
  });
  const items: QuickPickItem<{ configuration: string | undefined }>[] = rows;
  if (options.schemeDefault) {
    // The reset row is an action, not a configuration, so it gets an icon and sits below a separator.
    const inEffect = options.current === undefined;
    const schemeFile = await findSchemeFile(options.xcworkspace, options.scheme);
    items.push(
      { label: "", kind: vscode.QuickPickItemKind.Separator, context: { configuration: undefined } },
      {
        label: `$(${inEffect ? "check" : "discard"}) Use the scheme's configuration`,
        description: schemeFile ? `${options.schemeDefault}, from ${path.basename(schemeFile)}` : options.schemeDefault,
        context: { configuration: undefined },
      },
    );
  }
  const selected = await showQuickPick({
    title: `Select configuration for scheme "${options.scheme}"`,
    items: items,
  });
  return selected.context;
}

export async function showYesNoQuestion(options: { title: string }): Promise<boolean> {
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
