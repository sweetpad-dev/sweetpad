import * as vscode from "vscode";
import { exec } from "../common/exec.js";
import { formatLogger } from "./logger.js";
import { Timer } from "../common/timer.js";
import { getWorkspaceConfig } from "../common/config.js";

/**
 * Get formatter command parameters from workspace configuration.
 */
function getFormatterCommand(filename: string): {
  command: string;
  args: string[];
} {
  const path = getWorkspaceConfig("format.path");

  // We use "swift-format" as default command if no path is provided,
  // "args" config are ignored in this case
  if (!path) {
    return {
      command: "swift-format",
      args: ["--in-place", filename],
    };
  }

  // By default we use "swift-format" arguments
  const args: string[] | undefined = getWorkspaceConfig("format.args") ?? ["--in-place", "${file}"];
  const replacedArgs = args.map((arg) => (arg === "${file}" ? filename : arg));

  return {
    command: path,
    args: replacedArgs,
  };
}

/**
 * Format given document using swift-format executable.
 */
export async function formatDocument(document: vscode.TextDocument) {
  if (document.languageId !== "swift") {
    return;
  }

  const filename = document.fileName;
  const { command, args } = getFormatterCommand(filename);

  const timer = new Timer();
  try {
    await exec({
      command: command,
      args: args,
    });
  } catch (error) {
    formatLogger.error("Failed to format code", {
      executable: command,
      args: args,
      filename: filename,
      execTime: `${timer.elapsed}ms`,
      error: error,
    });
    return;
  }

  formatLogger.log("Code successfully formatted", {
    executable: command,
    args: args,
    filename: filename,
    execTime: `${timer.elapsed}ms`,
  });
}
