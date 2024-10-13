import { getTargets } from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { ExtensionError } from "../common/errors";
import { showQuickPick } from "../common/quick-pick";

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
): Promise<string> {
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
    throw new ExtensionError("No testing targets found");
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
      console.log(target, cachedTarget, target === cachedTarget);
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
