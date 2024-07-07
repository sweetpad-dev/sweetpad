import * as vscode from 'vscode';
import { ExtensionContext } from "../common/commands.js";

let context: ExtensionContext;
let statusBarTargetPicker: vscode.StatusBarItem;

export function activate(c: ExtensionContext)
{
	context = c;

	setupStatusBarPicker(c);
}

function setupStatusBarPicker(c: ExtensionContext) {
	statusBarTargetPicker = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 0);
	statusBarTargetPicker.command = "sweetpad.pickDestination";
	statusBarTargetPicker.tooltip = "Select Destination for debugging";
	updateStatusBarTargetPicker(c);
	statusBarTargetPicker.show();
}

export function updateStatusBarTargetPicker(c: ExtensionContext)
{	
	const destination = c.getWorkspaceState("build.xcodeDestination");
	statusBarTargetPicker.text = destination?.name ?? destination?.udid ?? "No device selected";
}
