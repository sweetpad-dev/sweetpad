import * as path from "node:path";

import { ProtocolError } from "../protocol/errors";
import { errorResponse } from "../protocol/envelope";
import { parseArgv } from "./argv";
import { runBuildCommand } from "./commands/build";
import { exitCodeForErrorCode } from "./exit-codes";

const USAGE = `\
sweetpad — agent-facing CLI for SweetPad

Usage:
  sweetpad build --scheme=<name> --destination=<id-or-name> --config=<name>
                 [--workspace=<root-dir>] [--xcworkspace=<file>] [--debug]

  --workspace      Override the project root (default: walk cwd up to the
                   first .xcworkspace / .xcodeproj / Package.swift /
                   Project.swift).
  --xcworkspace    Override the .xcworkspace / Package.swift xcodebuild
                   targets. Auto-detected within the workspace by default.

Output is a single JSON envelope on stdout. Exit codes:
  0 — success
  1 — build failed / server unreachable / transient
  2 — user error (invalid flag, ambiguous scheme/destination, etc.)
`;

async function main(): Promise<void> {
  const argv = process.argv.slice(2);
  if (argv.length === 0 || argv[0] === "-h" || argv[0] === "--help") {
    process.stdout.write(USAGE);
    process.exit(0);
  }

  const subcommand = argv[0];
  const rest = parseArgv(argv.slice(1));
  const cliEntryDir = path.dirname(__filename);

  try {
    if (subcommand === "build") {
      const result = await runBuildCommand(rest, { cliEntryDir });
      writeEnvelope(result.envelope);
      process.exit(result.exitCode);
    }

    process.stderr.write(`unknown subcommand: ${subcommand}\n${USAGE}`);
    process.exit(2);
  } catch (error) {
    const envelope = errorResponse(0, asProtocolError(error).toPayload(), undefined);
    writeEnvelope(envelope);
    process.exit(exitCodeForErrorCode(envelope.error.code));
  }
}

function writeEnvelope(envelope: object): void {
  process.stdout.write(`${JSON.stringify(envelope)}\n`);
}

function asProtocolError(error: unknown): ProtocolError {
  if (error instanceof ProtocolError) return error;
  if (error instanceof Error && error.message.includes("ENOENT") && error.message.includes(".sock")) {
    return new ProtocolError("SERVER_UNREACHABLE", error.message);
  }
  return new ProtocolError("INTERNAL", error instanceof Error ? error.message : String(error));
}

main().catch((error) => {
  process.stderr.write(`sweetpad CLI crashed: ${error instanceof Error ? error.stack ?? error.message : String(error)}\n`);
  process.exit(1);
});
