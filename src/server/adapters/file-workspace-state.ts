import * as fs from "node:fs";
import * as path from "node:path";

import type { WorkspaceState, WorkspaceStateKey, WorkspaceTypes } from "../../core/state/types";

const STATE_FILE_NAME = "state.json";
const STATE_DIR = ".sweetpad";

/**
 * File-backed `WorkspaceState`, persisted as `<workspace>/.sweetpad/state.json`.
 * Loaded lazily on first access; writes are synchronous to keep the in-memory
 * map and the on-disk JSON in lockstep (no async race between `update` calls
 * and a server crash). Synchronous fs is fine here: the state object is small
 * (a few KB at most) and writes are infrequent compared to the build itself.
 */
export class FileWorkspaceState implements WorkspaceState {
  private readonly filePath: string;
  private cache: Record<string, unknown> | undefined;

  constructor(workspacePath: string) {
    this.filePath = path.join(workspacePath, STATE_DIR, STATE_FILE_NAME);
  }

  get<K extends WorkspaceStateKey>(key: K): WorkspaceTypes[K] | undefined {
    return this.load()[key] as WorkspaceTypes[K] | undefined;
  }

  update<K extends WorkspaceStateKey>(key: K, value: WorkspaceTypes[K] | undefined): void {
    const state = this.load();
    if (value === undefined) {
      delete state[key];
    } else {
      state[key] = value;
    }
    this.persist();
  }

  reset(): void {
    this.cache = {};
    this.persist();
  }

  private load(): Record<string, unknown> {
    if (this.cache) return this.cache;
    try {
      const raw = fs.readFileSync(this.filePath, "utf8");
      this.cache = JSON.parse(raw);
    } catch {
      // ENOENT, malformed JSON, etc. — treat as empty state.
      this.cache = {};
    }
    return this.cache as Record<string, unknown>;
  }

  private persist(): void {
    fs.mkdirSync(path.dirname(this.filePath), { recursive: true });
    fs.writeFileSync(this.filePath, `${JSON.stringify(this.cache ?? {}, null, 2)}\n`, "utf8");
  }
}
