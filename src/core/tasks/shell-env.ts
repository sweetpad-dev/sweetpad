import { spawn } from "node:child_process";
import * as crypto from "node:crypto";

import type { Logger } from "../logger/types";

// Spawns the user's login + interactive shell, asks it to dump its resolved
// environment, and caches the result for the rest of the process lifetime.
// Design goals:
//   - Tolerate arbitrary stdout chatter from dotfiles (banners, MOTD,
//     oh-my-zsh update prompts, fortune, etc.). We do this with a uuid
//     delimiter pair: anything printed before the start marker or after the
//     end marker is discarded.
//   - Don't invoke node inside the shell. Use POSIX `command env` instead —
//     no nested quoting, no ELECTRON_RUN_AS_NODE dance, no Electron-binary
//     edge cases.
//   - Hard timeout so a hung dotfile can't stall the host.
//
// Reference for the sentinel approach: sindresorhus/shell-env.

const DEFAULT_TIMEOUT_MS = 5000;

// Electron sets these on any node-ish child it spawns. They must not leak into
// tool children we later spawn via the task terminal.
const ELECTRON_ONLY_VARS = new Set(["ELECTRON_RUN_AS_NODE", "ELECTRON_NO_ATTACH_CONSOLE"]);

export type ShellEnvOptions = {
  logger: Logger;
  /** Override shell binary (defaults to $SHELL or /bin/sh). */
  shell?: string | null;
  /** Probe timeout in ms; the shell is SIGKILLed past this. */
  timeoutMs?: number;
  /** Working directory for the probe shell. Picks up directory-scoped envs (mise, direnv). */
  cwd?: string;
  /** Called once per cache cycle with a user-facing message when the probe fails. */
  onWarning?: (message: string) => void;
};

let cachedPromise: Promise<NodeJS.ProcessEnv> | null = null;
let notifiedFailureThisCycle = false;

export function getShellEnv(options: ShellEnvOptions): Promise<NodeJS.ProcessEnv> {
  if (!cachedPromise) {
    cachedPromise = resolveShellEnv(options).catch((error) => {
      options.logger.warn("Shell environment resolution failed; falling back to process.env", {
        error: error instanceof Error ? error.message : String(error),
      });
      maybeNotifyShellEnvFailure(error, options);
      return { ...process.env };
    });
  }
  return cachedPromise;
}

export async function refreshShellEnv(options: ShellEnvOptions): Promise<NodeJS.ProcessEnv> {
  cachedPromise = null;
  notifiedFailureThisCycle = false;
  return getShellEnv(options);
}

/**
 * Eagerly kicks off shell env resolution so the first task doesn't wait.
 * Call this from extension activate() or CLI bootstrap.
 */
export function warmShellEnv(options: ShellEnvOptions): void {
  void getShellEnv(options);
}

async function resolveShellEnv(options: ShellEnvOptions): Promise<NodeJS.ProcessEnv> {
  const shell = options.shell || process.env.SHELL || "/bin/sh";
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const cwd = options.cwd;

  const uuid = crypto.randomUUID();
  const startMarker = `__SW_ENV_START_${uuid}__`;
  const endMarker = `__SW_ENV_END_${uuid}__`;

  // -i (interactive) + -l (login) ensures both the "login" dotfile group
  // (.zprofile / .bash_profile) AND the "interactive" dotfile group (.zshrc /
  // .bashrc) are sourced — same as opening a fresh Terminal.app tab. fish
  // can't combine -i with -c, so we drop interactive for it.
  const args = shellSupportsInteractive(shell) ? ["-ilc"] : ["-lc"];
  args.push(`echo '${startMarker}' && command env && echo '${endMarker}'`);

  return await new Promise<NodeJS.ProcessEnv>((resolve, reject) => {
    const child = spawn(shell, args, {
      cwd,
      env: {
        ...process.env,
        // Suppress oh-my-zsh chatter that would otherwise print during rc
        // sourcing — auto-updates, tmux auto-start prompts, directory
        // auto-titling. These vars are respected by OMZ and harmless to
        // setups that ignore them.
        DISABLE_AUTO_UPDATE: "true",
        DISABLE_AUTO_TITLE: "true",
        DISABLE_UPDATE_PROMPT: "true",
        ZSH_TMUX_AUTOSTARTED: "true",
        ZSH_TMUX_AUTOSTART: "false",
      },
      // Don't inherit stdin — a dotfile that reads from stdin would hang us.
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";

    const timeout = setTimeout(() => {
      try {
        child.kill("SIGKILL");
      } catch {}
      reject(
        new Error(
          `Resolving shell environment timed out after ${timeoutMs}ms. Review your shell startup files (~/.zshrc, ~/.bashrc, ~/.zprofile) for slow operations, or increase sweetpad.shellEnv.timeout.`,
        ),
      );
    }, timeoutMs);

    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", (err) => {
      clearTimeout(timeout);
      reject(err);
    });
    child.on("close", (code) => {
      clearTimeout(timeout);
      if (code !== 0) {
        reject(new Error(`Shell environment probe exited with code ${code}. stderr: ${stderr.slice(0, 500)}`));
        return;
      }
      const parsed = extractEnvBetweenMarkers(stdout, startMarker, endMarker);
      if (!parsed) {
        reject(
          new Error(
            `Could not find env markers in shell output. Your .zshrc / .bashrc may have exited early or never reached the env dump. Stdout head: ${stdout.slice(0, 200)}`,
          ),
        );
        return;
      }
      resolve(cleanResolvedEnv(parsed));
    });
  });
}

/**
 * Anything printed before `startMarker` (dotfile banners, MOTD, fortune,
 * zsh-welcome messages, oh-my-zsh update prompts) and anything after
 * `endMarker` is discarded. Only the block between the markers is parsed
 * as env output — that block comes exclusively from our own `command env`.
 */
function extractEnvBetweenMarkers(stdout: string, startMarker: string, endMarker: string): NodeJS.ProcessEnv | null {
  const startIdx = stdout.indexOf(startMarker);
  if (startIdx === -1) return null;
  const afterStart = stdout.slice(startIdx + startMarker.length);
  const endIdx = afterStart.indexOf(endMarker);
  if (endIdx === -1) return null;
  return parseKeyValueLines(afterStart.slice(0, endIdx));
}

/**
 * Parses "KEY=VALUE" lines as emitted by POSIX `env`. Keys must be valid
 * identifiers; anything else is ignored (defensive against continuation
 * lines of multiline values — rare on macOS iOS toolchains).
 */
function parseKeyValueLines(block: string): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {};
  for (const rawLine of block.split("\n")) {
    const eq = rawLine.indexOf("=");
    if (eq <= 0) continue;
    const key = rawLine.slice(0, eq);
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) continue;
    env[key] = rawLine.slice(eq + 1);
  }
  return env;
}

function cleanResolvedEnv(env: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  const cleaned: NodeJS.ProcessEnv = {};
  for (const [key, value] of Object.entries(env)) {
    if (ELECTRON_ONLY_VARS.has(key)) continue;
    cleaned[key] = value;
  }
  return cleaned;
}

function shellSupportsInteractive(shell: string): boolean {
  // fish doesn't like `-i` combined with `-c` the way POSIX shells do.
  return !/\/fish(\.exe)?$/.test(shell);
}

function maybeNotifyShellEnvFailure(error: unknown, options: ShellEnvOptions): void {
  if (notifiedFailureThisCycle) return;
  notifiedFailureThisCycle = true;
  if (!options.onWarning) return;
  const message = error instanceof Error ? error.message : String(error);
  options.onWarning(`could not resolve shell environment (${message}). Tasks will run with the host's PATH.`);
}
