import { commonLogger } from "../../common/logger";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES } from "../types";
import type { HandlerFn } from "./context";

const DEFAULT_LINES = 50;
const MAX_LINES = 1000;

type LogLevel = "debug" | "info" | "warning" | "error";
const LEVEL_RANK: Record<LogLevel, number> = { debug: 0, info: 1, warning: 2, error: 3 };

/**
 * Last N entries from the SweetPad: Common output channel, optionally filtered
 * to entries at or above a given level. The logger's in-memory buffer caps at
 * 1000 entries so older content has to be read out of VS Code's panel UI.
 */
export const logsTail: HandlerFn<
  { lines?: number; level?: string },
  { count: number; entries: { time: string; level: LogLevel; message: string }[] }
> = (params) => {
  const requested = typeof params?.lines === "number" && params.lines > 0 ? params.lines : DEFAULT_LINES;
  const lines = Math.min(requested, MAX_LINES);
  const minLevel = parseLevel(params?.level);
  const messages = commonLogger.last(lines * 4); // overshoot so filtering still leaves N
  const entries = messages
    .map((m) => ({
      time: typeof m.time === "string" ? m.time : new Date().toISOString(),
      level: numericToLevel(typeof m.level === "number" ? m.level : 1),
      message: m.message,
    }))
    .filter((e) => LEVEL_RANK[e.level] >= LEVEL_RANK[minLevel])
    .slice(-lines);
  return { count: entries.length, entries };
};

function parseLevel(value: unknown): LogLevel {
  if (value === undefined || value === null) return "debug";
  if (value === "debug" || value === "info" || value === "warning" || value === "error") return value;
  throw new SweetpadRpcError(
    ERROR_CODES.INVALID_PARAMS,
    `Unknown level: ${String(value)}. Expected debug|info|warning|error.`,
  );
}

function numericToLevel(level: number): LogLevel {
  if (level <= 0) return "debug";
  if (level === 1) return "info";
  if (level === 2) return "warning";
  return "error";
}
