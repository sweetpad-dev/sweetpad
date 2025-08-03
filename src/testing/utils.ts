import * as vscode from "vscode";
import { askConfigurationBase } from "../common/askers";
import { type XcodeBuildSettings, getSchemes, getTargets } from "../common/cli/scripts";
import { getWorkspaceConfig } from "../common/config";
import type { ExtensionContext } from "../common/context";
import { type QuickPickItem, showQuickPick } from "../common/quick-pick";
import type { XcodeWorkspace } from "../common/xcode/workspace";
import type { DestinationPlatform } from "../destination/constants";
import type { Destination } from "../destination/types";
import { splitSupportedDestinatinos } from "../destination/utils";

/**
 * Ask user to select target to build
 */
export async function askTestingTarget(
  context: ExtensionContext,
  options: {
    title: string;
    xcworkspace: XcodeWorkspace;
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
    xcworkspace: XcodeWorkspace;
    scheme: string;
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
    scheme: options.scheme,
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

  return await selectDestinationForTesting(context, {
    destinations: destinations,
    supportedPlatforms: supportedPlatforms,
  });
}

export async function selectDestinationForTesting(
  context: ExtensionContext,
  options: {
    destinations: Destination[];
    supportedPlatforms: DestinationPlatform[] | undefined;
  },
): Promise<Destination> {
  const { supported, unsupported } = splitSupportedDestinatinos({
    destinations: options.destinations,
    supportedPlatforms: options.supportedPlatforms,
  });

  const supportedItems: QuickPickItem<Destination>[] = supported.map((destination) => ({
    label: destination.name,
    iconPath: new vscode.ThemeIcon(destination.icon),
    detail: destination.quickPickDetails,
    context: destination,
  }));
  const unsupportedItems: QuickPickItem<Destination>[] = unsupported.map((destination) => ({
    label: destination.name,
    iconPath: new vscode.ThemeIcon(destination.icon),
    detail: destination.quickPickDetails,
    context: destination,
  }));

  const items: QuickPickItem<Destination>[] = [];
  if (unsupported.length === 0 && supported.length === 0) {
    // Show that no destinations found
    items.push({
      label: "No destinations found",
      kind: vscode.QuickPickItemKind.Separator,
      context: supported[0],
    });
  } else if (supported.length > 0 && unsupported.length > 0) {
    // Split supported and unsupported destinations
    items.push({
      label: "Supported",
      kind: vscode.QuickPickItemKind.Separator,
      context: supported[0],
    });
    items.push(...supportedItems);
    items.push({
      label: "Other",
      kind: vscode.QuickPickItemKind.Separator,
      context: supported[0],
    });
    items.push(...unsupportedItems);
  } else {
    // Just make flat list, one is empty and another is not
    items.push(...supportedItems);
    items.push(...unsupportedItems);
  }

  const selected = await showQuickPick<Destination>({
    title: "Select destination to test on",
    items: items,
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
    xcworkspace: XcodeWorkspace;
    ignoreCache?: boolean;
  },
): Promise<string> {
  const cachedScheme = context.buildManager.getDefaultSchemeForTesting();
  if (cachedScheme && !options.ignoreCache) {
    return cachedScheme;
  }

  context.updateProgressStatus("Searching for scheme");
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
