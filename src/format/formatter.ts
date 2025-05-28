import * as vscode from "vscode";
import { getXcodeVersionInstalled } from "../common/cli/scripts.js";
import { getWorkspaceConfig } from "../common/config.js";
import { exec } from "../common/exec.js";
import { Timer } from "../common/timer.js";
import { formatLogger } from "./logger.js";

/**
 * Register swiftpad as formatter for Swift documents. User then can use
 * "editor.defaultFormatter" in `.vscode/settings.json` to set swiftpad as default formatter
 * for Swift documents.
 */
export function registerFormatProvider(formatter: SwiftFormattingProvider): vscode.Disposable {
  return vscode.languages.registerDocumentFormattingEditProvider("swift", formatter);
}

/**
 * Register swiftpad as range formatter for Swift documents, enabling the
 * "Format Selection" command for swift files.
 */
export function registerRangeFormatProvider(formatter: SwiftFormattingProvider): vscode.Disposable {
  return vscode.languages.registerDocumentRangeFormattingEditProvider("swift", formatter);
}

export class SwiftFormattingProvider
  implements vscode.DocumentFormattingEditProvider, vscode.DocumentRangeFormattingEditProvider
{
  private isBundledSwiftFormat: boolean | undefined = undefined;

  provideDocumentFormattingEdits(document: vscode.TextDocument): vscode.TextEdit[] {
    // if I await this function, vscode won't update the document. i have no idea why :/
    void this.formatDocument(document);
    // we edit directly in the file, so no edits are returned
    return [];
  }

  provideDocumentRangeFormattingEdits(
    document: vscode.TextDocument,
    range: vscode.Range,
    options: vscode.FormattingOptions,
    token: vscode.CancellationToken,
  ): vscode.ProviderResult<vscode.TextEdit[]> {
    void this.formatDocument(document, [range]);
    return [];
  }

  provideDocumentRangesFormattingEdits(
    document: vscode.TextDocument,
    ranges: vscode.Range[],
    options: vscode.FormattingOptions,
    token: vscode.CancellationToken,
  ): vscode.ProviderResult<vscode.TextEdit[]> {
    void this.formatDocument(document, ranges);
    return [];
  }

  /**
   * Checks if swift-format is bundled with Xcode (available since Xcode 16).
   */
  async isSwiftFormatXcodeBundled(): Promise<boolean> {
    if (this.isBundledSwiftFormat !== undefined) {
      return this.isBundledSwiftFormat;
    }

    try {
      const xcodeVersion = await getXcodeVersionInstalled();
      if (xcodeVersion.major >= 16) {
        await exec({ command: "xcrun", args: ["--find", "swift-format"] });
        this.isBundledSwiftFormat = true;
        return true;
      }
    } catch (error) {
      formatLogger.debug("Swift-format not available", { error });
    }

    this.isBundledSwiftFormat = false;
    return false;
  }

  /**
   * Get formatter command parameters from workspace configuration.
   */
  async getFormatterCommand(
    document: vscode.TextDocument,
    ranges: vscode.Range[],
  ): Promise<{
    command: string;
    args: string[];
  }> {
    // User might specify a custom arguments and path for swift-format in the workspace settings.
    const rawArgs = getWorkspaceConfig("format.args");
    const filename = document.fileName;
    let args: string[];
    if (rawArgs && Array.isArray(rawArgs)) {
      // For user supplied arguments, we use "${file}" as a placeholder for the file name
      args = rawArgs.map((arg) => (arg === "${file}" ? filename : arg));
    } else {
      // This is default parameters for swift-format. For different formatter with different parameters,
      // user should specify format.args in the workspace settings.
      args = ["--in-place", filename];
    }

    const path = getWorkspaceConfig("format.path");
    if (path) {
      args.push(...this.getCustomRangeArgument(document, ranges));
      return { command: path, args: args };
    }

    args.push(...this.getSwiftFormatRangeArgument(document, ranges));

    // Since Xcode 16, swift-format is bundled with Xcode. By default, we try to use it.
    // WIKI: xcrun is cli tool that is used to run Xcode tools bundled with Xcode
    if (await this.isSwiftFormatXcodeBundled()) {
      return { command: "xcrun", args: ["swift-format", ...args] };
    }

    // swift-format can be also installed via Homebrew for older Xcode versions
    return { command: "swift-format", args: [...args] };
  }

  private getCustomRangeArgument(document: vscode.TextDocument, ranges: vscode.Range[]): string[] {
    const selectionArgs = getWorkspaceConfig("format.selectionArgs");
    if (!selectionArgs) {
      return [];
    }

    const args: string[] = [];
    for (const range of ranges) {
      const startOffset = document.offsetAt(range.start);
      const endOffset = document.offsetAt(range.end);
      const startLine = document.lineAt(range.start).lineNumber;
      const endLine = document.lineAt(range.end).lineNumber;
      args.push(
        ...selectionArgs.map((arg) =>
          arg
            .replace(/\${startOffset}/g, String(startOffset))
            .replace(/\${endOffset}/g, String(endOffset))
            .replace(/\${startLine}/g, String(startLine))
            .replace(/\${endLine}/g, String(endLine)),
        ),
      );
    }

    return args;
  }

  private getSwiftFormatRangeArgument(document: vscode.TextDocument, ranges: vscode.Range[]): string[] {
    const args: string[] = [];
    for (const range of ranges) {
      const rangeStartOffset = document.offsetAt(range.start);
      const rangeEndOffset = document.offsetAt(range.end);
      args.push("--offsets");
      args.push(`${rangeStartOffset}:${rangeEndOffset}`);
    }
    return args;
  }

  /**
   * Format given document using swift-format executable.
   */
  async formatDocument(document: vscode.TextDocument, ranges?: vscode.Range[]) {
    if (document.languageId !== "swift") {
      return;
    }

    const { command, args } = await this.getFormatterCommand(document, ranges ?? []);

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
        filename: document.fileName,
        execTime: `${timer.elapsed}ms`,
        error: error,
      });
      return;
    }

    formatLogger.log("Code successfully formatted", {
      executable: command,
      args: args,
      filename: document.fileName,
      execTime: `${timer.elapsed}ms`,
    });
  }
}
