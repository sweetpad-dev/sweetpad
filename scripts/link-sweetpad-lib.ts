/**
 * Local-dev helper: symlink a sweetpad-lib cargo build into the location
 * the extension expects, so the next Extension Development Host launch
 * picks it up without re-running `npm run fetch-sweetpad`.
 *
 *   npm run link-sweetpad-lib              # uses target/debug/sweetpad
 *   npm run link-sweetpad-lib -- --release # uses target/release/sweetpad
 *
 * Assumes you've already built the crate (cargo build [--release]) — the
 * script just locates the binary and creates the symlink at
 * `out/bin/sweetpad-darwin-universal`. Pass `--lib-dir <path>` if your
 * sweetpad-lib checkout lives somewhere other than `../sweetpad-lib`.
 */

import { existsSync, lstatSync, mkdirSync, symlinkSync, unlinkSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const BINARY = "sweetpad-darwin-universal";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "..");

interface Options {
  libDir: string;
  profile: "release" | "debug";
}

function parseArgs(argv: string[]): Options {
  const opts: Options = {
    libDir: resolve(repoRoot, "..", "sweetpad-lib"),
    profile: "debug",
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--debug") {
      opts.profile = "debug";
    } else if (arg === "--release") {
      opts.profile = "release";
    } else if (arg === "--lib-dir") {
      const next = argv[i + 1];
      if (!next) {
        throw new Error("--lib-dir requires a path argument");
      }
      opts.libDir = resolve(next);
      i++;
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }
  return opts;
}

function main(): void {
  const opts = parseArgs(process.argv.slice(2));

  const binarySource = join(opts.libDir, "target", opts.profile, "sweetpad");
  if (!existsSync(binarySource)) {
    throw new Error(
      `Binary not found at ${binarySource}.\n` +
        `Build it first: (cd ${opts.libDir} && cargo build${opts.profile === "release" ? " --release" : ""}).`,
    );
  }

  const binDir = join(repoRoot, "out", "bin");
  const binTarget = join(binDir, BINARY);
  mkdirSync(binDir, { recursive: true });
  if (existsSync(binTarget) || isSymlink(binTarget)) {
    unlinkSync(binTarget);
  }
  symlinkSync(binarySource, binTarget);

  console.log(`Symlinked ${binTarget} → ${binarySource}`);
}

function isSymlink(p: string): boolean {
  try {
    return lstatSync(p).isSymbolicLink();
  } catch {
    return false;
  }
}

main();
