import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { getProjectsIndexFile } from "./paths";
import { projectKey, registerProject, unregisterProject } from "./registry";
import type { CliServerMetadata } from "./types";

function meta(workspacePath: string, overrides: Partial<CliServerMetadata> = {}): CliServerMetadata {
  return {
    name: "abc123",
    socket: "/tmp/sweetpad-abc123.sock",
    workspacePath,
    pid: 4242,
    startedAt: new Date().toISOString(),
    extensionVersion: "test",
    protocolVersion: "1.0",
    ...overrides,
  };
}

describe("registry", () => {
  let workspacePath: string;
  let stateHome: string;
  let prev: string | undefined;

  beforeEach(async () => {
    workspacePath = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-reg-ws-"));
    stateHome = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-reg-state-"));
    prev = process.env.XDG_STATE_HOME;
    process.env.XDG_STATE_HOME = stateHome;
  });

  afterEach(async () => {
    if (prev === undefined) delete process.env.XDG_STATE_HOME;
    else process.env.XDG_STATE_HOME = prev;
    await fs.rm(workspacePath, { recursive: true, force: true });
    await fs.rm(stateHome, { recursive: true, force: true });
  });

  async function readEntry(): Promise<CliServerMetadata | undefined> {
    const index = JSON.parse(await fs.readFile(getProjectsIndexFile(), "utf8"));
    return index.projects[await projectKey(workspacePath)];
  }

  it("registers under the canonical (realpath) key", async () => {
    await registerProject(workspacePath, meta(workspacePath));
    const index = JSON.parse(await fs.readFile(getProjectsIndexFile(), "utf8"));
    expect(index.version).toBe(1);
    expect(Object.keys(index.projects)).toEqual([await fs.realpath(workspacePath)]);
    expect((await readEntry())?.socket).toBe("/tmp/sweetpad-abc123.sock");
  });

  it("last-writer-wins: a second register replaces the entry", async () => {
    await registerProject(workspacePath, meta(workspacePath, { name: "first", pid: 1 }));
    await registerProject(workspacePath, meta(workspacePath, { name: "second", pid: 2 }));
    expect((await readEntry())?.name).toBe("second");
  });

  it("unregister removes our entry", async () => {
    await registerProject(workspacePath, meta(workspacePath, { pid: 7 }));
    await unregisterProject(workspacePath, 7);
    expect(await readEntry()).toBeUndefined();
  });

  it("unregister leaves a newer window's entry intact", async () => {
    await registerProject(workspacePath, meta(workspacePath, { name: "newer", pid: 99 }));
    // An older server tearing down must not clobber the newer pointer.
    await unregisterProject(workspacePath, 7);
    expect((await readEntry())?.name).toBe("newer");
  });
});
