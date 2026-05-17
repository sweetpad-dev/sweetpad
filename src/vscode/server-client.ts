import * as path from "node:path";

import * as vscode from "vscode";

import { ProtocolClient } from "../cli/protocol";
import { defaultServerEntryPath, ensureServerRunning } from "../cli/spawn";
import { resolveWorkspace } from "../cli/workspace";
import type { ConfigProvider } from "../core/config/types";
import type { Logger } from "../core/logger/types";
import type { WorkspaceRoot } from "../core/workspace-root";
import { isSuccess } from "../protocol/envelope";
import { ProtocolError } from "../protocol/errors";
import type { MethodName, ParamsFor, ResultFor } from "../protocol/methods";
import { getServerSocketPath } from "../protocol/socket-path";
import type { AttachRequestParams, BuildEvent, WireResponse } from "../protocol/types";

/**
 * Thin VS Code-side wrapper around `ProtocolClient` that manages
 * connection + auto-spawn lifecycle for the extension. One connection per
 * call — the underlying `ProtocolClient` is single-request, and re-opening
 * is cheap (the server stays warm for 5 minutes between requests).
 *
 * Server mode is opt-in via `sweetpad.system.experimental.serverMode`. The
 * client itself doesn't gate on the flag — that's the caller's choice. The
 * extension constructs one `ServerClient` regardless and only routes
 * through it when the flag is on.
 */
export class ServerClient implements vscode.Disposable {
  constructor(
    private readonly deps: {
      logger: Logger;
      workspaceRoot: WorkspaceRoot;
      config: ConfigProvider;
      /** Directory the extension was loaded from. `server.js` is expected next to it. */
      extensionOutDir: string;
    },
  ) {}

  /**
   * Resolve workspace + socket path, ensure the server is up, send one
   * request, return the typed response. Caller decides whether the
   * envelope is success or error.
   */
  async request<M extends MethodName>(method: M, params: ParamsFor<M>): Promise<WireResponse<ResultFor<M>>> {
    const socketPath = await this.prepareSocket();
    const client = await ProtocolClient.connect(socketPath);
    try {
      return await client.request(method, params);
    } finally {
      client.close();
    }
  }

  /**
   * Same as `request` but throws `ProtocolError` on error envelopes. Use
   * this when the caller doesn't want to switch on `ok`.
   */
  async requestOrThrow<M extends MethodName>(method: M, params: ParamsFor<M>): Promise<ResultFor<M>> {
    const response = await this.request(method, params);
    if (!isSuccess(response)) {
      throw new ProtocolError(response.error.code, response.error.message, {
        hint: response.error.hint,
      });
    }
    return response.data;
  }

  /**
   * Streaming attach. Resolves to the error envelope when the server bails
   * before any events, or `null` after a clean stream-then-close.
   */
  async attach(params: AttachRequestParams, onEvent: (event: BuildEvent) => void): Promise<WireResponse | null> {
    const socketPath = await this.prepareSocket();
    const client = await ProtocolClient.connect(socketPath);
    try {
      return await client.attach(params, onEvent);
    } finally {
      client.close();
    }
  }

  private async prepareSocket(): Promise<string> {
    const workspacePath = resolveWorkspace(this.deps.workspaceRoot.getPath());
    const socketPath = getServerSocketPath(workspacePath);
    const serverEntryPath = defaultServerEntryPath(this.deps.extensionOutDir);
    await ensureServerRunning({ socketPath, serverEntryPath, workspacePath });
    return socketPath;
  }

  dispose(): void {
    // No persistent state today — each request opens and closes its own
    // connection. Kept as a Disposable so future iterations (event
    // subscriptions, pooled sockets) have a place to clean up.
  }
}

/** Reads the feature flag the way the extension consistently does. */
export function isServerModeEnabled(config: ConfigProvider): boolean {
  return config.get("system.experimental.serverMode") === true;
}

/** Resolve the directory holding the extension's bundled `out/` files (so we can find `server.js`). */
export function extensionOutDirFromContext(context: vscode.ExtensionContext): string {
  // The extension activates from `<extension>/out/extension.js`. The
  // server bundle lives alongside it as `<extension>/out/server.js`.
  return path.join(context.extensionPath, "out");
}
