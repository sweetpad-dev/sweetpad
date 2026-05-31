import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { execa } from "execa";

const VERSION = "v0.2.0";
const REPO = "sweetpad-dev/sweetpad-lib";
const BINARY = "sweetpad-darwin-universal";

// Lives inside the build output so the runtime lookup can resolve it via
// `__dirname` from `out/extension.js` without traversing parent dirs.
const BIN_DIR = join(process.cwd(), "out", "bin");
const BIN_PATH = join(BIN_DIR, BINARY);
const VERSION_MARKER = join(BIN_DIR, ".version");

function isUpToDate(): boolean {
  if (!existsSync(BIN_PATH) || !existsSync(VERSION_MARKER)) {
    return false;
  }
  return readFileSync(VERSION_MARKER, "utf8").trim() === VERSION;
}

async function main(): Promise<void> {
  if (isUpToDate()) {
    console.log(`sweetpad ${VERSION} already present at ${BIN_PATH}`);
    return;
  }

  mkdirSync(BIN_DIR, { recursive: true });

  const url = `https://github.com/${REPO}/releases/download/${VERSION}/${BINARY}`;
  console.log(`Downloading ${url}`);
  await execa("curl", ["-fL", "--retry", "3", "-o", BIN_PATH, url], { stdio: "inherit" });

  chmodSync(BIN_PATH, 0o755);
  writeFileSync(VERSION_MARKER, VERSION);
  console.log(`Installed sweetpad ${VERSION} at ${BIN_PATH}`);
}

void main();
