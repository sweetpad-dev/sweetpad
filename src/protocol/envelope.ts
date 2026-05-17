import {
  SCHEMA_VERSION,
  type WireErrorPayload,
  type WireErrorResponse,
  type WireResponse,
  type WireSuccessResponse,
} from "./types";

export function successResponse<T>(id: number, data: T): WireSuccessResponse<T> {
  return {
    id,
    ok: true,
    schemaVersion: SCHEMA_VERSION,
    data,
  };
}

export function errorResponse(id: number, error: WireErrorPayload, extra?: Record<string, unknown>): WireErrorResponse {
  return {
    id,
    ok: false,
    schemaVersion: SCHEMA_VERSION,
    error,
    ...(extra ?? {}),
  };
}

export function isSuccess<T>(response: WireResponse<T>): response is WireSuccessResponse<T> {
  return response.ok === true;
}
