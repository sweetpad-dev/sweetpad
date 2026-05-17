import type { PickItem, PickItemRow, UserAsker } from "../../core/asker/types";
import { ProtocolError } from "../../protocol/errors";

/**
 * Headless `UserAsker` for the CLI/server path. The server is never run
 * interactively in v1 — agents pass scheme/destination/configuration via
 * flags. If the engine reaches an asker (e.g. via a code path that the
 * `buildExplicit` carve-out doesn't cover yet), throw a structured error so
 * the agent sees a clear "you didn't pass enough" envelope rather than a hang.
 *
 * TTY interactive mode lives in a follow-up: spawned only when the CLI's
 * `stdin && stdout` are both TTYs and `--json` is unset.
 */
export class CliAsker implements UserAsker {
  pick<T>(_options: { title: string; items: PickItem<T>[] }): Promise<PickItemRow<T>> {
    return Promise.reject(
      new ProtocolError("INVALID_ARGUMENT", "Headless CLI cannot prompt; pass the required flag explicitly"),
    );
  }

  input(_options: { title: string; value?: string }): Promise<string | undefined> {
    return Promise.reject(
      new ProtocolError("INVALID_ARGUMENT", "Headless CLI cannot prompt; pass the required flag explicitly"),
    );
  }
}
