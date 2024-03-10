import * as vscode from "vscode";
import { askConfiguration, askScheme, askSimulatorToRunOn, askXcodeWorkspacePath } from "./utils";
import { DEFAULT_SDK, buildApp, runOnDevice, resolveDependencies } from "./commands";
import { ExtensionContext } from "../common/commands";

interface TaskDefinition extends vscode.TaskDefinition {
  type: string;
  action: string;
}

export class XcodeBuildTaskProvider implements vscode.TaskProvider {
  public type = "sweetpad";
  context: ExtensionContext;

  constructor(context: ExtensionContext) {
    this.context = context;
  }

  async provideTasks(token: vscode.CancellationToken): Promise<vscode.Task[]> {
    return [
      this.getTask({
        name: "launch",
        action: "launch",
        details: "Build and Launch the app",
        callback: async () => {
          const scheme = await askScheme();
          const configuration = await askConfiguration(this.context);
          const simulator = await askSimulatorToRunOn(this.context);

          await buildApp(this.context, {
            scheme: scheme,
            sdk: DEFAULT_SDK,
            configuration: configuration,
            shouldBuild: true,
            shouldClean: false,
          });

          await runOnDevice(this.context, {
            scheme: scheme,
            simulator: simulator,
            item: undefined,
            sdk: DEFAULT_SDK,
            configuration: configuration,
          });
        },
      }),
      this.getTask({
        name: "build",
        action: "build",
        details: "Build the app",
        callback: async () => {
          const scheme = await askScheme();
          const configuration = await askConfiguration(this.context);

          await buildApp(this.context, {
            scheme: scheme,
            sdk: DEFAULT_SDK,
            configuration: configuration,
            shouldBuild: true,
            shouldClean: false,
          });
        },
      }),
      this.getTask({
        name: "clean",
        action: "clean",
        details: "Clean the app",
        callback: async () => {
          const scheme = await askScheme();
          const configuration = await askConfiguration(this.context);

          await buildApp(this.context, {
            scheme: scheme,
            sdk: DEFAULT_SDK,
            configuration: configuration,
            shouldBuild: false,
            shouldClean: true,
          });
        },
      }),
      this.getTask({
        name: "resolve-dependencies",
        action: "resolve-dependencies",
        details: "Resolve dependencies",
        callback: async () => {
          const scheme = await askScheme();
          const xcworkspacePath = await askXcodeWorkspacePath(this.context);

          await resolveDependencies({
            scheme: scheme,
            xcodeWorkspacePath: xcworkspacePath,
          });
        },
      }),
    ];
  }

  private getTask(options: {
    name: string;
    action: string;
    details: string;
    callback: () => Promise<void>;
  }): vscode.Task {
    const definition: TaskDefinition = { type: this.type, action: options.action };
    console.log("definition", definition);

    // Task looks like this:
    // -------
    // sweetpad: ${options.name}
    // ${options.details}
    // -------
    const task = new vscode.Task(
      definition,
      vscode.TaskScope.Workspace,
      options.name, // name, after source
      "sweetpad", // source, before name`
      new vscode.CustomExecution(async () => {
        await options.callback();
        return new XcodeBuildTerminal();
      })
    );

    task.detail = options.details;

    return task;
  }

  async resolveTask(_task: vscode.Task, token: vscode.CancellationToken): Promise<vscode.Task | undefined> {
    return _task;
  }
}

class XcodeBuildTerminal implements vscode.Pseudoterminal {
  private writeEmitter = new vscode.EventEmitter<string>();
  private closeEmitter = new vscode.EventEmitter<number>();

  constructor() {}

  onDidWrite = this.writeEmitter.event;
  onDidClose = this.closeEmitter.event;

  open(): void {
    this.writeEmitter.fire("Building and running the app...\r\n");
    this.closeEmitter.fire(0);
  }

  close(): void {
    this.writeEmitter.dispose();
    this.closeEmitter.fire(0);
    this.closeEmitter.dispose();
  }
}
