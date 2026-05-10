import { createConnection } from "node:net";
import { quote } from "shell-quote";
import * as vscode from "vscode";
import { getWorkspaceConfig } from "../common/config.js";
import { exec } from "../common/exec.js";

const PROBE_TIMEOUT_MS = 500;
const TERMINAL_NAME = "iOS Tunnel";
// pymobiledevice3's default tunneld REST API port.
const TUNNELD_PORT = 49_151;

/**
 * Manages `sudo pymobiledevice3 remote tunneld` in a foreground integrated terminal.
 * Fire-and-forget: errors and the sudo prompt surface in the terminal, not here. The
 * TCP probe only exists to adopt an existing tunneld (this window, another window, or
 * a leftover from a previous session) instead of spawning a duplicate.
 */
export class TunnelManager implements vscode.Disposable {
  private terminal: vscode.Terminal | undefined;
  private terminalListener: vscode.Disposable;
  // Cached while a start() pass is mid-flight so concurrent callers see the same probe/spawn
  // instead of racing past the probe and creating two terminals. Cleared once the call returns.
  private starting: Promise<void> | undefined;

  constructor() {
    this.terminalListener = vscode.window.onDidCloseTerminal((closed) => {
      if (closed === this.terminal) {
        this.terminal = undefined;
      }
    });
  }

  /** Opt-in start gated by `build.deviceTunnelAutoStart`. */
  async autoStart(): Promise<void> {
    if (!getWorkspaceConfig("build.deviceTunnelAutoStart")) return;
    await this.start();
  }

  async start(): Promise<void> {
    if (this.starting) return this.starting;
    this.starting = this.startInner().finally(() => {
      this.starting = undefined;
    });
    return this.starting;
  }

  private async startInner(): Promise<void> {
    if (await this.probe()) {
      return;
    }

    if (this.terminal) {
      this.terminal.show(true);
      return;
    }

    const pymd3 = getWorkspaceConfig("build.pymobiledevice3Path") ?? "pymobiledevice3";
    // Pymd3Sidecar surfaces the missing-binary message during launch with install hints,
    // so flagging it here too would just be noise.
    if (!(await isPymobiledevice3Available(pymd3))) {
      return;
    }
    this.terminal = vscode.window.createTerminal({
      name: TERMINAL_NAME,
      iconPath: new vscode.ThemeIcon("broadcast"),
    });
    this.terminal.show(true);
    // sudo is required for the utun interface. Quoted so a pymd3 path with spaces
    // doesn't split into separate shell tokens.
    this.terminal.sendText(quote(["sudo", pymd3, "remote", "tunneld"]), true);
  }

  dispose(): void {
    this.terminalListener.dispose();
    // Don't dispose the terminal — VS Code tears it down on reload, which sends SIGHUP
    // to tunneld via the pty, same as a manual close.
  }

  private probe(): Promise<boolean> {
    return new Promise((resolve) => {
      const socket = createConnection({ host: "127.0.0.1", port: TUNNELD_PORT });
      let settled = false;
      const finish = (ok: boolean) => {
        if (settled) return;
        settled = true;
        socket.destroy();
        resolve(ok);
      };
      const timeout = setTimeout(() => finish(false), PROBE_TIMEOUT_MS);
      socket.once("connect", () => {
        clearTimeout(timeout);
        finish(true);
      });
      socket.once("error", () => {
        clearTimeout(timeout);
        finish(false);
      });
    });
  }
}

async function isPymobiledevice3Available(binaryPath: string): Promise<boolean> {
  try {
    await exec({ command: binaryPath, args: ["version"] });
    return true;
  } catch {
    return false;
  }
}
