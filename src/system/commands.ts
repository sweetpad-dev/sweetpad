import * as vscode from "vscode";
import type { CommandExecution } from "../common/commands";
import { commonLogger } from "../common/logger";

export async function resetSweetpadCache(execution: CommandExecution) {
  execution.context.resetWorkspaceState();
  vscode.window.showInformationMessage("Sweetpad cache has been reset");
}

async function createIssue(options: { title: string; body: string; labels: string[] }) {
  const url = new URL("https://github.com/sweetpad-dev/sweetpad/issues/new");
  url.searchParams.append("title", options.title);
  url.searchParams.append("body", options.body);
  url.searchParams.append("labels", options.labels.join(","));

  vscode.env.openExternal(vscode.Uri.parse(url.toString()));
}

export async function createIssueGenericCommand(execution: CommandExecution) {
  await createIssue({
    title: "Sweetpad issue",
    body: "Please describe your issue here",
    labels: ["bug"],
  });
}

export async function createIssueNoSchemesCommand() {
  const logs = commonLogger.lastFormatted(5);
  const logsBlock = `\`\`\`json\n${logs}\n\`\`\``;
  await createIssue({
    title: "Sweetpad issue: No schemes",
    body: `Please describe your issue here.\n\n\nLast logs:\n${logsBlock}`,
    labels: ["bug"],
  });
}
