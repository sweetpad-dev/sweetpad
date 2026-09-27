import { promises as fs } from "node:fs";

import * as sweetpadLib from "@sweetpad/native";

import { activateCurrentXcodeWorkspacePath } from "../../build/utils";
import { methodHint } from "../method-catalog";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES } from "../types";
import { requireString } from "./_common";
import type { HandlerFn } from "./context";

const MAX_SCHEME_XML_BYTES = 1024 * 1024;

export const schemeReveal: HandlerFn<
  { name?: string },
  { name: string; path: string; xml: string; allPaths: string[] }
> = async (params, ctx) => {
  const name = requireString(params?.name, "scheme.reveal", "name");
  const xcworkspace = activateCurrentXcodeWorkspacePath({
    workspaceState: ctx.workspaceState,
    workspaceContext: ctx.workspaceContext,
  });
  if (!xcworkspace) {
    throw new SweetpadRpcError(ERROR_CODES.NO_WORKSPACE, "No Xcode workspace detected for this folder.", {
      hint: "open the project in VS Code so SweetPad can detect the workspace",
    });
  }
  // The files `xcodebuild` reads for the current project, the one it uses first: the project's
  // own, its member projects' and its local packages', and only this user's `xcuserdata`.
  const all = sweetpadLib.schemeFiles(xcworkspace, name);
  if (all.length === 0) {
    throw new SweetpadRpcError(ERROR_CODES.SCHEME_FILE_NOT_FOUND, `No .xcscheme file found for "${name}".`, {
      hint: methodHint("scheme.list"),
    });
  }
  const primary = all[0];
  const stat = await fs.stat(primary);
  if (stat.size > MAX_SCHEME_XML_BYTES) {
    throw new SweetpadRpcError(
      ERROR_CODES.SCHEME_FILE_NOT_FOUND,
      `Scheme file is ${stat.size} bytes — over the ${MAX_SCHEME_XML_BYTES}-byte limit.`,
    );
  }
  const xml = await fs.readFile(primary, "utf8");
  return { name, path: primary, xml, allPaths: all };
};
