import * as vscode from "vscode";
import { exec } from "../common/exec.js";
import { buildLogger } from "./logger.js";

type ChangeEventData = BuildTreeItem | undefined | null | void;

interface ProjectConfig {
  project: {
    configurations: string[];
    name: string;
    schemes: string[];
    targets: string[];
  };
}

export enum ItemType {
  Project,
  Configuration,
  Scheme,
  Target,
  Destination,
}

export class BuildTreeItem extends vscode.TreeItem {
  private provider: BuildTreeProvider;
  type: ItemType;

  constructor(options: {
    label: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    command?: vscode.Command;
    provider: BuildTreeProvider;
    type: ItemType;
    icon: vscode.ThemeIcon;
  }) {
    super(options.label, options.collapsibleState);
    this.command = options.command;
    this.provider = options.provider;
    this.type = options.type;
    this.iconPath = options.icon;
  }

  refresh() {
    this.provider.refresh();
  }

  async build() {
    // nothing to do here
  }
}

export class SchemeTreeItem extends BuildTreeItem {
  constructor(options: {
    label: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    command?: vscode.Command;
    provider: BuildTreeProvider;
    icon: vscode.ThemeIcon;
  }) {
    super({ ...options, type: ItemType.Scheme });
  }
}

export class DestinationTreeItem extends BuildTreeItem {
  scheme: SchemeTreeItem;

  constructor(options: {
    label: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    command?: vscode.Command;
    provider: BuildTreeProvider;
    icon: vscode.ThemeIcon;
    scheme: SchemeTreeItem;
  }) {
    super({ ...options, type: ItemType.Destination });
    this.scheme = options.scheme;
    this.contextValue = "destination";
  }

  async build() {
    const buildCommand = `xcodebuild -scheme ${this.scheme.label} -destination 'generic/platform=iOS Simulator'`;
    const bootDeviceCommand = `xcrun simctl boot 'iPhone 15'`;
    const openSimulatorCommand = `open -a Simulator`;
    const runSimulatorCommand = `xcrun simctl launch booted 'dev.hyzyla.terminal23'`;

    const commands = [buildCommand, bootDeviceCommand, openSimulatorCommand, runSimulatorCommand];

    // Execute commands one by one
    for (const command of commands) {
      const task = new vscode.Task(
        { type: "shell" },
        vscode.TaskScope.Workspace,
        "Build",
        "xcodebuild",
        new vscode.ShellExecution(command)
      );
      const execution = await vscode.tasks.executeTask(task);
      await new Promise<void>((resolve, reject) => {
        vscode.tasks.onDidEndTaskProcess((e) => {
          if (e.execution === execution) {
            if (e.exitCode === 0) {
              resolve();
            } else {
              reject(new Error(`Task execution failed with exit code ${e.exitCode}`));
            }
          }
        });
      });
    }
  }
}

export class BuildTreeProvider implements vscode.TreeDataProvider<BuildTreeItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<ChangeEventData>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  getChildren(element?: BuildTreeItem | undefined): vscode.ProviderResult<BuildTreeItem[]> {
    if (!element) {
      return this.getSchemes();
    }
    if (element.type === ItemType.Scheme) {
      return this.getDestinations(element);
    }

    return [];
  }

  getTreeItem(element: BuildTreeItem): vscode.TreeItem | Thenable<vscode.TreeItem> {
    return element;
  }

  getDestinations(scheme: SchemeTreeItem) {
    return [
      new DestinationTreeItem({
        label: "iPhone 15",
        collapsibleState: vscode.TreeItemCollapsibleState.None,
        provider: this,
        icon: new vscode.ThemeIcon("device-mobile"),
        scheme,
      }),
    ];
  }

  async getSchemes(): Promise<BuildTreeItem[]> {
    const { stdout, error } = await exec`xcodebuild -list -json`;
    if (error) {
      // todo: add proper error handling
      buildLogger.error("Failed to get build data", { error });
      return [];
    }
    const data = JSON.parse(stdout) as ProjectConfig;

    const items: BuildTreeItem[] = [];

    for (const scheme of data.project.schemes) {
      const item = new BuildTreeItem({
        label: scheme,
        collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
        provider: this,
        type: ItemType.Scheme,
        icon: new vscode.ThemeIcon("symbol-method"),
      });
      items.push(item);
    }

    return items;
  }
}
