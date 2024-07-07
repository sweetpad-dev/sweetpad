import { selectDestination } from "../build/utils";
import { IosSimulator } from "../common/cli/scripts";
import { CommandExecution } from "../common/commands";
import { IosDevice } from "../common/xcode/devicectl";
import * as vscode from 'vscode';

class DestinationItem implements vscode.QuickPickItem {
    destination: IosDevice | IosSimulator;
    label: string;
    description: string;
    context: IosDevice | IosSimulator; // Add the context property
    iconPath?: vscode.Uri | { light: vscode.Uri; dark: vscode.Uri; } | vscode.ThemeIcon | undefined;
    picked?: boolean | undefined;

    constructor(destination: IosDevice | IosSimulator, isPicked: boolean = false) {
        this.destination = destination;
        this.label = destination.label;
        this.description = destination.udid;
        this.context = destination; // Assign the destination to the context property
        this.iconPath = new vscode.ThemeIcon("device-mobile");
        this.picked = isPicked;
    }
}

export async function pickDestinationCommand(context: CommandExecution) {
    await selectDestination(context.context)
}