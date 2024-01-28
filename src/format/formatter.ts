import * as vscode from "vscode";
import { exec } from "../common/exec.js";
import { formatLogger } from "./logger.js";

/**
 * Get path to swift-format executable from user settings.
 */
function getSwiftFormatPath() {
  const config = vscode.workspace.getConfiguration("sweetpad");
  return config.get("format.path") ?? "swift-format";
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
  const { error, time } = await exec`${executable} --in-place ${filename}`;

  if (error) {
    formatLogger.error("Failed to format code", {
      executable: executable,
      filename: filename,
      execTime: `${time}ms`,
      error: error,
    });
    return;
  }
  formatLogger.log("Code successfully formatted", {
    executable: executable,
    filename: filename,
    execTime: `${time}ms`,
  });
}
