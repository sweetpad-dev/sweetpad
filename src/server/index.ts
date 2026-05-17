import { BuildManager } from "../core/build/manager";
import { DestinationsManager } from "../core/destination/manager";
import { DevicesManager } from "../core/devices/manager";
import { noopLspRefresher } from "../core/lsp/types";
import { noopNotifier } from "../core/notifier/types";
import { noopProgressReporter } from "../core/progress";
import { SimulatorsManager } from "../core/simulators/manager";
import { getServerLockfilePath, getServerSocketPath } from "../protocol/socket-path";
import { CliAsker } from "./adapters/cli-asker";
import { CliWorkspaceRoot } from "./adapters/cli-workspace-root";
import { FileWorkspaceState } from "./adapters/file-workspace-state";
import { JsonConfigProvider } from "./adapters/json-config";
import { JsonDiagnosticsCollector } from "./adapters/json-diagnostics";
import { NodeTaskRunner } from "./adapters/node-task-runner";
import { MethodDispatcher } from "./dispatcher";
import { Listener } from "./listener";
import { removeLockfile, tryAcquireLock, writeLockfile } from "./lockfile";
import { StderrJsonLogger } from "./logger";
import { createBuildMethod } from "./methods/build";
import { BuildRegistry } from "./registry";

const DEFAULT_IDLE_TIMEOUT_MS = 5 * 60 * 1000;

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2));
  if (args.help || args.command === undefined) {
    printUsage();
    process.exit(args.help ? 0 : 2);
  }

  if (args.command !== "start") {
    process.stderr.write(`unknown subcommand: ${args.command}\n`);
    printUsage();
    process.exit(2);
  }

  const logger = new StderrJsonLogger(args.logLevel);
  const workspaceRoot = new CliWorkspaceRoot(args.workspace ?? process.cwd());
  const workspacePath = workspaceRoot.getPath();
  logger.log("Resolved workspace", { workspacePath });

  const socketPath = getServerSocketPath(workspacePath);
  const lockfilePath = getServerLockfilePath(workspacePath);
  const lock = tryAcquireLock(lockfilePath);
  if (lock.status === "locked") {
    process.stderr.write(
      `another sweetpad-server (pid ${lock.holder.pid}) holds the workspace lock at ${socketPath}\n`,
    );
    process.exit(2);
  }

  const config = new JsonConfigProvider();
  const state = new FileWorkspaceState(workspacePath);
  const asker = new CliAsker();
  const diagnostics = new JsonDiagnosticsCollector();
  const taskRunner = new NodeTaskRunner({ workspaceRoot, config, logger });

  const devicesManager = new DevicesManager({ logger, workspaceRoot });
  const simulatorsManager = new SimulatorsManager({ logger, config, workspaceRoot });
  const destinationsManager = new DestinationsManager({
    simulatorsManager,
    devicesManager,
    workspace: state,
  });

  const buildManager = new BuildManager({
    logger,
    config,
    state,
    asker,
    progress: noopProgressReporter,
    taskRunner,
    notifier: noopNotifier,
    lsp: noopLspRefresher,
    destinations: destinationsManager,
    diagnostics,
    workspaceRoot,
  });
  await buildManager.start();

  const registry = new BuildRegistry();
  const dispatcher = new MethodDispatcher(logger);
  dispatcher.register(
    "build",
    createBuildMethod({
      buildManager,
      destinationsManager,
      registry,
      diagnostics,
      workspaceRoot,
      config,
      state,
    }),
  );

  const idleTimeoutMs = readIdleTimeout();
  let idleTimer: NodeJS.Timeout | undefined;
  const startIdleTimer = () => {
    clearIdleTimer();
    if (idleTimeoutMs <= 0) return;
    idleTimer = setTimeout(() => {
      // Last-second check before shutting down — a build can be in-flight even
      // when no client is connected (the originator may have disconnected
      // while waiting for the response).
      if (registry.running().length > 0) {
        startIdleTimer();
        return;
      }
      logger.log("Idle timeout reached, shutting down");
      void shutdown(0);
    }, idleTimeoutMs);
  };
  const clearIdleTimer = () => {
    if (idleTimer) {
      clearTimeout(idleTimer);
      idleTimer = undefined;
    }
  };

  const listener = new Listener({
    socketPath,
    dispatcher,
    logger,
    onActiveChange: (n) => {
      if (n === 0) startIdleTimer();
      else clearIdleTimer();
    },
  });

  try {
    await listener.listen();
  } catch (error) {
    process.stderr.write(`failed to listen on ${socketPath}: ${error instanceof Error ? error.message : String(error)}\n`);
    process.exit(2);
  }

  writeLockfile(lockfilePath, {
    pid: process.pid,
    socketPath,
    startedAt: new Date().toISOString(),
  });
  logger.log("Server listening", { socketPath, pid: process.pid });

  // Start the idle timer immediately — if no client connects shortly after
  // spawn, we don't want to linger forever.
  startIdleTimer();

  let shuttingDown = false;
  const shutdown = async (exitCode: number): Promise<void> => {
    if (shuttingDown) return;
    shuttingDown = true;
    clearIdleTimer();
    logger.log("Shutting down", { exitCode });
    try {
      await listener.close();
    } catch (error) {
      logger.warn("Error closing listener", { error });
    }
    removeLockfile(lockfilePath);
    process.exit(exitCode);
  };

  process.once("SIGINT", () => void shutdown(0));
  process.once("SIGTERM", () => void shutdown(0));
}

type Args = {
  command: "start" | undefined;
  workspace: string | undefined;
  logLevel: "debug" | "info" | "warning" | "error";
  help: boolean;
};

function parseArgs(argv: string[]): Args {
  const result: Args = { command: undefined, workspace: undefined, logLevel: "info", help: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "-h" || arg === "--help") {
      result.help = true;
    } else if (arg === "start") {
      result.command = "start";
    } else if (arg.startsWith("--workspace=")) {
      result.workspace = arg.slice("--workspace=".length);
    } else if (arg === "--workspace") {
      result.workspace = argv[++i];
    } else if (arg.startsWith("--log-level=")) {
      const lvl = arg.slice("--log-level=".length);
      if (lvl === "debug" || lvl === "info" || lvl === "warning" || lvl === "error") {
        result.logLevel = lvl;
      }
    }
  }
  return result;
}

function printUsage(): void {
  process.stderr.write(
    `usage: sweetpad-server start [--workspace=<path>] [--log-level=debug|info|warning|error]\n`,
  );
}

function readIdleTimeout(): number {
  const raw = process.env.SWEETPAD_IDLE_TIMEOUT_MS;
  if (!raw) return DEFAULT_IDLE_TIMEOUT_MS;
  const parsed = Number.parseInt(raw, 10);
  return Number.isFinite(parsed) ? parsed : DEFAULT_IDLE_TIMEOUT_MS;
}

main().catch((error) => {
  process.stderr.write(`sweetpad-server failed: ${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
  process.exit(1);
});
