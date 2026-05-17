import { SCHEMA_VERSION, type UsageResponseData } from "../../protocol/types";
import type { MethodDispatcher } from "../dispatcher";

export type UsageMethodDeps = {
  dispatcher: MethodDispatcher;
};

/**
 * Self-describing endpoint. Lets an agent enumerate every method the server
 * exposes without having to know the schema up front. The dispatcher is the
 * source of truth — anything registered shows up here automatically.
 */
export function createUsageMethod(deps: UsageMethodDeps) {
  return async (): Promise<UsageResponseData> => {
    return {
      schemaVersion: SCHEMA_VERSION,
      methods: deps.dispatcher.listMethods(),
    };
  };
}
