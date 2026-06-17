// Emits the scaffolded Swift bootstrap (PREVIEW_HOST_BOOTSTRAP) to a file so CI
// can compile it against the real Swift toolchain. The committed copy lives at
// examples/preview-bridge/Sources/GeneratedHost/SweetPadPreviewHost.swift and a
// unit test guards it against drift from the TypeScript source of truth.
//
// Usage: node scripts/emit-preview-host.mjs [outputPath]

import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

import ts from "typescript";

const SOURCE = "src/previews/host-bootstrap.ts";
const DEFAULT_OUT = "examples/preview-bridge/Sources/GeneratedHost/SweetPadPreviewHost.swift";

const transpiled = ts.transpileModule(readFileSync(SOURCE, "utf8"), {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
}).outputText;

const tmpFile = join(mkdtempSync(join(tmpdir(), "sweetpad-emit-")), "host-bootstrap.mjs");
writeFileSync(tmpFile, transpiled);
const { PREVIEW_HOST_BOOTSTRAP } = await import(pathToFileURL(tmpFile).href);

const out = process.argv[2] ?? DEFAULT_OUT;
writeFileSync(out, PREVIEW_HOST_BOOTSTRAP);
console.log(`Wrote ${PREVIEW_HOST_BOOTSTRAP.length} bytes to ${dirname(out)}/${out.split("/").pop()}`);
