/**
 * Unit tests for finding the `sweetpad` CLI a debug session runs and telling whether it has
 * `sweetpad dap`.
 */

import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

import type { Mock } from "vitest";
import * as vscode from "vscode";

import { getSweetpadCliPath } from "../common/cli/scripts";
import { exec } from "../common/exec";
import { getSweetpadCliStatus, resetSweetpadCliStatusCache } from "./cli";

vi.mock("../common/exec", () => ({ exec: vi.fn() }));

vi.mock("../common/cli/scripts", () => ({ getSweetpadCliPath: vi.fn() }));

vi.mock("../common/logger", () => ({
  commonLogger: { log: vi.fn(), debug: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

let dir: string;
let cliPath: string;

function useCliPathSetting(value: string | null) {
  (vscode.workspace.getConfiguration as Mock).mockImplementation(() => ({
    get: vi.fn((key: string) => (key === "debugger.cliPath" ? value : undefined)),
    inspect: vi.fn(),
  }));
}

beforeEach(() => {
  vi.clearAllMocks();
  resetSweetpadCliStatusCache();
  dir = mkdtempSync(path.join(os.tmpdir(), "sweetpad-cli-spec-"));
  cliPath = path.join(dir, "sweetpad");
  writeFileSync(cliPath, "#!/bin/sh\n");
  chmodSync(cliPath, 0o755);
  useCliPathSetting(cliPath);
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
});

describe("getSweetpadCliStatus", () => {
  it("is ready when 'sweetpad dap --help' succeeds", async () => {
    (exec as Mock).mockResolvedValue("Usage: sweetpad dap");

    expect(await getSweetpadCliStatus()).toEqual({ kind: "ready", path: cliPath });
    expect(exec).toHaveBeenCalledWith({ command: cliPath, args: ["dap", "--help"], cwd: null });
  });

  it("reports a CLI without dap, which exits non-zero on 'sweetpad dap --help'", async () => {
    (exec as Mock).mockRejectedValue(new Error("unrecognized subcommand 'dap'"));

    expect(await getSweetpadCliStatus()).toEqual({ kind: "no-dap", path: cliPath });
  });

  it("probes a ready CLI once", async () => {
    (exec as Mock).mockResolvedValue("Usage: sweetpad dap");

    await getSweetpadCliStatus();
    await getSweetpadCliStatus();

    expect(exec).toHaveBeenCalledTimes(1);
  });

  it("probes again after a CLI without dap, so an upgrade takes effect", async () => {
    (exec as Mock).mockRejectedValueOnce(new Error("unrecognized subcommand 'dap'"));
    (exec as Mock).mockResolvedValueOnce("Usage: sweetpad dap");

    expect((await getSweetpadCliStatus()).kind).toBe("no-dap");
    expect((await getSweetpadCliStatus()).kind).toBe("ready");
  });

  it("probes again when the path setting changes", async () => {
    (exec as Mock).mockResolvedValue("Usage: sweetpad dap");
    await getSweetpadCliStatus();

    const other = path.join(dir, "sweetpad-other");
    writeFileSync(other, "#!/bin/sh\n");
    chmodSync(other, 0o755);
    useCliPathSetting(other);

    expect(await getSweetpadCliStatus()).toEqual({ kind: "ready", path: other });
    expect(exec).toHaveBeenCalledTimes(2);
  });

  it("reports a configured path that is not there, without searching PATH", async () => {
    useCliPathSetting(path.join(dir, "missing"));

    expect(await getSweetpadCliStatus()).toEqual({ kind: "missing", configuredPath: path.join(dir, "missing") });
    expect(getSweetpadCliPath).not.toHaveBeenCalled();
    expect(exec).not.toHaveBeenCalled();
  });

  it("refuses a configured path that is not executable", async () => {
    chmodSync(cliPath, 0o644);

    expect((await getSweetpadCliStatus()).kind).toBe("missing");
  });

  it("uses the CLI on the login shell's PATH when no path is configured", async () => {
    useCliPathSetting(null);
    (getSweetpadCliPath as Mock).mockResolvedValue(cliPath);
    (exec as Mock).mockResolvedValue("Usage: sweetpad dap");

    expect(await getSweetpadCliStatus()).toEqual({ kind: "ready", path: cliPath });
  });
});
