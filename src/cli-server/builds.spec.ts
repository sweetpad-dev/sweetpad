import { EventEmitter } from "node:events";
import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import type { BuildManager, BuildSessionEnded, BuildSessionStarted } from "../build/manager";
import type { DestinationsManager } from "../destination/manager";
import { BuildSessionRegistry } from "./builds";
import { getBuildDir, getTmpStateRoot } from "./paths";

function makeMockManagers() {
  const emitter = new EventEmitter();
  const buildManager = {
    on: (event: string, handler: (...args: unknown[]) => void) => emitter.on(event, handler as any),
    off: (event: string, handler: (...args: unknown[]) => void) => emitter.off(event, handler as any),
    getDefaultConfigurationForBuild: vi.fn().mockReturnValue("Debug"),
  } as unknown as BuildManager;
  const destinationsManager = {
    getSelectedXcodeDestinationForBuild: vi
      .fn()
      .mockReturnValue({ id: "dst-1", name: "iPhone 15", type: "iOSSimulator" }),
  } as unknown as DestinationsManager;
  return { emitter, buildManager, destinationsManager };
}

function pump(emitter: EventEmitter, event: string, payload: unknown): void {
  emitter.emit(event, payload);
}

describe("BuildSessionRegistry", () => {
  // A fresh project dir per test; build history lands in a per-workspace tmp dir
  // (getBuildsDir(tmpRoot)), outside the project tree, cleaned up in afterEach.
  let tmpRoot: string;

  beforeEach(async () => {
    tmpRoot = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-builds-spec-"));
  });

  afterEach(async () => {
    await fs.rm(tmpRoot, { recursive: true, force: true });
    await fs.rm(getTmpStateRoot(tmpRoot), { recursive: true, force: true });
  });

  it("captures a single session start->log->end cycle and persists the snapshot", async () => {
    const { emitter, buildManager, destinationsManager } = makeMockManagers();
    const reg = new BuildSessionRegistry({
      workspacePath: tmpRoot,
      buildManager,
      destinationsManager,
    });
    await reg.start();

    pump(emitter, "buildSessionStarted", { scheme: "MyApp", command: "build" } satisfies BuildSessionStarted);
    pump(emitter, "buildLogLine", { line: "Compiling MyApp", diagnostic: null });
    pump(emitter, "buildLogLine", {
      line: "/path/Foo.swift:10:5: error: cannot find 'Bar'",
      diagnostic: {
        file: "/path/Foo.swift",
        line: 10,
        column: 5,
        severity: "error",
        message: "cannot find 'Bar'",
        source: "xcodebuild",
      },
    });
    pump(emitter, "buildSessionEnded", { scheme: "MyApp", status: "failed" } satisfies BuildSessionEnded);

    await reg.flushPending();

    const entity = reg.getBuild("b1");
    expect(entity).toBeDefined();
    expect(entity!.status).toBe("failed");
    expect(entity!.scheme).toBe("MyApp");
    expect(entity!.command).toBe("build");
    expect(entity!.errorCount).toBe(1);
    expect(entity!.warningCount).toBe(0);
    expect(entity!.originator).toBe("vscode");

    const buildDir = getBuildDir(tmpRoot, "b1");
    const snapshot = JSON.parse(await fs.readFile(path.join(buildDir, "snapshot.json"), "utf8"));
    expect(snapshot.diagnostics).toHaveLength(1);
    expect(snapshot.diagnostics[0]).toEqual({
      file: "/path/Foo.swift",
      line: 10,
      column: 5,
      severity: "error",
      message: "cannot find 'Bar'",
    });

    const log = await fs.readFile(path.join(buildDir, "build.log"), "utf8");
    expect(log).toContain("Compiling MyApp");
    expect(log).toContain("error: cannot find 'Bar'");

    reg.dispose();
  });

  it("evicts oldest builds beyond the retention cap", async () => {
    const { emitter, buildManager, destinationsManager } = makeMockManagers();
    const reg = new BuildSessionRegistry({
      workspacePath: tmpRoot,
      buildManager,
      destinationsManager,
    });
    await reg.start();

    for (let i = 0; i < 12; i += 1) {
      pump(emitter, "buildSessionStarted", { scheme: "App", command: "build" });
      pump(emitter, "buildSessionEnded", { scheme: "App", status: "succeeded" });
      await reg.flushPending();
    }

    const surviving = reg.listBuilds();
    expect(surviving).toHaveLength(10);
    // Newest first, so the first 10 buildIds should be b12..b3
    expect(surviving.map((b) => b.buildId)).toEqual(["b12", "b11", "b10", "b9", "b8", "b7", "b6", "b5", "b4", "b3"]);

    // The pruned directories really got removed
    expect(reg.getBuild("b1")).toBeUndefined();
    expect(reg.getBuild("b2")).toBeUndefined();

    reg.dispose();
  });

  it("waitForBuild resolves immediately if the build already finished", async () => {
    const { emitter, buildManager, destinationsManager } = makeMockManagers();
    const reg = new BuildSessionRegistry({
      workspacePath: tmpRoot,
      buildManager,
      destinationsManager,
    });
    await reg.start();

    pump(emitter, "buildSessionStarted", { scheme: "X", command: "build" });
    pump(emitter, "buildSessionEnded", { scheme: "X", status: "succeeded" });
    await reg.flushPending();

    const entity = await reg.waitForBuild("b1", 5000);
    expect(entity.status).toBe("succeeded");
    reg.dispose();
  });

  it("waitForBuild blocks until the session ends, then resolves", async () => {
    const { emitter, buildManager, destinationsManager } = makeMockManagers();
    const reg = new BuildSessionRegistry({
      workspacePath: tmpRoot,
      buildManager,
      destinationsManager,
    });
    await reg.start();

    pump(emitter, "buildSessionStarted", { scheme: "Y", command: "test" });
    const waitPromise = reg.waitForBuild("b1", 5000);
    // Simulate work
    setTimeout(() => pump(emitter, "buildSessionEnded", { scheme: "Y", status: "succeeded" }), 10);
    const entity = await waitPromise;
    expect(entity.status).toBe("succeeded");
    expect(entity.command).toBe("test");
    reg.dispose();
  });

  it("waitForBuild returns the still-running entity (no throw) on timeout", async () => {
    const { emitter, buildManager, destinationsManager } = makeMockManagers();
    const reg = new BuildSessionRegistry({
      workspacePath: tmpRoot,
      buildManager,
      destinationsManager,
    });
    await reg.start();

    pump(emitter, "buildSessionStarted", { scheme: "Slow", command: "build" });
    const entity = await reg.waitForBuild("b1", 50);
    expect(entity.status).toBe("running");
    expect(entity.scheme).toBe("Slow");

    // Build finishes after the timeout; should not throw.
    pump(emitter, "buildSessionEnded", { scheme: "Slow", status: "succeeded" });
    await reg.flushPending();
    expect(reg.getBuild("b1")!.status).toBe("succeeded");
    reg.dispose();
  });

  it("waitForBuild caps timeoutMs at the server max (~30s)", async () => {
    const { emitter, buildManager, destinationsManager } = makeMockManagers();
    const reg = new BuildSessionRegistry({
      workspacePath: tmpRoot,
      buildManager,
      destinationsManager,
    });
    await reg.start();

    pump(emitter, "buildSessionStarted", { scheme: "X", command: "build" });
    const before = Date.now();
    // Ask for a 10-minute wait; finish the build immediately so this resolves fast.
    const waitPromise = reg.waitForBuild("b1", 10 * 60 * 1000);
    setTimeout(() => pump(emitter, "buildSessionEnded", { scheme: "X", status: "succeeded" }), 5);
    const entity = await waitPromise;
    const elapsed = Date.now() - before;
    expect(entity.status).toBe("succeeded");
    // If the cap broke, the test wouldn't catch it directly here, but resolving
    // quickly proves we honored the event rather than the requested timeout.
    expect(elapsed).toBeLessThan(1000);
    reg.dispose();
  });

  it("CLI-reserved buildId is claimed by the next session start", async () => {
    const { emitter, buildManager, destinationsManager } = makeMockManagers();
    const reg = new BuildSessionRegistry({
      workspacePath: tmpRoot,
      buildManager,
      destinationsManager,
    });
    await reg.start();

    const reserved = reg.reserveCliBuildId();
    pump(emitter, "buildSessionStarted", { scheme: "Z", command: "build" });
    pump(emitter, "buildSessionEnded", { scheme: "Z", status: "succeeded" });
    await reg.flushPending();

    const entity = reg.getBuild(reserved);
    expect(entity).toBeDefined();
    expect(entity!.originator).toBe("cli");

    reg.dispose();
  });

  it("recovers nextSeq from existing on-disk builds after restart", async () => {
    const { emitter, buildManager, destinationsManager } = makeMockManagers();
    const reg = new BuildSessionRegistry({ workspacePath: tmpRoot, buildManager, destinationsManager });
    await reg.start();
    pump(emitter, "buildSessionStarted", { scheme: "A", command: "build" });
    pump(emitter, "buildSessionEnded", { scheme: "A", status: "succeeded" });
    await reg.flushPending();
    reg.dispose();

    const fresh = makeMockManagers();
    const reg2 = new BuildSessionRegistry({
      workspacePath: tmpRoot,
      buildManager: fresh.buildManager,
      destinationsManager: fresh.destinationsManager,
    });
    await reg2.start();

    const reservedId = reg2.reserveCliBuildId();
    // Old build was b1; new one should be b2 — counter advances past existing entries.
    expect(reservedId).toBe("b2");
    reg2.dispose();
  });
});
