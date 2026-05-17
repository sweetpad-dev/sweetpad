import * as path from "node:path";

import { ProtocolError } from "../protocol/errors";
import { errorResponse } from "../protocol/envelope";
import { parseArgv } from "./argv";
import { runAttachCommand } from "./commands/attach";
import { runBuildCommand } from "./commands/build";
import { runBuildsCommand } from "./commands/builds";
import { runDestinationsCommand } from "./commands/destinations";
import { runErrorsCommand } from "./commands/errors";
import { runLogsCommand } from "./commands/logs";
import { runSchemesCommand } from "./commands/schemes";
import { runShowCommand } from "./commands/show";
import { runUsageCommand } from "./commands/usage";
import { exitCodeForErrorCode } from "./exit-codes";

const USAGE = `\
sweetpad — agent-facing CLI for SweetPad

Usage:
  sweetpad build --scheme=<name> --destination=<id-or-name> --config=<name>
                 [--workspace=<root-dir>] [--xcworkspace=<file>] [--debug]

  sweetpad builds       [--status=<status>] [--limit=<n>] [--workspace=...]
  sweetpad show <buildId>                                  [--workspace=...]
  sweetpad errors       [--build=<buildId>]                [--workspace=...]
  sweetpad logs <buildId> [--tail=<n>]                     [--workspace=...]
  sweetpad attach <buildId> [--no-replay]                  [--workspace=...]

  sweetpad schemes      [--workspace=<root-dir>] [--xcworkspace=<file>]
  sweetpad destinations [--workspace=<root-dir>] [--kind=<kind>] [--refresh]
  sweetpad usage        [--workspace=<root-dir>]

Common flags:
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

    if (subcommand === "builds") {
      const result = await runBuildsCommand(rest, { cliEntryDir });
      writeEnvelope(result.envelope);
      process.exit(result.exitCode);
    }

    if (subcommand === "show") {
      const result = await runShowCommand(rest, { cliEntryDir });
      writeEnvelope(result.envelope);
      process.exit(result.exitCode);
    }

    if (subcommand === "errors") {
      const result = await runErrorsCommand(rest, { cliEntryDir });
      writeEnvelope(result.envelope);
      process.exit(result.exitCode);
    }

    if (subcommand === "logs") {
      const result = await runLogsCommand(rest, { cliEntryDir });
      writeEnvelope(result.envelope);
      process.exit(result.exitCode);
    }

    if (subcommand === "attach") {
      const result = await runAttachCommand(rest, { cliEntryDir });
      if (result.envelope !== null) writeEnvelope(result.envelope);
      process.exit(result.exitCode);
    }

    if (subcommand === "schemes") {
      const result = await runSchemesCommand(rest, { cliEntryDir });
      writeEnvelope(result.envelope);
      process.exit(result.exitCode);
    }

    if (subcommand === "destinations") {
      const result = await runDestinationsCommand(rest, { cliEntryDir });
      writeEnvelope(result.envelope);
      process.exit(result.exitCode);
    }

    if (subcommand === "usage") {
      const result = await runUsageCommand(rest, { cliEntryDir });
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
