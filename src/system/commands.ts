import * as vscode from "vscode";
import type { ExtensionContext } from "../common/commands";
import { commonLogger } from "../common/logger";

export async function resetSweetpadCache(context: ExtensionContext) {
  context.updateProgressStatus("Resetting SweetPad cache");
  context.resetWorkspaceState();
  vscode.window.showInformationMessage("SweetPad cache has been reset");
}

async function createIssue(options: { title: string; body: string; labels: string[] }) {
  const url = new URL("https://github.com/sweetpad-dev/sweetpad/issues/new");
  url.searchParams.append("title", options.title);
  url.searchParams.append("body", options.body);
  url.searchParams.append("labels", options.labels.join(","));

  vscode.env.openExternal(vscode.Uri.parse(url.toString()));
}

export async function createIssueGenericCommand(context: ExtensionContext) {
  await createIssue({
    title: "SweetPad issue",
    body: "Please describe your issue here",
    labels: ["bug"],
  });
}

export async function createIssueNoSchemesCommand() {
  const logs = commonLogger.lastFormatted(5);
  const logsBlock = `\`\`\`json\n${logs}\n\`\`\``;
  await createIssue({
    title: "SweetPad issue: No schemes",
    body: `Please describe your issue here.\n\n\nLast logs:\n${logsBlock}`,
    labels: ["bug"],
  });
}

export async function testErrorReportingCommand() {
  commonLogger.log("Testing error reporting", {
    contextKey: "Context value",
  });
  throw new Error("This is a test error");
}

export async function openTerminalPanel() {
  vscode.window.terminals.at(-1)?.show();
}
