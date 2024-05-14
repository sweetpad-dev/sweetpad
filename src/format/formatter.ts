import * as vscode from "vscode";
import { exec } from "../common/exec.js";
import { formatLogger } from "./logger.js";
import { Timer } from "../common/timer.js";
import { getWorkspaceConfig } from "../common/config.js";

/**
 * Get path to formatter executable from user settings.
 */
function getFormatterPath(): string {
  const path = getWorkspaceConfig("format.path");
  return path ?? "swift-format";
}

/**
 * Get args for the formatter executable from user settings.
 */
function getFormatterArgs(): string[] {
  const args: string[] | undefined = getWorkspaceConfig("format.args");

  return args ?? [
    "--in-place",
  ];
}

/**
 * Format given document using swift-format executable.
 */
export async function formatDocument(document: vscode.TextDocument) {
  if (document.languageId !== "swift") {
    return;
  }
  const executable = getFormatterPath();
  const args = getFormatterArgs();

  const filename = document.fileName;

  const timer = new Timer();
  try {
    await exec({
      command: executable,
      args: [...args, filename],
    });
  } catch (error) {
    formatLogger.error("Failed to format code", {
      executable: executable,
      filename: filename,
      execTime: `${timer.elapsed}ms`,
      error: error,
    });
    return;
  }

  formatLogger.log("Code successfully formatted", {
    executable: executable,
    filename: filename,
    execTime: `${timer.elapsed}ms`,
  });
}
