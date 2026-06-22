import { ResponseError } from "vscode-jsonrpc/node";

import { SweetpadRpcError, toResponseError } from "./rpc";
import { ERROR_CODES, JSON_RPC_INTERNAL_ERROR, JSON_RPC_INVALID_PARAMS, SWEETPAD_APPLICATION_ERROR } from "./types";

describe("server/rpc", () => {
  describe("toResponseError", () => {
    it("maps SweetpadRpcError to the application code and surfaces the string code, hint and extra data", () => {
      const err = toResponseError(
        new SweetpadRpcError(ERROR_CODES.SCHEME_NOT_FOUND, "no such scheme", {
          hint: "sweetpad scheme list",
          data: { tried: "X" },
        }),
      );
      expect(err).toBeInstanceOf(ResponseError);
      expect(err.code).toBe(SWEETPAD_APPLICATION_ERROR);
      expect(err.message).toBe("no such scheme");
      expect(err.data?.code).toBe("SCHEME_NOT_FOUND");
      expect(err.data?.hint).toBe("sweetpad scheme list");
      expect(err.data?.tried).toBe("X");
    });

    it("maps the INVALID_PARAMS string code to JSON-RPC -32602", () => {
      const err = toResponseError(new SweetpadRpcError(ERROR_CODES.INVALID_PARAMS, "bad args"));
      expect(err.code).toBe(JSON_RPC_INVALID_PARAMS);
      expect(err.data?.code).toBe("INVALID_PARAMS");
    });

    it("maps an unknown Error to INTERNAL_ERROR, preserving the message", () => {
      const err = toResponseError(new Error("kaboom"));
      expect(err.code).toBe(JSON_RPC_INTERNAL_ERROR);
      expect(err.message).toBe("kaboom");
    });

    it("stringifies a non-Error throw", () => {
      const err = toResponseError("oops");
      expect(err.code).toBe(JSON_RPC_INTERNAL_ERROR);
      expect(err.message).toBe("oops");
    });
  });
});
