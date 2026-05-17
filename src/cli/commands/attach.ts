import { isSuccess } from "../../protocol/envelope";
import { ProtocolError } from "../../protocol/errors";
import type { AttachRequestParams, BuildEvent } from "../../protocol/types";
import { type ParsedArgs, getBool } from "../argv";
import { exitCodeForErrorCode } from "../exit-codes";
import { type CommandEnv, resolveSocketPath, withClient } from "../runner";

export type AttachCommandResult = {
  exitCode: number;
  envelope: object | null;
};

export type AttachCommandEnv = CommandEnv & {
  /** Override sink for events. Default: print one JSON line per event to stdout. */
  onEvent?: (event: BuildEvent) => void;
};

/**
 * `sweetpad attach <buildId> [--no-replay]` — stream every event a build
 * emits. On a running build, follows live until `build.finished`. On a
 * finished build, replays the recorded `events.jsonl` unless --no-replay
 * was passed. Each event is emitted as its own JSON line on stdout.
 */
export async function runAttachCommand(args: ParsedArgs, env: AttachCommandEnv): Promise<AttachCommandResult> {
  const buildId = args._[0];
  if (!buildId) {
    throw new ProtocolError("INVALID_ARGUMENT", "missing positional argument: <buildId>", {
      hint: "sweetpad attach <buildId> [--no-replay]",
    });
  }

  // `--no-replay` is the natural CLI for "do not replay" — argv keeps it as
  // `no-replay: true`. Translate to the wire param's semantics.
  const replay = !getBool(args, "no-replay");

  const socketPath = await resolveSocketPath(args, env);
  const onEvent = env.onEvent ?? defaultOnEvent;

  return await withClient(socketPath, async (client) => {
    const params: AttachRequestParams = { buildId, replay };
    const errorEnvelope = await client.attach(params, onEvent);

    if (errorEnvelope && !isSuccess(errorEnvelope)) {
      return { exitCode: exitCodeForErrorCode(errorEnvelope.error.code), envelope: errorEnvelope };
    }
    return { exitCode: 0, envelope: null };
  });
}

function defaultOnEvent(event: BuildEvent): void {
  process.stdout.write(`${JSON.stringify(event)}\n`);
}
