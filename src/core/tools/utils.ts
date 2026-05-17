import type { UserAsker } from "../asker/types";
import { TOOLS, type Tool } from "./constants";

/**
 * Ask user to select a tool from the list of available tools
 */
export async function askTool(asker: UserAsker, options: { title: string }): Promise<Tool> {
  const tools = TOOLS;
  const selected = await asker.pick({
    title: options.title,
    items: tools.map((tool) => ({
      label: tool.label,
      context: { tool: tool },
    })),
  });
  return selected.context.tool;
}
