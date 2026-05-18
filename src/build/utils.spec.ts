import { readFileSync } from "node:fs";
import path from "node:path";

import { LaunchAction, SchemeDocument } from "../common/xcode/xcscheme";
import { launchActionToSettings } from "./utils";

const FIXTURE_DIR = path.resolve(__dirname, "../../tests/xcscheme-data");

function loadLaunchAction(fixture: string): LaunchAction {
  const xml = readFileSync(path.join(FIXTURE_DIR, fixture), "utf8");
  const doc = SchemeDocument.parse(xml);
  const action = doc.launchAction();
  if (!action) throw new Error(`Fixture ${fixture} has no <LaunchAction>`);
  return action;
}

describe("launchActionToSettings", () => {
  it("returns empty settings for a bare LaunchAction", () => {
    const action = new LaunchAction();
    expect(launchActionToSettings(action)).toEqual({ args: [], env: {} });
  });

  it("auto-injects -AppleLanguages and -AppleLocale from language + region", () => {
    const action = new LaunchAction();
    action.language = "he";
    action.region = "IL";
    expect(launchActionToSettings(action).args).toEqual(["-AppleLanguages", "(he)", "-AppleLocale", "he_IL"]);
  });

  it("emits -AppleLanguages but not -AppleLocale when only language is set", () => {
    const action = new LaunchAction();
    action.language = "ar";
    expect(launchActionToSettings(action).args).toEqual(["-AppleLanguages", "(ar)"]);
  });

  it("emits no locale flags when only region is set (bare region is not a valid locale id)", () => {
    const action = new LaunchAction();
    action.region = "JP";
    expect(launchActionToSettings(action).args).toEqual([]);
  });

  it("tokenizes <CommandLineArgument> rows on whitespace", () => {
    const action = new LaunchAction();
    action.addCommandLineArgument({ argument: "-AppleLanguages (he)" });
    action.addCommandLineArgument({ argument: "--flag" });
    expect(launchActionToSettings(action).args).toEqual(["-AppleLanguages", "(he)", "--flag"]);
  });

  it("skips disabled CommandLineArguments", () => {
    const action = new LaunchAction();
    action.addCommandLineArgument({ argument: "--keep" });
    action.addCommandLineArgument({ argument: "--skip", isEnabled: false });
    expect(launchActionToSettings(action).args).toEqual(["--keep"]);
  });

  it("collects enabled EnvironmentVariables and drops disabled ones", () => {
    const action = new LaunchAction();
    action.addEnvironmentVariable({ key: "KEEP", value: "1" });
    action.addEnvironmentVariable({ key: "SKIP", value: "x", isEnabled: false });
    expect(launchActionToSettings(action).env).toEqual({ KEEP: "1" });
  });

  it("extracts the discussion #197 wikipedia-ios-rtl fixture end-to-end", () => {
    const action = loadLaunchAction("wikipedia-ios-rtl.xcscheme");
    const { args, env } = launchActionToSettings(action);
    // The fixture has language="he" region="IL" *and* explicit -AppleLanguages /
    // -AppleLocale CLI args. Both flow through (Xcode keeps both; Foundation
    // reads the first match).
    expect(args).toContain("-WMFVisualTestBatchRecordMode");
    expect(args.filter((a) => a === "-AppleLanguages")).toHaveLength(2);
    expect(args.filter((a) => a === "-AppleLocale")).toHaveLength(2);
    expect(args).toContain("(he)");
    expect(args).toContain("he_IL");
    expect(env).toEqual({});
  });

  it("extracts EnvironmentVariables from the signal-ios-staging fixture", () => {
    const action = loadLaunchAction("signal-ios-staging.xcscheme");
    const { env } = launchActionToSettings(action);
    expect(env.OS_ACTIVITY_MODE).toBe("disable");
    expect(env.USE_STAGING).toBe("1");
  });
});
