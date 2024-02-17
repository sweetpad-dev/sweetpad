import * as vscode from "vscode";
import { execPrepared } from "../common/exec.js";

type EventData = ToolTreeItem | undefined | null | void;

/**
 * Tree view that helps to install basic ios development tools. It should have inline button to install and check if
 * tools are installed or empty state when it's not installed.
 */
export class ToolTreeProvider implements vscode.TreeDataProvider<ToolTreeItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<EventData>();
  readonly onDidChangeTreeData: vscode.Event<EventData> = this._onDidChangeTreeData.event;

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  getChildren(element?: ToolTreeItem | undefined): vscode.ProviderResult<ToolTreeItem[]> {
    // get elements only for root
    if (!element) {
      return this.getTools();
    }

    return [];
  }

  getTreeItem(element: ToolTreeItem): vscode.TreeItem | Thenable<vscode.TreeItem> {
    return element;
  }

  async getTools(): Promise<ToolTreeItem[]> {
    const items: {
      id: string;
      label: string;
      checkCommand: string;
      installCommand: string;
      documentation: string;
    }[] = [
      {
        id: "brew",
        label: "Homebrew",
        checkCommand: "brew --version",
        installCommand: `/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"`,
        documentation: "https://brew.sh/",
      },
      {
        id: "swift-format",
        label: "swift-format",
        checkCommand: "swift-format --version",
        installCommand: "brew install swift-format",
        documentation: "https://github.com/apple/swift-format",
      },
      {
        id: "xcodegen",
        label: "XcodeGen",
        checkCommand: "xcodegen --version",
        installCommand: "brew install xcodegen",
        documentation: "https://github.com/yonaskolb/XcodeGen",
      },
      {
        id: "swiftlint",
        label: "SwiftLint",
        checkCommand: "swiftlint --version",
        installCommand: "brew install swiftlint",
        documentation: "https://github.com/realm/SwiftLint",
      },
      {
        id: "xcbeautify",
        label: "xcbeautify",
        checkCommand: "xcbeautify --version",
        installCommand: "brew install xcbeautify",
        documentation: "https://github.com/cpisciotta/xcbeautify",
      },
      {
        id: "xcode-build-server",
        label: "Xcode Build Server",
        checkCommand: "xcode-build-server --help",
        installCommand: "brew install xcode-build-server",
        documentation: "https://github.com/SolaWing/xcode-build-server",
      },
    ];
    const results = await Promise.all(
      items.map(async (item) => {
        const { stdout, error } = await execPrepared(item.checkCommand);

        return {
          ...item,
          stdout,
          stderr: error?.message,
        };
      })
    );
    return results.map((item) => {
      return new ToolTreeItem({
        label: item.label,
        collapsibleState: vscode.TreeItemCollapsibleState.None,
        isInstalled: !item.stderr,
        installCommand: item.installCommand,
        documentation: item.documentation,
        provider: this,
      });
    });
  }
}

export class ToolTreeItem extends vscode.TreeItem {
  private provider: ToolTreeProvider;
  installCommand: string;
  documentation: string;

  constructor(options: {
    label: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    isInstalled: boolean;
    documentation: string;
    installCommand: string;
    provider: ToolTreeProvider;
  }) {
    super(options.label, options.collapsibleState);

    this.provider = options.provider;
    this.contextValue = options.isInstalled ? "installed" : "notInstalled";
    this.documentation = options.documentation;
    this.installCommand = options.installCommand;
    if (options.isInstalled) {
      this.iconPath = new vscode.ThemeIcon("check");
    } else {
      this.iconPath = new vscode.ThemeIcon("x");
    }
  }

  refresh() {
    this.provider.refresh();
  }
}
