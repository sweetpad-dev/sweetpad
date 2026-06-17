import { type ChildProcess, spawn } from "node:child_process";

import * as vscode from "vscode";

import { getWorkspacePath } from "../build/utils.js";
import { ExtensionError } from "../common/errors.js";
import { commonLogger } from "../common/logger.js";
import { getShellEnv } from "../common/tasks/shell-env.js";
import type { SimulatorDestination } from "./types.js";

/**
 * Default port that `serve-sim` uses for its preview UI. When several streams
 * run at once we hand out `BASE_PREVIEW_PORT`, `BASE_PREVIEW_PORT + 1`, … so
 * each simulator gets its own server.
 */
const BASE_PREVIEW_PORT = 3200;

/** How long we wait for `serve-sim` to print its "ready" line before giving up. */
const READY_TIMEOUT_MS = 30_000;

type ServeSimStream = {
  udid: string;
  name: string;
  port: number;
  /** Preview URL on the extension host, e.g. `http://localhost:3200`. */
  url: string;
  process: ChildProcess;
  panel?: vscode.WebviewPanel;
};

/**
 * Owns the lifecycle of [`serve-sim`](https://github.com/EvanBacon/serve-sim)
 * helpers — "the `npx serve` of Apple Simulators". For a given simulator it
 * spawns `npx serve-sim "<name>" --port <port>`, which streams the simulator
 * framebuffer as MJPEG with a WebSocket control channel, then surfaces that
 * preview either inside a VS Code webview or via the browser.
 *
 * One helper process is kept per simulator UDID and reused across the
 * "Stream", "Open in Browser" and "Copy URL" commands. Helpers are killed when
 * their webview panel is closed or when the extension is disposed.
 */
export class ServeSimManager implements vscode.Disposable {
  private streams = new Map<string, ServeSimStream>();

  /**
   * `serve-sim` only runs on Apple Silicon Macs. SweetPad is macOS-only
   * already, so here we just guard against Intel, where the helper can't start.
   */
  private assertSupported(): void {
    if (process.platform !== "darwin" || process.arch !== "arm64") {
      throw new ExtensionError(
        "serve-sim requires an Apple Silicon (arm64) Mac. Use the 'Open simulator' command to launch Simulator.app instead.",
      );
    }
  }

  /** Open (or focus) the in-editor webview that mirrors the simulator. */
  async stream(simulator: SimulatorDestination): Promise<void> {
    const stream = await this.getOrStart(simulator);
    await this.reveal(stream);
  }

  /** Open the live preview in the user's default browser. */
  async openInBrowser(simulator: SimulatorDestination): Promise<void> {
    const stream = await this.getOrStart(simulator);
    const externalUri = await vscode.env.asExternalUri(vscode.Uri.parse(stream.url));
    await vscode.env.openExternal(externalUri);
  }

  /** Copy the (forwarded) preview URL to the clipboard. */
  async copyUrl(simulator: SimulatorDestination): Promise<void> {
    const stream = await this.getOrStart(simulator);
    const externalUri = await vscode.env.asExternalUri(vscode.Uri.parse(stream.url));
    await vscode.env.clipboard.writeText(externalUri.toString());
    void vscode.window.showInformationMessage(`SweetPad: Copied stream URL for "${stream.name}"`);
  }

  /** Reuse the running helper for this simulator, or start a fresh one. */
  private async getOrStart(simulator: SimulatorDestination): Promise<ServeSimStream> {
    const existing = this.streams.get(simulator.udid);
    if (existing) {
      return existing;
    }
    this.assertSupported();
    return await this.spawn(simulator);
  }

  /** Pick the lowest preview port not already used by one of our streams. */
  private pickPort(): number {
    const used = new Set([...this.streams.values()].map((stream) => stream.port));
    let port = BASE_PREVIEW_PORT;
    while (used.has(port)) {
      port++;
    }
    return port;
  }

  private async spawn(simulator: SimulatorDestination): Promise<ServeSimStream> {
    const port = this.pickPort();
    const env = await getShellEnv();

    // `npx serve-sim "<device name>" --port <port>` boots/attaches to the
    // simulator and serves the preview. We keep the child running and watch its
    // output to know when the preview is reachable. The resolved shell env gives
    // the child the same PATH as the user's terminal, so `npx`/`node` are found.
    const child = spawn("npx", ["serve-sim", simulator.name, "--port", String(port)], {
      cwd: getWorkspacePath(),
      env: env,
    });

    const stream: ServeSimStream = {
      udid: simulator.udid,
      name: simulator.name,
      port: port,
      url: `http://localhost:${port}`,
      process: child,
    };
    this.streams.set(simulator.udid, stream);

    // If the helper dies on its own (crash, simulator shut down, `serve-sim
    // --kill`, …) drop it from the map and close any open panel.
    child.on("exit", (code) => {
      commonLogger.debug("serve-sim helper exited", { name: simulator.name, code: code });
      this.streams.delete(simulator.udid);
      stream.panel?.dispose();
    });
    child.on("error", (error) => {
      commonLogger.error("serve-sim helper failed", { name: simulator.name, error: error });
    });

    try {
      await this.waitUntilReady(child, simulator.name);
    } catch (error) {
      child.kill();
      this.streams.delete(simulator.udid);
      throw new ExtensionError(`Failed to start serve-sim for "${simulator.name}"`, {
        context: { error: error instanceof Error ? error.message : String(error) },
      });
    }

    return stream;
  }

  /**
   * Resolve once `serve-sim` reports a reachable URL on its output, reject if
   * it exits first, and fall back to assuming readiness after a timeout (the
   * port is fixed via `--port`, so the URL is known regardless).
   */
  private waitUntilReady(child: ChildProcess, name: string): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      let settled = false;
      const finish = (action: () => void) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        action();
      };

      const timer = setTimeout(() => finish(resolve), READY_TIMEOUT_MS);

      const onData = (chunk: Buffer) => {
        const text = chunk.toString();
        commonLogger.debug("serve-sim output", { name: name, output: text });
        if (/https?:\/\/(localhost|127\.0\.0\.1):\d+/.test(text)) {
          finish(resolve);
        }
      };
      child.stdout?.on("data", onData);
      child.stderr?.on("data", onData);

      child.on("error", (error) => finish(() => reject(error)));
      child.on("exit", (code) => finish(() => reject(new Error(`serve-sim exited early (code ${code})`))));
    });
  }

  private async reveal(stream: ServeSimStream): Promise<void> {
    if (stream.panel) {
      stream.panel.reveal(vscode.ViewColumn.Beside);
      return;
    }

    const panel = vscode.window.createWebviewPanel(
      "sweetpad.serveSim",
      `📱 ${stream.name}`,
      { viewColumn: vscode.ViewColumn.Beside, preserveFocus: false },
      { enableScripts: true, retainContextWhenHidden: true },
    );
    stream.panel = panel;

    // Closing the panel stops the underlying helper — the stream's lifetime is
    // tied to the view that shows it.
    panel.onDidDispose(() => {
      stream.panel = undefined;
      this.stop(stream.udid);
    });

    const externalUri = await vscode.env.asExternalUri(vscode.Uri.parse(stream.url));
    panel.webview.html = renderWebview(externalUri.toString());
  }

  /** Stop and forget the helper for a single simulator. */
  private stop(udid: string): void {
    const stream = this.streams.get(udid);
    if (!stream) return;
    this.streams.delete(udid);
    stream.process.kill();
  }

  dispose(): void {
    for (const stream of this.streams.values()) {
      stream.panel?.dispose();
      stream.process.kill();
    }
    this.streams.clear();
  }
}

/**
 * Full-bleed webview that embeds the serve-sim preview in an iframe. The
 * preview is plain HTTP on the host; `asExternalUri` upgrades it to a
 * forwarded https URL in remote/Codespaces sessions, which the CSP allows.
 */
function renderWebview(url: string): string {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <meta
    http-equiv="Content-Security-Policy"
    content="default-src 'none'; style-src 'unsafe-inline'; frame-src https: http://localhost:* http://127.0.0.1:*;"
  />
  <style>
    html, body { margin: 0; padding: 0; height: 100%; background: #000; }
    iframe { border: 0; width: 100%; height: 100vh; display: block; }
  </style>
</head>
<body>
  <iframe
    src="${url}"
    allow="fullscreen; clipboard-read; clipboard-write"
    sandbox="allow-scripts allow-same-origin allow-forms allow-pointer-lock"
  ></iframe>
</body>
</html>`;
}
