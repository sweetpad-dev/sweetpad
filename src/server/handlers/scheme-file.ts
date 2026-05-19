import { promises as fs } from "node:fs";

import { findFilesRecursive } from "../../common/files";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES } from "../types";
import { requireString } from "./_common";
import type { HandlerFn, RpcContext } from "./context";

const MAX_SCHEME_XML_BYTES = 1024 * 1024;
const SKIP_DIRS = ["node_modules", ".build", "DerivedData", ".git"];

async function locateSchemeFiles(workspacePath: string, name: string): Promise<string[]> {
  const target = `${name}.xcscheme`;
  const candidates = await findFilesRecursive({
    directory: workspacePath,
    depth: 8,
    matcher: (file) => file.name === target,
    ignore: SKIP_DIRS,
  });
  // Restrict to standard Xcode locations and prefer shared schemes first
  // (matches Xcode's own resolution order).
  const shared = candidates.filter((p) => p.includes("/xcshareddata/xcschemes/"));
  const user = candidates.filter((p) => /\/xcuserdata\/[^/]+\.xcuserdatad\/xcschemes\//.test(p));
  return [...shared, ...user];
}

async function resolveScheme(ctx: RpcContext, name: string): Promise<string> {
  const found = await locateSchemeFiles(ctx.workspacePath, name);
  if (found.length === 0) {
    throw new SweetpadRpcError(ERROR_CODES.SCHEME_FILE_NOT_FOUND, `No .xcscheme file found for "${name}".`, {
      hint: "sweetpad scheme.list",
    });
  }
  return found[0];
}

export const schemeReveal: HandlerFn<
  { name?: string },
  { name: string; path: string; xml: string; allPaths: string[] }
> = async (params, ctx) => {
  const name = requireString(params?.name, "scheme.reveal", "name");
  const all = await locateSchemeFiles(ctx.workspacePath, name);
  if (all.length === 0) {
    throw new SweetpadRpcError(ERROR_CODES.SCHEME_FILE_NOT_FOUND, `No .xcscheme file found for "${name}".`, {
      hint: "sweetpad scheme.list",
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

export const schemeWrite: HandlerFn<
  { name?: string; xml?: string; path?: string },
  { name: string; path: string }
> = async (params, ctx) => {
  const name = requireString(params?.name, "scheme.write", "name");
  const xml = requireString(params?.xml, "scheme.write", "xml");
  const target = params?.path && params.path.length > 0 ? params.path : await resolveScheme(ctx, name);
  try {
    await fs.writeFile(target, xml, "utf8");
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new SweetpadRpcError(ERROR_CODES.SCHEME_FILE_WRITE_FAILED, message);
  }
  return { name, path: target };
};
