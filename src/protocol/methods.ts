import type { BuildRequestParams, BuildResponseData } from "./types";

/**
 * Single source of truth for every method the server exposes. Each entry maps
 * the method name → its params shape and result shape. The dispatcher uses
 * this to type-check handler registrations; the client uses it to type-check
 * request calls. Adding a method is one line.
 *
 * The shapes themselves live in `./types` (alongside the wire envelopes) so a
 * future MCP/JSON-RPC adapter can reuse them without depending on this map.
 */
export type MethodMap = {
  build: { params: BuildRequestParams; result: BuildResponseData };
};

export type MethodName = keyof MethodMap;
export type ParamsFor<M extends MethodName> = MethodMap[M]["params"];
export type ResultFor<M extends MethodName> = MethodMap[M]["result"];
