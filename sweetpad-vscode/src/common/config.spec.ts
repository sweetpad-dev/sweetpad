import type { Mock } from "vitest";
import * as vscode from "vscode";

import { isWorkspaceConfigSetByUser } from "./config";

const mockGetConfiguration = vscode.workspace.getConfiguration as unknown as Mock;

/** Stand in for what `WorkspaceConfiguration.inspect` reports for a setting. */
function inspectReports(scopes: Record<string, unknown> | undefined): void {
  mockGetConfiguration.mockReturnValue({ get: vi.fn(), inspect: vi.fn(() => scopes) });
}

describe("isWorkspaceConfigSetByUser", () => {
  it("is false when the only value is the one package.json contributes", () => {
    inspectReports({ key: "sweetpad.buildServer.provider", defaultValue: "sweetpad" });

    expect(isWorkspaceConfigSetByUser("buildServer.provider")).toBe(false);
  });

  it.each([["globalValue"], ["workspaceValue"], ["workspaceFolderValue"]])("is true for a %s", (scope) => {
    inspectReports({ defaultValue: "sweetpad", [scope]: "xcode-build-server" });

    expect(isWorkspaceConfigSetByUser("buildServer.provider")).toBe(true);
  });

  it("is true when the chosen value happens to equal the default", () => {
    inspectReports({ defaultValue: "sweetpad", workspaceValue: "sweetpad" });

    // Choosing the same value the manifest suggests is still choosing, and the
    // two answers lead somewhere different: a workspace that picked `sweetpad`
    // wants our server even where one already exists for something else.
    expect(isWorkspaceConfigSetByUser("buildServer.provider")).toBe(true);
  });

  it("is false when the setting is unknown to VS Code", () => {
    inspectReports(undefined);

    expect(isWorkspaceConfigSetByUser("buildServer.provider")).toBe(false);
  });
});
