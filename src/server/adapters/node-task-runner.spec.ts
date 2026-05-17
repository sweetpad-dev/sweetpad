import { describe, expect, it } from "vitest";

import { noopLogger } from "../../core/logger/types";
import type { ConfigProvider } from "../../core/config/types";
import type { WorkspaceRoot } from "../../core/workspace-root";
import { NodeTaskRunner } from "./node-task-runner";

function makeRunner(): NodeTaskRunner {
  const workspaceRoot: WorkspaceRoot = {
    getPath: () => process.cwd(),
    getStoragePath: async () => process.cwd(),
    getRelativePath: (p) => p,
  };
  const config = {
    get: () => undefined,
    isDefined: () => false,
    update: async () => {},
  } as unknown as ConfigProvider;
  return new NodeTaskRunner({ workspaceRoot, config, logger: noopLogger });
}

/**
 * Exercises the real ProcessGroup: spawn detached children, await exit,
 * cleanup on callback return. No mocks — these run actual `sh` processes
 * because the contract is "kills the process group", and that only has
 * meaning at the kernel level.
 */
describe("NodeTaskRunner.runGroup — pipe-mode", () => {
  it("returns the callback's value", async () => {
    const runner = makeRunner();
    let runGroupResult: number | undefined;
    await runner.run({
      name: "t",
      lock: "test.runGroup.value",
      terminateLocked: false,
      callback: async (terminal) => {
        runGroupResult = await terminal.runGroup(async () => 42);
      },
    });
    expect(runGroupResult).toBe(42);
  });

  it("spawned children's onData callbacks receive stdout chunks", async () => {
    const runner = makeRunner();
    const lines: string[] = [];

    await runner.run({
      name: "t",
      lock: "test.runGroup.onData",
      terminateLocked: false,
      callback: async (terminal) => {
        await terminal.runGroup(async (group) => {
          const handle = group.spawn({ command: "sh", args: ["-c", "echo hello world"] });
          handle.onData((chunk) => lines.push(chunk));
          await handle.exit;
        });
      },
    });

    expect(lines.join("")).toContain("hello world");
  });

  it("separates stdout and stderr when pty=false (default)", async () => {
    const runner = makeRunner();
    const stdoutChunks: string[] = [];
    const stderrChunks: string[] = [];

    await runner.run({
      name: "t",
      lock: "test.runGroup.split",
      terminateLocked: false,
      callback: async (terminal) => {
        await terminal.runGroup(async (group) => {
          const handle = group.spawn({
            command: "sh",
            args: ["-c", "echo OUT; echo ERR 1>&2"],
          });
          handle.onData((c) => stdoutChunks.push(c));
          handle.onError((c) => stderrChunks.push(c));
          await handle.exit;
        });
      },
    });

    expect(stdoutChunks.join("")).toContain("OUT");
    expect(stderrChunks.join("")).toContain("ERR");
    expect(stdoutChunks.join("")).not.toContain("ERR");
  });

  it("merges stderr into onData when pty=true (matches contract)", async () => {
    const runner = makeRunner();
    const data: string[] = [];
    const errors: string[] = [];

    await runner.run({
      name: "t",
      lock: "test.runGroup.merge",
      terminateLocked: false,
      callback: async (terminal) => {
        await terminal.runGroup(async (group) => {
          const handle = group.spawn({
            command: "sh",
            args: ["-c", "echo OUT; echo ERR 1>&2"],
            pty: true,
          });
          handle.onData((c) => data.push(c));
          handle.onError((c) => errors.push(c));
          await handle.exit;
        });
      },
    });

    expect(data.join("")).toContain("OUT");
    expect(data.join("")).toContain("ERR");
    expect(errors).toEqual([]);
  });

  it("kills surviving children when the callback returns", async () => {
    const runner = makeRunner();
    let exitCode: number | null = null;
    let exitSignal: NodeJS.Signals | null = null;

    await runner.run({
      name: "t",
      lock: "test.runGroup.cleanup",
      terminateLocked: false,
      callback: async (terminal) => {
        await terminal.runGroup(async (group) => {
          // `sleep 30` long enough that the callback returns first.
          const handle = group.spawn({ command: "sh", args: ["-c", "sleep 30"] });
          // Don't await the long sleep — return immediately so the cleanup
          // path is what kills it.
          handle.exit.then((e) => {
            exitCode = e.code;
            exitSignal = e.signal;
          });
        });
      },
    });

    // Give the kill a moment to propagate.
    await new Promise((r) => setTimeout(r, 200));
    expect(exitSignal === "SIGTERM" || exitCode === -1).toBe(true);
  });

  it("kill() on a handle terminates that specific process", async () => {
    const runner = makeRunner();
    let exitSignal: NodeJS.Signals | null = null;

    await runner.run({
      name: "t",
      lock: "test.runGroup.kill",
      terminateLocked: false,
      callback: async (terminal) => {
        await terminal.runGroup(async (group) => {
          const handle = group.spawn({ command: "sh", args: ["-c", "sleep 30"] });
          handle.kill("SIGTERM");
          const e = await handle.exit;
          exitSignal = e.signal;
        });
      },
    });

    expect(exitSignal).toBe("SIGTERM");
  });
});
