import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { noopLogger } from "../core/logger/types";
import { LogWriter } from "./log-writer";

describe("LogWriter", () => {
  let dir: string;
  let logPath: string;

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-logwriter-"));
    logPath = path.join(dir, "deep", "log.txt");
  });

  afterEach(() => {
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it("creates parent directories on open", async () => {
    const w = LogWriter.open({ logger: noopLogger, logPath });
    w.write("hello");
    await w.close();
    expect(fs.existsSync(logPath)).toBe(true);
  });

  it("appends a trailing newline per line", async () => {
    const w = LogWriter.open({ logger: noopLogger, logPath });
    w.write("one");
    w.write("two");
    await w.close();
    expect(fs.readFileSync(logPath, "utf8")).toBe("one\ntwo\n");
  });

  it("truncates on reopen (build N starts clean even if N-1 wrote to it)", async () => {
    let w = LogWriter.open({ logger: noopLogger, logPath });
    w.write("stale line from old build");
    await w.close();

    w = LogWriter.open({ logger: noopLogger, logPath });
    w.write("fresh line");
    await w.close();
    expect(fs.readFileSync(logPath, "utf8")).toBe("fresh line\n");
  });

  it("is a no-op when used after close()", async () => {
    const w = LogWriter.open({ logger: noopLogger, logPath });
    w.write("first");
    await w.close();
    w.write("second"); // must not throw
    expect(fs.readFileSync(logPath, "utf8")).toBe("first\n");
  });

  it("supports `await using` for auto-close", async () => {
    async function run() {
      await using w = LogWriter.open({ logger: noopLogger, logPath });
      w.write("hello");
    }
    await run();
    expect(fs.readFileSync(logPath, "utf8")).toBe("hello\n");
  });
});
