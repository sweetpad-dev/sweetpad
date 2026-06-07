import { promises as fs } from "node:fs";

import { findProjectRoot, getCliConfigFile } from "../cli-server/paths";
import type { CliServerMetadata } from "../cli-server/types";
import type { ParsedArgv } from "./argv";
import { rpc, RpcError } from "./client";
import { parseDuration } from "./duration";

export type CliExit = { code: number; stdout?: unknown; stderr?: unknown };

// The CLI talks to the single control server advertised in `.sweetpad/cli.json`
// (last-writer-wins across windows). Read its socket; the connect itself surfaces
// a dead server as ECONNREFUSED.
async function resolveSocket(): Promise<string | CliExit> {
  const noServer = (message: string): CliExit => ({ code: 2, stderr: errorEnvelope("NO_SERVER", message) });
  const root = await findProjectRoot(process.cwd());
  if (!root) {
    return noServer("No .sweetpad project found from the current directory.");
  }
  try {
    const meta = JSON.parse(await fs.readFile(getCliConfigFile(root), "utf8")) as CliServerMetadata;
    if (typeof meta?.socket !== "string") throw new Error("cli.json has no socket");
    return meta.socket;
  } catch {
    return noServer(
      "No running SweetPad server (.sweetpad/cli.json not found). Enable sweetpad.server.enabled and open the project in VS Code.",
    );
  }
}

function errorEnvelope(
  code: string,
  message: string,
  hint?: string,
  data?: Record<string, unknown>,
): { ok: false; error: { code: string; message: string; hint?: string; data?: Record<string, unknown> } } {
  return { ok: false, error: { code, message, ...(hint ? { hint } : {}), ...(data ? { data } : {}) } };
}

async function callRpc(method: string, params: unknown, timeoutMs?: number): Promise<CliExit> {
  const sock = await resolveSocket();
  if (typeof sock !== "string") return sock;
  try {
    const result = await rpc({ socketPath: sock, method, params, timeoutMs });
    return { code: 0, stdout: result };
  } catch (err) {
    if (err instanceof RpcError) {
      return {
        code: 1,
        stderr: errorEnvelope(err.data?.code ?? "RPC_ERROR", err.message, err.data?.hint, err.data),
      };
    }
    const message = err instanceof Error ? err.message : String(err);
    return { code: 2, stderr: errorEnvelope("CLI_ERROR", message) };
  }
}

type Mapped = { method: string; params: unknown; timeoutMs?: number };

function strFlag(v: string | boolean | undefined): string | undefined {
  return typeof v === "string" ? v : undefined;
}
function boolFlag(v: string | boolean | undefined): boolean | undefined {
  if (v === true || v === "true") return true;
  if (v === "false") return false;
  return undefined;
}
function numFlag(v: string | boolean | undefined): number | undefined {
  if (typeof v !== "string") return undefined;
  const n = Number.parseFloat(v);
  return Number.isFinite(n) ? n : undefined;
}

function buildStartParams(parsed: ParsedArgv, command: string): Mapped {
  const debug = parsed.flags.debug === true || parsed.flags.debug === "true";
  const caller = strFlag(parsed.flags.caller) ?? process.env.SWEETPAD_CALLER ?? undefined;
  return {
    method: "build.start",
    params: { command, debug, ...(caller ? { caller } : {}) },
  };
}

function buildWaitParams(parsed: ParsedArgv, positionalId: string | undefined): Mapped {
  const timeoutStr = strFlag(parsed.flags.timeout);
  const timeoutSec = timeoutStr !== undefined ? parseDuration(timeoutStr) : undefined;
  const params: Record<string, unknown> = {};
  if (positionalId) params.buildId = positionalId;
  if (timeoutSec !== undefined) params.timeoutMs = Math.round(timeoutSec * 1000);
  // Pad the client timeout past the server's so a slow round-trip doesn't trip us first.
  const clientTimeoutMs = timeoutSec !== undefined ? Math.round(timeoutSec * 1000) + 10_000 : undefined;
  return { method: "build.wait", params, timeoutMs: clientTimeoutMs };
}

function methodParams(parsed: ParsedArgv): Mapped | undefined {
  const method = parsed.method;
  if (!method) return undefined;
  const first = parsed.positionals[0];

  switch (method) {
    case "meta.usage":
    case "meta.version":
    case "meta.workspacePath":
    case "state.get":
    case "scheme.list":
    case "scheme.get":
    case "destination.get":
    case "buildConfig.list":
    case "buildConfig.get":
    case "build.stop":
      return { method, params: {} };

    case "meta.schema":
      return { method, params: first ? { method: first } : {} };

    case "scheme.set":
    case "buildConfig.set":
      return { method, params: { name: first } };
    case "destination.set":
      return { method, params: { id: first } };

    case "destination.list": {
      const params: Record<string, unknown> = {};
      const type = strFlag(parsed.flags.type);
      const platform = strFlag(parsed.flags.platform);
      const booted = boolFlag(parsed.flags.booted);
      if (type) params.type = type;
      if (platform) params.platform = platform;
      if (booted !== undefined) params.booted = booted;
      return { method, params };
    }

    case "simulator.list": {
      const params: Record<string, unknown> = {};
      const state = strFlag(parsed.flags.state);
      const available = boolFlag(parsed.flags.available);
      if (state) params.state = state;
      if (available !== undefined) params.available = available;
      return { method, params };
    }
    case "simulator.start":
    case "simulator.stop":
      return { method, params: { id: first } };
    case "simulator.refresh":
      return { method, params: {} };

    case "build.start":
      return buildStartParams(parsed, first ?? "build");
    case "build.wait":
      return buildWaitParams(parsed, first);
    case "build.status":
    case "build.logs":
    case "build.diagnostics":
      return { method, params: first ? { buildId: first } : {} };
    case "build.list": {
      const limit = numFlag(parsed.flags.limit);
      return { method, params: limit !== undefined ? { limit } : {} };
    }

    case "scheme.reveal":
      return { method, params: { name: first } };

    case "simulator.install":
      return { method, params: { udid: first, appPath: parsed.positionals[1] } };
    case "simulator.uninstall":
    case "simulator.terminateApp":
      return { method, params: { udid: first, bundleId: parsed.positionals[1] } };
    case "simulator.openUrl":
      return { method, params: { udid: first, url: parsed.positionals[1] } };
    case "simulator.screenshot":
      return {
        method,
        params: { udid: first, ...(strFlag(parsed.flags.path) ? { path: strFlag(parsed.flags.path) } : {}) },
      };
    case "simulator.launchApp": {
      const params: Record<string, unknown> = { udid: first, bundleId: parsed.positionals[1] };
      const args = parseJsonFlag(parsed.flags["args-json"]);
      const env = parseJsonFlag(parsed.flags["env-json"]);
      if (Array.isArray(args)) params.args = args;
      if (env && typeof env === "object") params.env = env;
      if (parsed.flags["wait-for-debugger"] === true) params.waitForDebugger = true;
      return { method, params };
    }

    case "device.install":
      return { method, params: { deviceId: first, appPath: parsed.positionals[1] } };
    case "device.terminate":
      return { method, params: { deviceId: first, bundleId: parsed.positionals[1] } };
    case "device.launch": {
      const params: Record<string, unknown> = { deviceId: first, bundleId: parsed.positionals[1] };
      const args = parseJsonFlag(parsed.flags["args-json"]);
      const env = parseJsonFlag(parsed.flags["env-json"]);
      if (Array.isArray(args)) params.args = args;
      if (env && typeof env === "object") params.env = env;
      if (parsed.flags["no-terminate-existing"] === true) params.terminateExisting = false;
      return { method, params };
    }

    case "buildSettings.get": {
      const params: Record<string, unknown> = {};
      const scheme = strFlag(parsed.flags.scheme);
      const configuration = strFlag(parsed.flags.configuration);
      const sdk = strFlag(parsed.flags.sdk);
      const xcworkspace = strFlag(parsed.flags.xcworkspace);
      const keysCsv = strFlag(parsed.flags.keys);
      if (scheme) params.scheme = scheme;
      if (configuration) params.configuration = configuration;
      if (sdk) params.sdk = sdk;
      if (xcworkspace) params.xcworkspace = xcworkspace;
      if (keysCsv)
        params.keys = keysCsv
          .split(",")
          .map((k) => k.trim())
          .filter((k) => k.length > 0);
      return { method, params };
    }
    case "xcodebuild.list": {
      const xcworkspace = strFlag(parsed.flags.xcworkspace);
      return { method, params: xcworkspace ? { xcworkspace } : {} };
    }
    case "appPath.find":
    case "bundleId.get": {
      const params: Record<string, unknown> = {};
      const scheme = strFlag(parsed.flags.scheme);
      const configuration = strFlag(parsed.flags.configuration);
      const sdk = strFlag(parsed.flags.sdk);
      const xcworkspace = strFlag(parsed.flags.xcworkspace);
      if (scheme) params.scheme = scheme;
      if (configuration) params.configuration = configuration;
      if (sdk) params.sdk = sdk;
      if (xcworkspace) params.xcworkspace = xcworkspace;
      return { method, params };
    }
    case "derivedData.path":
      return { method, params: {} };

    case "workspace.detect": {
      const depth = numFlag(parsed.flags.depth);
      return { method, params: depth !== undefined ? { depth } : {} };
    }
    case "workspace.use":
      return { method, params: { path: first } };
    case "workspace.recent":
      return { method, params: {} };

    case "workspaceState.get":
    case "workspaceState.delete":
      return { method, params: { key: first } };
    case "workspaceState.keys":
      return { method, params: {} };
    case "workspaceState.set": {
      const params: Record<string, unknown> = { key: first };
      const valueJson = strFlag(parsed.flags.value);
      if (valueJson !== undefined) params.value = parseValueFlag(valueJson);
      return { method, params };
    }

    case "vscode.executeCommand": {
      const params: Record<string, unknown> = { command: first };
      const args = parseJsonFlag(parsed.flags["args-json"]);
      if (Array.isArray(args)) params.args = args;
      else if (parsed.positionals.length > 1) params.args = parsed.positionals.slice(1);
      return { method, params };
    }
    case "vscodeSettings.get":
    case "vscodeSettings.inspect":
      return { method, params: { key: first } };
    case "vscodeSettings.list":
      return { method, params: {} };
    case "vscodeSettings.set": {
      const params: Record<string, unknown> = { key: first };
      const valueJson = strFlag(parsed.flags.value);
      const target = strFlag(parsed.flags.target);
      if (valueJson !== undefined) params.value = parseValueFlag(valueJson);
      if (target) params.target = target;
      return { method, params };
    }

    case "logs.tail": {
      const params: Record<string, unknown> = {};
      const lines = numFlag(parsed.flags.lines);
      const level = strFlag(parsed.flags.level);
      if (lines !== undefined) params.lines = lines;
      if (level) params.level = level;
      return { method, params };
    }

    default:
      return { method, params: {} };
  }
}

// `--value '{"a":1}'`, `--value true`, or `--value "plain string"` all work.
function parseValueFlag(raw: string): unknown {
  try {
    return JSON.parse(raw);
  } catch {
    return raw;
  }
}

function parseJsonFlag(raw: unknown): unknown {
  if (typeof raw !== "string") return undefined;
  try {
    return JSON.parse(raw);
  } catch {
    return undefined;
  }
}

export async function dispatchCli(parsed: ParsedArgv): Promise<CliExit> {
  if (parsed.help) return helpExit();
  if (!parsed.method) return helpExit();
  const mp = methodParams(parsed);
  if (!mp) return helpExit();

  return await callRpc(mp.method, mp.params, mp.timeoutMs);
}

function helpExit(): CliExit {
  const usage = `sweetpad — JSON-RPC client for the SweetPad VS Code extension

Usage:
  sweetpad <method> [args...] [--raw]

Methods (canonical dot-form; arguments listed after the method name):
  meta.usage
  meta.schema [<method>]
  meta.version
  meta.workspacePath

  state.get

  scheme.list
  scheme.get
  scheme.set <name>
  scheme.reveal <name>

  destination.list [--type <t>] [--platform <p>] [--booted]
  destination.get
  destination.set <id>

  simulator.list [--state Booted] [--available]
  simulator.start <id-or-udid>
  simulator.stop <id-or-udid>
  simulator.refresh
  simulator.install <udid> <appPath>
  simulator.uninstall <udid> <bundleId>
  simulator.launchApp <udid> <bundleId> [--args-json '[...]'] [--env-json '{...}'] [--wait-for-debugger]
  simulator.terminateApp <udid> <bundleId>
  simulator.openUrl <udid> <url>
  simulator.screenshot <udid> [--path <p>]

  device.install <deviceId> <appPath>
  device.launch <deviceId> <bundleId> [--args-json '[...]'] [--env-json '{...}'] [--no-terminate-existing]
  device.terminate <deviceId> <bundleId>

  buildConfig.list
  buildConfig.get
  buildConfig.set <name>

  buildSettings.get [--scheme <s>] [--configuration <c>] [--sdk <s>] [--xcworkspace <p>] [--keys K1,K2]
  xcodebuild.list [--xcworkspace <p>]
  appPath.find [--scheme <s>] [--configuration <c>] [--sdk <s>] [--xcworkspace <p>]
  bundleId.get [--scheme <s>] [--configuration <c>] [--sdk <s>] [--xcworkspace <p>]
  derivedData.path

  build.start <cmd> [--debug] [--caller <label>]
  build.stop
  build.wait [<id>] [--timeout <30s|5m|1h>]
  build.status [<id>]
  build.logs [<id>]
  build.diagnostics [<id>]
  build.list [--limit N]

  workspace.detect [--depth N]
  workspace.use <path>
  workspace.recent

  workspaceState.get <key>
  workspaceState.set <key> --value <json|string>
  workspaceState.keys
  workspaceState.delete <key>

  vscode.executeCommand <command> [...args]                      # or --args-json '[...]'
  vscodeSettings.get <key>
  vscodeSettings.set <key> --value <json|string> [--target global|workspace|workspaceFolder]
  vscodeSettings.inspect <key>
  vscodeSettings.list

  logs.tail [--lines N] [--level debug|info|warning|error]

Flags:
  --raw                    minify JSON output
  --timeout <30s|5m|1h>    duration for build.wait (capped server-side ~30s)
  --caller <label>         label build originator (also SWEETPAD_CALLER env)`;
  return { code: 2, stderr: errorEnvelope("USAGE", usage) };
}
