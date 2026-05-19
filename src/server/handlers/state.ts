import type { ConfigurationEntity, SchemeEntity, StateSnapshot } from "../types";
import type { HandlerFn } from "./context";
import { destinationGet } from "./destination";

export const stateGet: HandlerFn<unknown, StateSnapshot> = async (_params, ctx) => {
  const schemeName = ctx.buildManager.getDefaultSchemeForBuild();
  const configurationName = ctx.buildManager.getDefaultConfigurationForBuild();
  const { destination } = await destinationGet({}, ctx);
  const builds = ctx.buildRegistry.listBuilds(5);
  const scheme: SchemeEntity | null = schemeName ? { name: schemeName, isSelected: true } : null;
  const configuration: ConfigurationEntity | null = configurationName
    ? { name: configurationName, isSelected: true }
    : null;
  return {
    workspacePath: ctx.workspacePath,
    scheme,
    destination,
    configuration,
    running: builds.find((b) => b.status === "running") ?? null,
    latest: builds[0] ?? null,
  };
};
