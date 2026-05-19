import { promises as fs } from "node:fs";
import * as path from "node:path";

import { rpc, RpcError } from "../cli/client";
import { SocketServer } from "./server";

describe("SocketServer", () => {
  let tmpRoot: string;
  let originalXdg: string | undefined;
  let server: SocketServer | undefined;

  beforeEach(async () => {
    // Keep this path short — Unix sockets cap at 104 chars on macOS, and the
    // default os.tmpdir() under /var/folders is already 50+ chars before we
    // append /sweetpad/sockets/<name>.sock.
    tmpRoot = await fs.mkdtemp("/tmp/sw-");
    originalXdg = process.env.XDG_STATE_HOME;
    process.env.XDG_STATE_HOME = tmpRoot;
  });

  afterEach(async () => {
    if (server) await server.dispose();
    server = undefined;
    if (originalXdg === undefined) {
      delete process.env.XDG_STATE_HOME;
    } else {
      process.env.XDG_STATE_HOME = originalXdg;
    }
    await fs.rm(tmpRoot, { recursive: true, force: true });
  });

  it("round-trips a JSON-RPC call end-to-end over the Unix socket", async () => {
    server = new SocketServer({
      workspacePath: "/fake/workspace",
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

  it("writes a metadata sidecar with correct fields", async () => {
    server = new SocketServer({
      workspacePath: "/some/workspace",
      extensionVersion: "9.9.9",
      handlers: {},
    });
    await server.start();

    const metaPath = path.join(tmpRoot, "sweetpad", "sockets", `${server.name}.json`);
    const raw = await fs.readFile(metaPath, "utf8");
    const meta = JSON.parse(raw);
    expect(meta.name).toBe(server.name);
    expect(meta.workspacePath).toBe("/some/workspace");
    expect(meta.extensionVersion).toBe("9.9.9");
    expect(meta.protocolVersion).toBe("1.0");
    expect(typeof meta.pid).toBe("number");
    expect(typeof meta.startedAt).toBe("string");
  });

  it("removes the socket and sidecar on dispose", async () => {
    server = new SocketServer({
      workspacePath: "/fake/workspace",
      extensionVersion: "test",
      handlers: {},
    });
    await server.start();
    const socketPath = server.socket;
    const metaPath = path.join(tmpRoot, "sweetpad", "sockets", `${server.name}.json`);

    await server.dispose();
    server = undefined;

    await expect(fs.access(socketPath)).rejects.toThrow(/ENOENT/);
    await expect(fs.access(metaPath)).rejects.toThrow(/ENOENT/);
  });

  it("surfaces RPC errors with the application code in error.data", async () => {
    server = new SocketServer({
      workspacePath: "/fake/workspace",
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
});
