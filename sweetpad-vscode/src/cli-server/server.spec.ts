import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { rpc, RpcError } from "../cli/client";
import { getProjectsIndexFile } from "./paths";
import { projectKey, registerControlServer } from "./registry";
import { CliServer } from "./server";
import { PROTOCOL_VERSION } from "./types";

// Above any real pid on macOS and Linux, so `process.kill(pid, 0)` always reports it gone.
const DEAD_PID = 0x7fffffff;

describe("CliServer", () => {
  // A real temp dir as the workspace; an isolated XDG_STATE_HOME so the discovery
  // index never touches the developer's real ~/.local/state. The socket lives in
  // tmpdir (short path).
  let workspacePath: string;
  let stateHome: string;
  let prevStateHome: string | undefined;
  let server: CliServer | undefined;

  beforeEach(async () => {
    workspacePath = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-server-spec-"));
    stateHome = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-state-spec-"));
    prevStateHome = process.env.XDG_STATE_HOME;
    process.env.XDG_STATE_HOME = stateHome;
  });

  afterEach(async () => {
    if (server) await server.dispose();
    server = undefined;
    if (prevStateHome === undefined) delete process.env.XDG_STATE_HOME;
    else process.env.XDG_STATE_HOME = prevStateHome;
    await fs.rm(workspacePath, { recursive: true, force: true });
    await fs.rm(stateHome, { recursive: true, force: true });
  });

  async function controlEntry(): Promise<Record<string, unknown> | undefined> {
    const index = JSON.parse(await fs.readFile(getProjectsIndexFile(), "utf8"));
    return index.projects[await projectKey(workspacePath)]?.control;
  }

  it("round-trips a JSON-RPC call end-to-end over the Unix socket", async () => {
    server = new CliServer({
      workspacePath,
      extensionVersion: "test",
      handlers: {
        "echo.test": (params) => ({ received: params }),
      },
    });
    await server.start();

    const result = await rpc<{ received: { hello: string } }>({
      socketPath: server.socket,
      method: "echo.test",
      params: { hello: "world" },
    });
    expect(result.received).toEqual({ hello: "world" });
  });

  it("registers a control-server index entry with correct fields", async () => {
    server = new CliServer({
      workspacePath,
      extensionVersion: "9.9.9",
      handlers: {},
    });
    await server.start();

    const meta = await controlEntry();
    expect(meta?.name).toBe(server.name);
    expect(meta?.socket).toBe(server.socket);
    expect(meta?.workspacePath).toBe(workspacePath);
    expect(meta?.extensionVersion).toBe("9.9.9");
    expect(meta?.protocolVersion).toBe("1.0");
    expect(typeof meta?.pid).toBe("number");
    expect(typeof meta?.startedAt).toBe("string");
  });

  it("removes the socket and the control entry on dispose", async () => {
    server = new CliServer({
      workspacePath,
      extensionVersion: "test",
      handlers: {},
    });
    await server.start();
    const socketPath = server.socket;
    expect(await controlEntry()).toBeDefined();

    await server.dispose();
    server = undefined;

    await expect(fs.access(socketPath)).rejects.toThrow(/ENOENT/);
    expect(await controlEntry()).toBeUndefined();
  });

  // The CLI resolves a socket by walking up from its cwd, so in a multi-root workspace a folder
  // with no entry reports "no running server" even though the window is open and serving.
  describe("multi-root registration", () => {
    let secondPath: string;

    beforeEach(async () => {
      secondPath = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-server-spec-2nd-"));
    });

    afterEach(async () => {
      await fs.rm(secondPath, { recursive: true, force: true });
    });

    async function controlEntryFor(folder: string): Promise<Record<string, unknown> | undefined> {
      const index = JSON.parse(await fs.readFile(getProjectsIndexFile(), "utf8"));
      return index.projects[await projectKey(folder)]?.control;
    }

    /** Stand in for another VS Code window that already advertises `folder`. */
    async function seedForeignOwner(folder: string, pid: number, socket: string): Promise<void> {
      await registerControlServer(folder, {
        name: "other-window",
        socket,
        workspacePath: folder,
        pid,
        startedAt: new Date().toISOString(),
        extensionVersion: "test",
        protocolVersion: PROTOCOL_VERSION,
      });
    }

    it("advertises the same socket under every registered folder", async () => {
      server = new CliServer({
        workspacePath,
        registrationPaths: () => [workspacePath, secondPath],
        extensionVersion: "test",
        handlers: {},
      });
      await server.start();

      expect((await controlEntryFor(workspacePath))?.socket).toBe(server.socket);
      expect((await controlEntryFor(secondPath))?.socket).toBe(server.socket);
    });

    it("retracts every folder on dispose", async () => {
      server = new CliServer({
        workspacePath,
        registrationPaths: () => [workspacePath, secondPath],
        extensionVersion: "test",
        handlers: {},
      });
      await server.start();

      await server.dispose();
      server = undefined;

      expect(await controlEntryFor(workspacePath)).toBeUndefined();
      expect(await controlEntryFor(secondPath)).toBeUndefined();
    });

    it("drops a folder that leaves the workspace and picks up one that joins", async () => {
      let folders = [workspacePath, secondPath];
      server = new CliServer({
        workspacePath,
        registrationPaths: () => folders,
        extensionVersion: "test",
        handlers: {},
      });
      await server.start();
      expect(await controlEntryFor(secondPath)).toBeDefined();

      folders = [workspacePath];
      await server.syncRegistrations();

      expect(await controlEntryFor(secondPath)).toBeUndefined();
      expect(await controlEntryFor(workspacePath)).toBeDefined();
    });

    it("defers on a folder a live window already owns, and leaves it standing on dispose", async () => {
      // The parent process is a foreign pid that is certainly still running.
      await seedForeignOwner(secondPath, process.ppid, "/tmp/other-window.sock");

      server = new CliServer({
        workspacePath,
        registrationPaths: () => [workspacePath, secondPath],
        extensionVersion: "test",
        handlers: {},
      });
      await server.start();

      expect((await controlEntryFor(secondPath))?.socket).toBe("/tmp/other-window.sock");
      expect((await controlEntryFor(workspacePath))?.socket).toBe(server.socket);

      await server.dispose();
      server = undefined;

      expect((await controlEntryFor(secondPath))?.socket).toBe("/tmp/other-window.sock");
    });

    it("reclaims a folder whose owning window is gone", async () => {
      await seedForeignOwner(secondPath, DEAD_PID, "/tmp/stale-window.sock");

      server = new CliServer({
        workspacePath,
        registrationPaths: () => [workspacePath, secondPath],
        extensionVersion: "test",
        handlers: {},
      });
      await server.start();

      expect((await controlEntryFor(secondPath))?.socket).toBe(server.socket);
    });

    it("leaves nothing behind when dispose races an in-flight sync", async () => {
      server = new CliServer({
        workspacePath,
        registrationPaths: () => [workspacePath, secondPath],
        extensionVersion: "test",
        handlers: {},
      });
      await server.start();

      const inFlight = server.syncRegistrations();
      const teardown = server.dispose();
      await Promise.all([inFlight, teardown]);
      server = undefined;

      expect(await controlEntryFor(workspacePath)).toBeUndefined();
      expect(await controlEntryFor(secondPath)).toBeUndefined();
    });

    it("converges on the latest folder set when two syncs overlap", async () => {
      let folders = [workspacePath, secondPath];
      server = new CliServer({
        workspacePath,
        registrationPaths: () => folders,
        extensionVersion: "test",
        handlers: {},
      });
      await server.start();

      const first = server.syncRegistrations();
      folders = [workspacePath];
      const second = server.syncRegistrations();
      await Promise.all([first, second]);

      expect(await controlEntryFor(secondPath)).toBeUndefined();
      expect(await controlEntryFor(workspacePath)).toBeDefined();
    });
  });

  it("surfaces RPC errors with the application code in error.data", async () => {
    server = new CliServer({
      workspacePath,
      extensionVersion: "test",
      handlers: {
        "fail.now": () => {
          throw new Error("planned failure");
        },
      },
    });
    await server.start();

    await expect(
      rpc({
        socketPath: server.socket,
        method: "fail.now",
        params: {},
      }),
    ).rejects.toBeInstanceOf(RpcError);
  });

  it("answers an unknown method with JSON-RPC method-not-found (-32601)", async () => {
    server = new CliServer({
      workspacePath,
      extensionVersion: "test",
      handlers: {},
    });
    await server.start();

    const err = await rpc({ socketPath: server.socket, method: "does.not.exist", params: {} }).catch((e) => e);
    expect(err).toBeInstanceOf(RpcError);
    expect((err as RpcError).code).toBe(-32601);
  });
});
