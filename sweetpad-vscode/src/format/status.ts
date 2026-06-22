import * as vscode from "vscode";
import { Command } from "vscode-languageclient/node";

/**
 * Create a language status item for Swift language.
 *
 * Language status item is a small indicator in the status bar on the right side of the editor,
 * under "{}" icon.
 *
 * https://code.visualstudio.com/api/references/vscode-api?ref=trap.jp#LanguageStatusItem
 */
export function createFormatStatusItem(): vscode.LanguageStatusItem {
  const languageStatusItem = vscode.languages.createLanguageStatusItem("swift", "swift");
  languageStatusItem.name = "SweetPad";
  languageStatusItem.text = "SweetPad: Format";
  languageStatusItem.command = Command.create("Open logs", "sweetpad.format.showLogs");
  return languageStatusItem;
}
