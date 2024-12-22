import * as vscode from "vscode";
import { askConfigurationBase } from "../common/askers";
import { type XcodeBuildSettings, getSchemes, getTargets } from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { showQuickPick } from "../common/quick-pick";
import type { Destination } from "../destination/types";

/**
 * Ask user to select target to build
 */
export async function askTestingTarget(
  context: ExtensionContext,
  options: {
    title: string;
    xcworkspace: string;
    force?: boolean;
  },
): Promise<string | null> {
  // Testing target can be cached
  const cachedTarget = context.testingManager.getDefaultTestingTarget();
  if (cachedTarget && !options.force) {
    return cachedTarget;
  }

  // Get from commmand line or from xcode files
  const targets = await getTargets({
    xcworkspace: options.xcworkspace,
  });

  // Target is required for testing
  if (!targets.length) {
    return null;
  }

  // Auto select target if only one found
  if (targets.length === 1 && !options.force) {
    const targetName = targets[0];
    context.testingManager.setDefaultTestingTarget(targetName);
    return targetName;
  }

  // Offer user to select target if multiple found
  const target = await showQuickPick({
    title: options.title,
    items: targets.map((target) => {
      return {
        label: target,
        description: target === cachedTarget ? "(current)" : undefined,
        context: {
          target: target,
        },
      };
    }),
  });

  const targetName = target.context.target;
  context.testingManager.setDefaultTestingTarget(targetName);
  return targetName;
}

/**
 * Ask user to select configuration
 */
export async function askConfigurationForTesting(
  context: ExtensionContext,
  options: {
    xcworkspace: string;
  },
): Promise<string> {
  const fromConfig = getWorkspaceConfig("testing.configuration");
  if (fromConfig) {
    return fromConfig;
  }
  const cached = context.buildManager.getDefaultConfigurationForTesting();
  if (cached) {
    return cached;
  }
  const selected = await askConfigurationBase({
    xcworkspace: options.xcworkspace,
  });
  context.buildManager.setDefaultConfigurationForTesting(selected);
  return selected;
}

/**
 * Ask user to select simulator or device to run on
 */
export async function askDestinationToTestOn(
  context: ExtensionContext,
  buildSettings: XcodeBuildSettings | null,
): Promise<Destination> {
  // We can remove platforms that are not supported by the project
  const supportedPlatforms = buildSettings?.supportedPlatforms;

  const destinations = await context.destinationsManager.getDestinations({
    platformFilter: supportedPlatforms,
    mostUsedSort: true,
  });

  // If we have cached desination, use it
  const cachedDestination = context.destinationsManager.getSelectedXcodeDestinationForTesting();
  if (cachedDestination) {
    const destination = destinations.find(
      (destination) => destination.id === cachedDestination.id && destination.type === cachedDestination.type,
    );
    if (destination) {
      return destination;
    }
  }

  return selectDestinationForTesting(context, {
    destinations: destinations,
  });
}

export async function selectDestinationForTesting(
  context: ExtensionContext,
  options?: {
    destinations?: Destination[];
  },
): Promise<Destination> {
  const destinations = options?.destinations?.length
    ? options.destinations
    : await context.destinationsManager.getDestinations({
        mostUsedSort: true,
      });

  const selected = await showQuickPick<Destination>({
    title: "Select destination to test on",
    items: [
      ...destinations.map((destination) => {
        return {
          label: destination.name,
          iconPath: new vscode.ThemeIcon(destination.icon),
          detail: destination.quickPickDetails,
          context: destination,
        };
      }),
    ],
  });

  const destination = selected.context;

  context.destinationsManager.setWorkspaceDestinationForTesting(destination);
  return destination;
}

/**
 * Ask user to select scheme to build
 */
export async function askSchemeForTesting(
  context: ExtensionContext,
  options: {
    title?: string;
    xcworkspace: string;
    ignoreCache?: boolean;
  },
): Promise<string> {
  const cachedScheme = context.buildManager.getDefaultSchemeForTesting();
  if (cachedScheme && !options.ignoreCache) {
    return cachedScheme;
  }

  const schemes = await getSchemes({
    xcworkspace: options.xcworkspace,
  });

  const scheme = await showQuickPick({
    title: options?.title ?? "Select scheme to test on",
    items: schemes.map((scheme) => {
      return {
        label: scheme.name,
        context: {
          scheme: scheme,
        },
      };
    }),
  });

  const schemeName = scheme.context.scheme.name;
  context.buildManager.setDefaultSchemeForTesting(schemeName);
  return schemeName;
}
