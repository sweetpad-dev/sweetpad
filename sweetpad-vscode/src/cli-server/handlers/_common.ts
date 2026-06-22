import { isFileExists } from "../../common/files";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES, type ErrorCode } from "../types";

export function requireString(
  value: unknown,
  method: string,
  field: string,
  code: ErrorCode = ERROR_CODES.INVALID_PARAMS,
): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new SweetpadRpcError(code, `${method} requires { ${field}: string }`);
  }
  return value;
}

export const fileExists = isFileExists;
