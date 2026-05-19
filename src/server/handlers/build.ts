import * as vscode from "vscode";

import { getWorkspaceConfig } from "../../common/config";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES, type BuildCommand, type BuildEntity, type DiagnosticEntity } from "../types";
import type { HandlerFn, RpcContext } from "./context";

const VS_CODE_COMMAND: Record<BuildCommand, string> = {
  build: "sweetpad.build.build",
  run: "sweetpad.build.run",
  launch: "sweetpad.build.launch",
  test: "sweetpad.build.test",
  clean: "sweetpad.build.clean",
};

const DEBUG_VS_CODE_COMMAND: Partial<Record<BuildCommand, string>> = {
  build: "sweetpad.debugger.debuggingBuild",
  run: "sweetpad.debugger.debuggingRun",
  launch: "sweetpad.debugger.debuggingLaunch",
};

function resolveBuildId(ctx: RpcContext, buildId: string | undefined): BuildEntity {
  let entity: BuildEntity | undefined;
  if (buildId) {
    entity = ctx.buildRegistry.getBuild(buildId);
    if (!entity) {
      throw new SweetpadRpcError(ERROR_CODES.BUILD_NOT_FOUND, `No build with id ${buildId}`, {
        hint: "sweetpad build list",
      });
    }
  } else {
    entity = ctx.buildRegistry.getLatest();
    if (!entity) {
      throw new SweetpadRpcError(ERROR_CODES.NO_LAST_BUILD, "No builds have been recorded yet for this workspace.", {
        hint: "sweetpad build start build",
      });
    }
  }
  return entity;
}

export const buildStart: HandlerFn<
  { command?: string; debug?: boolean; caller?: string },
  { buildId: string }
> = async (params, ctx) => {
  if (!params?.command || typeof params.command !== "string") {
    throw new SweetpadRpcError(
      ERROR_CODES.INVALID_PARAMS,
      "build.start requires { command: build|run|launch|test|clean }",
    );
  }
  const command = params.command as BuildCommand;
  if (!VS_CODE_COMMAND[command]) {
    throw new SweetpadRpcError(ERROR_CODES.INVALID_PARAMS, `Unknown build command: ${params.command}`, {
      data: { available: Object.keys(VS_CODE_COMMAND) },
    });
  }

  // Fail fast before VS Code pops a QuickPick the agent can't answer.
  // `clean` is the only command that doesn't need a destination.
  const needsDestination = command !== "clean";
  const missing: string[] = [];
  if (!ctx.buildManager.getDefaultSchemeForBuild()) missing.push("scheme");
  if (needsDestination && !ctx.destinationsManager.getSelectedXcodeDestinationForBuild()) missing.push("destination");
  const cfgFromSetting = getWorkspaceConfig("build.configuration");
  if (!cfgFromSetting && !ctx.buildManager.getDefaultConfigurationForBuild()) missing.push("configuration");
  if (missing.length > 0) {
    throw new SweetpadRpcError(
      ERROR_CODES.MISSING_PREREQUISITES,
      `Cannot start ${command}: missing ${missing.join(", ")}.`,
      {
        hint: missing[0] === "scheme" ? "sweetpad scheme set <name>" : `sweetpad ${missing[0]} list`,
        data: { missing },
      },
    );
  }

  const debugFlag = params.debug === true;
  let target = VS_CODE_COMMAND[command];
  if (debugFlag) {
    const debugTarget = DEBUG_VS_CODE_COMMAND[command];
    if (!debugTarget) {
      throw new SweetpadRpcError(ERROR_CODES.INVALID_PARAMS, `--debug is not supported for command: ${command}`);
    }
    target = debugTarget;
  }

  const caller = typeof params.caller === "string" && params.caller.length > 0 ? params.caller : null;
  const buildId = ctx.buildRegistry.reserveCliBuildId({ caller });
  // Fire and forget — errors surface through the extension's normal error UI.
  void vscode.commands.executeCommand(target);
  return { buildId };
};

export const buildStop: HandlerFn<unknown, { stopped: boolean; buildId: string | null }> = async (_params, ctx) => {
  const scheme = ctx.buildManager.getRunningScheme();
  if (!scheme) {
    return { stopped: false, buildId: null };
  }
  const running = ctx.buildRegistry.listBuilds(1)[0];
  await ctx.buildManager.stopScheme(scheme);
  return { stopped: true, buildId: running?.status === "running" ? running.buildId : null };
};

export const buildWait: HandlerFn<{ buildId?: string; timeoutMs?: number }, BuildEntity> = async (params, ctx) => {
  const targetId = params?.buildId ?? ctx.buildRegistry.getLatest()?.buildId;
  if (!targetId) {
    throw new SweetpadRpcError(ERROR_CODES.NO_LAST_BUILD, "No build to wait on.");
  }
  if (!ctx.buildRegistry.getBuild(targetId)) {
    throw new SweetpadRpcError(ERROR_CODES.BUILD_NOT_FOUND, `No build with id ${targetId}`, {
      hint: "sweetpad build list",
    });
  }
  const timeoutMs = typeof params?.timeoutMs === "number" ? params.timeoutMs : undefined;
  return await ctx.buildRegistry.waitForBuild(targetId, timeoutMs);
};

export const buildStatus: HandlerFn<{ buildId?: string }, BuildEntity> = (params, ctx) => {
  return resolveBuildId(ctx, params?.buildId);
};

export const buildList: HandlerFn<{ limit?: number }, { builds: BuildEntity[] }> = (params, ctx) => {
  const limit = typeof params?.limit === "number" && params.limit > 0 ? params.limit : undefined;
  return { builds: ctx.buildRegistry.listBuilds(limit) };
};

export const buildLogs: HandlerFn<{ buildId?: string }, { buildId: string; log: string }> = async (params, ctx) => {
  const entity = resolveBuildId(ctx, params?.buildId);
  const log = await ctx.buildRegistry.readLog(entity.buildId);
  return { buildId: entity.buildId, log };
};

export const buildDiagnostics: HandlerFn<
  { buildId?: string },
  { buildId: string; diagnostics: DiagnosticEntity[] }
> = async (params, ctx) => {
  const entity = resolveBuildId(ctx, params?.buildId);
  const diagnostics = await ctx.buildRegistry.readDiagnostics(entity.buildId);
  return { buildId: entity.buildId, diagnostics };
};
