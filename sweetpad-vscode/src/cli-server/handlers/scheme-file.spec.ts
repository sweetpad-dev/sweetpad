import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import * as sweetpadLib from "@sweetpad/native";
import type { Mock } from "vitest";

import { activateCurrentXcodeWorkspacePath } from "../../build/utils";
import type { RpcContext } from "./context";
import { schemeReveal } from "./scheme-file";

vi.mock("@sweetpad/native", () => ({ schemeFiles: vi.fn() }));
vi.mock("../../build/utils", () => ({ activateCurrentXcodeWorkspacePath: vi.fn() }));

const mockSchemeFiles = sweetpadLib.schemeFiles as Mock;
const mockCurrent = activateCurrentXcodeWorkspacePath as Mock;
const ctx = { workspaceState: {}, workspaceContext: {} } as unknown as RpcContext;

describe("scheme.reveal", () => {
  let tmp: string;

  beforeEach(async () => {
    vi.resetAllMocks();
    tmp = await fs.mkdtemp(path.join(os.tmpdir(), "sw-scheme-file-spec-"));
  });

  afterEach(async () => {
    await fs.rm(tmp, { recursive: true, force: true });
  });

  // The lookup is the current project's, the way `xcodebuild` reads it, so a same-named scheme in
  // another project of the folder, or in another user's `xcuserdata`, is never the answer.
  it("reveals the scheme file the current project reads first", async () => {
    const shared = path.join(tmp, "App.xcodeproj", "xcshareddata", "xcschemes", "App.xcscheme");
    await fs.mkdir(path.dirname(shared), { recursive: true });
    await fs.writeFile(shared, "<Scheme/>");
    mockCurrent.mockReturnValue(path.join(tmp, "App.xcworkspace"));
    mockSchemeFiles.mockReturnValue([shared, path.join(tmp, "other.xcscheme")]);

    const out = await schemeReveal({ name: "App" }, ctx);

    expect(mockSchemeFiles).toHaveBeenCalledWith(path.join(tmp, "App.xcworkspace"), "App");
    expect(out).toEqual({
      name: "App",
      path: shared,
      xml: "<Scheme/>",
      allPaths: [shared, path.join(tmp, "other.xcscheme")],
    });
  });

  it("names the missing project when none is selected", async () => {
    mockCurrent.mockReturnValue(undefined);
    await expect(schemeReveal({ name: "App" }, ctx)).rejects.toThrow(/No Xcode workspace/);
    expect(mockSchemeFiles).not.toHaveBeenCalled();
  });

  it("reports a scheme with no file", async () => {
    mockCurrent.mockReturnValue(path.join(tmp, "App.xcodeproj"));
    mockSchemeFiles.mockReturnValue([]);
    await expect(schemeReveal({ name: "Auto" }, ctx)).rejects.toThrow(/No .xcscheme file found for "Auto"/);
  });
});
