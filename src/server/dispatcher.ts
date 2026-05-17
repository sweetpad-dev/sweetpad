import type { Logger } from "../core/logger/types";
import { errorResponse, successResponse } from "../protocol/envelope";
import { ProtocolError } from "../protocol/errors";
import type { MethodName, ParamsFor, ResultFor } from "../protocol/methods";
import type { WireRequest, WireResponse } from "../protocol/types";

export type MethodHandler<M extends MethodName> = (params: ParamsFor<M>) => Promise<ResultFor<M>>;

// Runtime-only erased view of `MethodHandler<M>` used by the dispatcher's
// internal map. The `register<M>` entry point keeps the static signature
// honest; this type just lets us store handlers of different M in one map.
type ErasedHandler = (params: unknown) => Promise<unknown>;

/**
 * Maps method names to handlers and wraps each invocation in the response
 * envelope. Handlers raise `ProtocolError` to short-circuit with a structured
 * error; anything else is treated as INTERNAL and the underlying error is
 * logged but not exposed (avoids leaking stack traces over the socket).
 *
 * Method names + param/result types come from `protocol/methods.ts`. Adding
 * a method requires (a) the MethodMap entry and (b) a `register("name", fn)`
 * call wired in server `index.ts`.
 */
export class MethodDispatcher {
  private readonly handlers = new Map<MethodName, ErasedHandler>();

  constructor(private readonly logger: Logger) {}

  register<M extends MethodName>(method: M, handler: MethodHandler<M>): void {
    this.handlers.set(method, handler as ErasedHandler);
  }

  async handle(request: WireRequest): Promise<WireResponse> {
    const handler = this.handlers.get(request.method as MethodName);
    if (!handler) {
      return errorResponse(request.id, {
        code: "INVALID_ARGUMENT",
        message: `Unknown method '${request.method}'`,
      });
    }

    try {
      const data = await handler(request.params ?? {});
      return successResponse(request.id, data);
    } catch (error) {
      if (error instanceof ProtocolError) {
        return errorResponse(request.id, error.toPayload(), error.extra);
      }
      const message = error instanceof Error ? error.message : String(error);
      this.logger.error("Unhandled error in method handler", {
        method: request.method,
        error,
      });
      return errorResponse(request.id, { code: "INTERNAL", message });
    }
  }
}
