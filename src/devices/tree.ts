import * as vscode from "vscode";
import { ExtensionContext } from "../common/commands.js";
import { DevicesManager } from "./manager.js";

export class DeviceTreeItem extends vscode.TreeItem {
  udid: string;
  state: "connected" | "disconnected" | "unavailable";
  private provider: DevicesTreeProvider;

  constructor(options: {
    label: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    command?: vscode.Command;
    udid: string;
    state: "connected" | "disconnected" | "unavailable";
    provider: DevicesTreeProvider;
  }) {
    super(options.label, options.collapsibleState);

    this.command = options.command;
    this.udid = options.udid;
    this.state = options.state;
    this.contextValue = this.state;
    this.provider = options.provider;

    if (this.state === "connected") {
      this.iconPath = new vscode.ThemeIcon("device-mobile");
    }
    if (this.state === "disconnected") {
      this.iconPath = new vscode.ThemeIcon("debug-disconnect");
    }
    if (this.state === "unavailable") {
      this.iconPath = new vscode.ThemeIcon("circle-slash");
    }
  }

  refresh() {
    this.provider.manager.refresh();
  }
}

export class DevicesTreeProvider implements vscode.TreeDataProvider<DeviceTreeItem> {
  public manager: DevicesManager;
  private welcomeScreen: "no-devicectl" | "no-devices" | null = null;

  private _onDidChangeTreeData = new vscode.EventEmitter<DeviceTreeItem | undefined | null | void>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  constructor(options: { manager: DevicesManager }) {
    this.manager = options.manager;
    this.manager.on("refresh", () => {
      this._onDidChangeTreeData.fire();
    });
    this.welcomeScreen = null;
  }

  refresh() {
    void this.manager.refresh();
  }

  getChildren(element?: DeviceTreeItem | undefined): vscode.ProviderResult<DeviceTreeItem[]> {
    // get elements only for root
    if (!element) {
      return this.getDevices();
    }

    return [];
  }

  getTreeItem(element: DeviceTreeItem): vscode.TreeItem | Thenable<vscode.TreeItem> {
    return element;
  }

  async getDevices(): Promise<DeviceTreeItem[]> {
    const devices = await this.manager.getDevices();

    if (this.manager.failed === "no-devicectl") {
      // Display welcome screen with explanation what to do.
      // See "viewsWelcome": [ {"view": "sweetpad.devices.noDevicectl", ...} ] in package.json
      vscode.commands.executeCommand("setContext", "sweetpad.devices.noDevicectl", true);
      this.welcomeScreen = "no-devicectl";
      return [];
    } else if (this.welcomeScreen == "no-devicectl") {
      // Remove welcome screen, when devicectl becomes available.
      vscode.commands.executeCommand("setContext", "sweetpad.devices.noDevicectl", undefined);
      this.welcomeScreen = null;
    }

    if (devices.length === 0) {
      // Display welcome screen with explanation what to do.
      // See "viewsWelcome": [ {"view": "sweetpad.devices.noDevices", ...} ] in package.json
      vscode.commands.executeCommand("setContext", "sweetpad.devices.noDevices", true);
      this.welcomeScreen = "no-devices";
      return [];
    } else if (this.welcomeScreen == "no-devices") {
      // Remove welcome screen, when devices become available.
      vscode.commands.executeCommand("setContext", "sweetpad.devices.noDevices", undefined);
      this.welcomeScreen = null;
    }

    return devices.map((device) => {
      return new DeviceTreeItem({
        label: device.label,
        collapsibleState: vscode.TreeItemCollapsibleState.None,
        udid: device.udid,
        state: device.state,
        provider: this,
      });
    });
  }
}
