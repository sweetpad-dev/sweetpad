import { METHOD_CATALOG, type MethodSchema } from "../method-catalog";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES, PROTOCOL_VERSION } from "../types";
import type { HandlerFn } from "./context";

export const metaUsage: HandlerFn<unknown, { methods: { method: string; description: string }[] }> = () => {
  const methods = Object.entries(METHOD_CATALOG).map(([method, schema]) => ({
    method,
    description: schema.description,
  }));
  return { methods };
};

export const metaSchema: HandlerFn<{ method?: string }, MethodSchema | Record<string, MethodSchema>> = (params) => {
  if (!params?.method) {
    return METHOD_CATALOG;
  }
  const schema = METHOD_CATALOG[params.method];
  if (!schema) {
    throw new SweetpadRpcError(ERROR_CODES.INVALID_PARAMS, `Unknown method: ${params.method}`, {
      hint: "sweetpad meta usage",
    });
  }
  return schema;
};

export const metaVersion: HandlerFn<unknown, { extensionVersion: string; protocolVersion: string }> = (
  _params,
  ctx,
) => {
  return {
    extensionVersion: ctx.extensionVersion,
    protocolVersion: PROTOCOL_VERSION,
  };
};

export const metaWorkspacePath: HandlerFn<unknown, { workspacePath: string }> = (_params, ctx) => {
  return { workspacePath: ctx.workspacePath };
};
