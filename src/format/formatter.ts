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

export class SwiftFormattingProvider implements vscode.DocumentFormattingEditProvider, vscode.DocumentRangeFormattingEditProvider {
  
  private isBundledSwiftFormat: boolean | undefined = undefined;

  provideDocumentFormattingEdits(document: vscode.TextDocument): vscode.TextEdit[] {
    // if I await this function, vscode won't update the document. i have no idea why :/
    void this.formatDocument(document);
    // we edit directly in the file, so no edits are returned
    return [];
  }

  provideDocumentRangeFormattingEdits(document: vscode.TextDocument, range: vscode.Range, options: vscode.FormattingOptions, token: vscode.CancellationToken): vscode.ProviderResult<vscode.TextEdit[]> {
    void this.formatDocument(document, [range])
    return []
  }

  provideDocumentRangesFormattingEdits(document: vscode.TextDocument, ranges: vscode.Range[], options: vscode.FormattingOptions, token: vscode.CancellationToken): vscode.ProviderResult<vscode.TextEdit[]> {
    void this.formatDocument(document, ranges)
    return []
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
  async getFormatterCommand(filename: string, offsets: { start: number, end: number }[]): Promise<{
    command: string;
    args: string[];
  }> {
    // User might specify a custom arguments and path for swift-format in the workspace settings.
    const rawArgs = getWorkspaceConfig("format.args");
    let args: string[];
    if (rawArgs && Array.isArray(rawArgs)) {
      // For user supplied arguments, we use "${file}" as a placeholder for the file name
      args = rawArgs.map((arg) => (arg === "${file}" ? filename : arg));
    } else {
      // This is default parameters for swift-format. For different formatter with different parameters,
      // user should specify format.args in the workspace settings.
      args = ["--in-place", filename];
    }

    offsets.forEach(offset => {
      args.push("--offsets")
      args.push(`${offset.start}:${offset.end}`)
    })

    const path = getWorkspaceConfig("format.path");
    if (path) {
      return { command: path, args: args };
    }

    // Since Xcode 16, swift-format is bundled with Xcode. By default, we try to use it.
    // WIKI: xcrun is cli tool that is used to run Xcode tools bundled with Xcode
    if (await this.isSwiftFormatXcodeBundled()) {
      return { command: "xcrun", args: ["swift-format", ...args] };
    }

    // swift-format can be also installed via Homebrew for older Xcode versions
    return { command: "swift-format", args: [...args] };
  }

  /**
   * Format given document using swift-format executable.
   */
  async formatDocument(document: vscode.TextDocument, ranges?: vscode.Range[]) {
    if (document.languageId !== "swift") {
      return;
    }

    var offsets: { start: number, end: number }[] = [];
    if (ranges) {
      ranges.forEach(range => {
        const rangeStartOffset = document.offsetAt(range.start);
        const rangeEndOffset = document.offsetAt(range.end);
        offsets.push({ start: rangeStartOffset, end: rangeEndOffset });
      });
    }

    const filename = document.fileName;
    const { command, args } = await this.getFormatterCommand(filename, offsets);

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
}
