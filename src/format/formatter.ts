import * as vscode from "vscode";
import { exec } from "../common/exec.js";
import { formatLogger } from "./logger.js";
import { Timer } from "../common/timer.js";
import { getWorkspaceConfig } from "../common/config.js";

/**
 * Get path to swift-format executable from user settings.
 */
function getSwiftFormatPath(): string {
  const path = getWorkspaceConfig("format.path");
  return path ?? "swift-format";
}

/**
 * Format given document using swift-format executable.
 */
export async function formatDocument(document: vscode.TextDocument) {
  if (document.languageId !== "swift") {
    return;
  }
  const executable = getSwiftFormatPath();

  const filename = document.fileName;

  const timer = new Timer();
  try {
    await exec({
      command: executable,
      args: ["--in-place", filename],
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
