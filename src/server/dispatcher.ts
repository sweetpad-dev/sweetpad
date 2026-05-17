import type { Logger } from "../core/logger/types";
import { errorResponse, successResponse } from "../protocol/envelope";
import { ProtocolError } from "../protocol/errors";
import type { MethodName, ParamsFor, ResultFor } from "../protocol/methods";
import type { MethodSummary, WireRequest, WireResponse } from "../protocol/types";

export type MethodHandler<M extends MethodName> = (params: ParamsFor<M>) => Promise<ResultFor<M>>;

export type RegisterOptions<M extends MethodName> = {
  description: string;
  handler: MethodHandler<M>;
};

// Runtime-only erased view of `MethodHandler<M>` used by the dispatcher's
// internal map. The `register<M>` entry point keeps the static signature
// honest; this type just lets us store handlers of different M in one map.
type ErasedHandler = (params: unknown) => Promise<unknown>;

type RegistryEntry = {
  handler: ErasedHandler;
  description: string;
};

/**
 * Maps method names to handlers and wraps each invocation in the response
 * envelope. Handlers raise `ProtocolError` to short-circuit with a structured
 * error; anything else is treated as INTERNAL and the underlying error is
 * logged but not exposed (avoids leaking stack traces over the socket).
 *
 * Method names + param/result types come from `protocol/methods.ts`. Adding
 * a method requires (a) the MethodMap entry and (b) a `register("name", { ... })`
 * call wired in server `index.ts`.
 */
export class MethodDispatcher {
  private readonly registry = new Map<MethodName, RegistryEntry>();

  constructor(private readonly logger: Logger) {}

  register<M extends MethodName>(method: M, options: RegisterOptions<M>): void {
    this.registry.set(method, {
      handler: options.handler as ErasedHandler,
      description: options.description,
    });
  }

  listMethods(): MethodSummary[] {
    return Array.from(this.registry.entries()).map(([name, entry]) => ({
      name,
      description: entry.description,
    }));
  }

  async handle(request: WireRequest): Promise<WireResponse> {
    const entry = this.registry.get(request.method as MethodName);
    if (!entry) {
      return errorResponse(request.id, {
        code: "INVALID_ARGUMENT",
        message: `Unknown method '${request.method}'`,
      });
    }

    try {
      const data = await entry.handler(request.params ?? {});
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
