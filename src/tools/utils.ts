import { showQuickPick } from "../common/quick-pick";

import { TOOLS, type Tool } from "./constants";

/**
 * Ask user to select a tool from the list of available tools
 */
export async function askTool(options: { title: string }): Promise<Tool> {
  const tools = TOOLS;
  const selected = await showQuickPick({
    title: options.title,
    items: tools.map((tool) => {
      return {
        label: tool.label,
        context: {
          tool: tool,
        },
      };
    }),
  });
  return selected.context.tool;
}
