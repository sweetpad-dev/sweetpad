import type {
  BuildGetRequestParams,
  BuildGetResponseData,
  BuildRequestParams,
  BuildResponseData,
  BuildsListRequestParams,
  BuildsListResponseData,
  DestinationsListRequestParams,
  DestinationsListResponseData,
  LogsGetRequestParams,
  LogsGetResponseData,
  SchemesListRequestParams,
  SchemesListResponseData,
  UsageRequestParams,
  UsageResponseData,
} from "./types";

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
  "builds.list": { params: BuildsListRequestParams; result: BuildsListResponseData };
  "build.get": { params: BuildGetRequestParams; result: BuildGetResponseData };
  "logs.get": { params: LogsGetRequestParams; result: LogsGetResponseData };
  "schemes.list": { params: SchemesListRequestParams; result: SchemesListResponseData };
  "destinations.list": { params: DestinationsListRequestParams; result: DestinationsListResponseData };
  usage: { params: UsageRequestParams; result: UsageResponseData };
};

export type MethodName = keyof MethodMap;
export type ParamsFor<M extends MethodName> = MethodMap[M]["params"];
export type ResultFor<M extends MethodName> = MethodMap[M]["result"];
