import { dispatch, parseRequest, SweetpadRpcError } from "./rpc";
import {
  ERROR_CODES,
  JSON_RPC_INTERNAL_ERROR,
  JSON_RPC_INVALID_PARAMS,
  JSON_RPC_INVALID_REQUEST,
  JSON_RPC_METHOD_NOT_FOUND,
  JSON_RPC_PARSE_ERROR,
  type JsonRpcFailure,
  type JsonRpcRequest,
  type JsonRpcResponse,
  type JsonRpcSuccess,
  SWEETPAD_APPLICATION_ERROR,
} from "./types";

function asRequest(r: JsonRpcRequest | JsonRpcFailure): JsonRpcRequest {
  if ("error" in r) throw new Error(`Expected JsonRpcRequest but got error: ${r.error.message}`);
  return r;
}

function asFailure(r: JsonRpcRequest | JsonRpcFailure): JsonRpcFailure {
  if (!("error" in r)) throw new Error("Expected JsonRpcFailure but got JsonRpcRequest");
  return r;
}

function asSuccess<T>(res: JsonRpcResponse<T>): JsonRpcSuccess<T> {
  if ("error" in res) throw new Error(`Expected success but got error: ${res.error.message}`);
  return res;
}

function asResponseFailure(res: JsonRpcResponse): JsonRpcFailure {
  if (!("error" in res)) throw new Error("Expected error response but got success");
  return res;
}

describe("server/rpc", () => {
  describe("parseRequest", () => {
    it("parses a valid request", () => {
      const r = asRequest(parseRequest('{"jsonrpc":"2.0","id":1,"method":"foo.bar","params":{"x":1}}'));
      expect(r.method).toBe("foo.bar");
      expect(r.id).toBe(1);
    });

    it("returns parse-error for invalid JSON", () => {
      const r = asFailure(parseRequest("not json"));
      expect(r.error.code).toBe(JSON_RPC_PARSE_ERROR);
    });

    it("returns invalid-request when missing method", () => {
      const r = asFailure(parseRequest('{"jsonrpc":"2.0","id":1}'));
      expect(r.error.code).toBe(JSON_RPC_INVALID_REQUEST);
    });

    it("returns invalid-request when jsonrpc != 2.0", () => {
      const r = asFailure(parseRequest('{"jsonrpc":"1.0","id":1,"method":"foo"}'));
      expect(r.error.code).toBe(JSON_RPC_INVALID_REQUEST);
    });
  });

  describe("dispatch", () => {
    it("invokes the handler and returns its result", async () => {
      const res = asSuccess(
        await dispatch(
          { jsonrpc: "2.0", id: 7, method: "say.hi", params: { name: "claude" } },
          {
            "say.hi": (params) => ({ greeting: `hello ${(params as { name: string }).name}` }),
          },
        ),
      );
      expect(res.result).toEqual({ greeting: "hello claude" });
    });

    it("returns METHOD_NOT_FOUND when handler is missing", async () => {
      const res = asResponseFailure(await dispatch({ jsonrpc: "2.0", id: 1, method: "nope" }, {}));
      expect(res.error.code).toBe(JSON_RPC_METHOD_NOT_FOUND);
    });

    it("maps SweetpadRpcError to application code and surfaces string code in data", async () => {
      const res = asResponseFailure(
        await dispatch(
          { jsonrpc: "2.0", id: 1, method: "boom" },
          {
            boom: () => {
              throw new SweetpadRpcError(ERROR_CODES.SCHEME_NOT_FOUND, "no such scheme", {
                hint: "sweetpad scheme list",
                data: { tried: "X" },
              });
            },
          },
        ),
      );
      expect(res.error.code).toBe(SWEETPAD_APPLICATION_ERROR);
      expect(res.error.data?.code).toBe("SCHEME_NOT_FOUND");
      expect(res.error.data?.hint).toBe("sweetpad scheme list");
      expect(res.error.data?.tried).toBe("X");
    });

    it("maps INVALID_PARAMS code to JSON-RPC -32602", async () => {
      const res = asResponseFailure(
        await dispatch(
          { jsonrpc: "2.0", id: 1, method: "boom" },
          {
            boom: () => {
              throw new SweetpadRpcError(ERROR_CODES.INVALID_PARAMS, "bad args");
            },
          },
        ),
      );
      expect(res.error.code).toBe(JSON_RPC_INVALID_PARAMS);
    });

    it("maps unknown errors to INTERNAL_ERROR", async () => {
      const res = asResponseFailure(
        await dispatch(
          { jsonrpc: "2.0", id: 1, method: "boom" },
          {
            boom: () => {
              throw new Error("kaboom");
            },
          },
        ),
      );
      expect(res.error.code).toBe(JSON_RPC_INTERNAL_ERROR);
      expect(res.error.message).toBe("kaboom");
    });
  });
});
