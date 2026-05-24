import * as path from "node:path";

const BINARY_NAME = "sweetpad-darwin-universal";

/**
 * Absolute path to the bundled sweetpad-lib binary.
 *
 * After rolldown emits `out/extension.js`, `__dirname` at runtime is the
 * `out/` directory inside `<extensionPath>`. The binary is dropped into
 * `out/bin/` by `scripts/fetch-sweetpad.ts` (production) or by
 * `npm run link-sweetpad-lib` (local dev — symlinks a pre-built sibling
 * `sweetpad-lib` crate's cargo output).
 */
export function getSweetpadBinPath(): string {
  return path.join(__dirname, "bin", BINARY_NAME);
}
