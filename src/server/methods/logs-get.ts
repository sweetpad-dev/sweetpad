import * as fs from "node:fs";

import { ProtocolError } from "../../protocol/errors";
import type { LogsGetRequestParams, LogsGetResponseData } from "../../protocol/types";
import type { BuildRegistry } from "../registry";

export type LogsGetMethodDeps = {
  registry: BuildRegistry;
};

export function createLogsGetMethod(deps: LogsGetMethodDeps) {
  return async (rawParams: unknown): Promise<LogsGetResponseData> => {
    const params = validateParams(rawParams);

    const build = deps.registry.get(params.buildId);
    if (!build) {
      throw new ProtocolError("BUILD_NOT_FOUND", `No build with id '${params.buildId}'`, {
        hint: "sweetpad builds — list everything in the registry",
      });
    }

    const logPath = deps.registry.getLogPath(params.buildId);
    if (!fs.existsSync(logPath)) {
      // The build exists but no log file ever got written. Could happen for
      // recovered "interrupted" builds where the server died before the
      // first line was flushed. Return an empty payload instead of erroring
      // — agents should still be able to ask without conditionals.
      return { buildId: params.buildId, content: "", lineCount: 0, truncated: false };
    }

    const raw = fs.readFileSync(logPath, "utf8");
    const lines = raw.length === 0 ? [] : raw.split("\n");
    // The LogWriter appends `\n` after every line; split() therefore leaves
    // a trailing empty string. Drop it so lineCount matches reality.
    if (lines.length > 0 && lines[lines.length - 1] === "") {
      lines.pop();
    }

    if (params.tail !== undefined && lines.length > params.tail) {
      const tailed = lines.slice(-params.tail);
      return {
        buildId: params.buildId,
        content: tailed.join("\n"),
        lineCount: lines.length,
        truncated: true,
      };
    }
    return {
      buildId: params.buildId,
      content: lines.join("\n"),
      lineCount: lines.length,
      truncated: false,
    };
  };
}

function validateParams(raw: unknown): LogsGetRequestParams {
  if (!raw || typeof raw !== "object") {
    throw new ProtocolError("INVALID_ARGUMENT", "Missing logs.get params");
  }
  const params = raw as Partial<LogsGetRequestParams>;

  if (typeof params.buildId !== "string" || params.buildId.length === 0) {
    throw new ProtocolError("INVALID_ARGUMENT", "'buildId' is required and must be a non-empty string");
  }
  if (params.tail !== undefined) {
    if (typeof params.tail !== "number" || !Number.isInteger(params.tail) || params.tail < 0) {
      throw new ProtocolError("INVALID_ARGUMENT", "'tail' must be a non-negative integer");
    }
  }

  return { buildId: params.buildId, tail: params.tail };
}
