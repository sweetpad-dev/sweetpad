import { ERROR_CODES } from "../types";
import { requireString } from "./_common";
import type { HandlerFn } from "./context";

const requireKey = (key: unknown, method: string) =>
  requireString(key, method, "key", ERROR_CODES.WORKSPACE_STATE_KEY_INVALID);

export const workspaceStateGet: HandlerFn<{ key?: string }, { key: string; value: unknown }> = (params, ctx) => {
  const key = requireKey(params?.key, "workspaceState.get");
  const value = ctx.workspace.rawGet(key);
  return { key, value: value === undefined ? null : value };
};

export const workspaceStateSet: HandlerFn<{ key?: string; value?: unknown }, { key: string; value: unknown }> = async (
  params,
  ctx,
) => {
  const key = requireKey(params?.key, "workspaceState.set");
  const value = params?.value === null ? undefined : params?.value;
  await ctx.workspace.rawUpdate(key, value);
  return { key, value: ctx.workspace.rawGet(key) ?? null };
};

export const workspaceStateKeys: HandlerFn<unknown, { keys: string[] }> = (_params, ctx) => {
  return { keys: ctx.workspace.rawKeys().toSorted() };
};

export const workspaceStateDelete: HandlerFn<{ key?: string }, { key: string; deleted: boolean }> = async (
  params,
  ctx,
) => {
  const key = requireKey(params?.key, "workspaceState.delete");
  const existed = ctx.workspace.rawGet(key) !== undefined;
  await ctx.workspace.rawUpdate(key, undefined);
  return { key, deleted: existed };
};
