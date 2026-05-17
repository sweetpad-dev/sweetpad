import { getServerSocketPath } from "../protocol/socket-path";
import { type ParsedArgs, getString } from "./argv";
import { ProtocolClient } from "./protocol";
import { defaultServerEntryPath, ensureServerRunning } from "./spawn";
import { resolveWorkspace } from "./workspace";

export type CommandEnv = {
  /** Working directory used for workspace resolution. Defaults to `process.cwd()`. */
  cwd?: string;
  /** Directory containing `cli.js` (and `server.js`). Used to auto-spawn the server. */
  cliEntryDir: string;
  /**
   * Pre-resolved socket path. When set, the workspace-root cwd-walk and
   * server auto-spawn are skipped — tests use this to point at an in-process
   * server bound to a tmp socket.
   */
  socketPathOverride?: string;
};

/**
 * Resolves the server socket for a command invocation: honors a test
 * override if present, otherwise walks the cwd up to find a workspace,
 * derives the workspace-keyed socket path, and auto-spawns the server.
 *
 * Reads `--workspace=<root>` from args to override the cwd-walk target.
 */
export async function resolveSocketPath(args: ParsedArgs, env: CommandEnv): Promise<string> {
  if (env.socketPathOverride) {
    return env.socketPathOverride;
  }
  const workspaceRootOverride = getString(args, "workspace");
  const cwd = env.cwd ?? process.cwd();
  const workspacePath = resolveWorkspace(workspaceRootOverride ?? cwd);
  const socketPath = getServerSocketPath(workspacePath);
  const serverEntryPath = defaultServerEntryPath(env.cliEntryDir);
  await ensureServerRunning({ socketPath, serverEntryPath, workspacePath });
  return socketPath;
}

/**
 * Connects to the server, runs `fn` with the client, and closes the client
 * even if `fn` throws. Wrap every CLI subcommand's wire interaction with
 * this to keep cleanup correct.
 */
export async function withClient<T>(socketPath: string, fn: (client: ProtocolClient) => Promise<T>): Promise<T> {
  const client = await ProtocolClient.connect(socketPath);
  try {
    return await fn(client);
  } finally {
    client.close();
  }
}
