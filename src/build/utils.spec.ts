import { launchActionToSettings } from "./utils";

type ArgInput = { argument: string; isEnabled?: boolean };
type EnvInput = { key: string; value?: string; isEnabled?: boolean };

// Build the subset of a parsed scheme (`sweetpadLib.SchemeInfo`) that
// `launchActionToSettings` reads, defaulting each row to enabled.
function launch(over: { args?: ArgInput[]; env?: EnvInput[]; language?: string; region?: string }) {
  return {
    launchArguments: (over.args ?? []).map((a) => ({ argument: a.argument, isEnabled: a.isEnabled ?? true })),
    launchEnvironmentVariables: (over.env ?? []).map((e) => ({
      key: e.key,
      value: e.value,
      isEnabled: e.isEnabled ?? true,
    })),
    launchLanguage: over.language,
    launchRegion: over.region,
  };
}

describe("launchActionToSettings", () => {
  it("returns empty settings for a bare launch action", () => {
    expect(launchActionToSettings(launch({}))).toEqual({ args: [], env: {} });
  });

  it("auto-injects -AppleLanguages and -AppleLocale from language + region", () => {
    expect(launchActionToSettings(launch({ language: "he", region: "IL" })).args).toEqual([
      "-AppleLanguages",
      "(he)",
      "-AppleLocale",
      "he_IL",
    ]);
  });

  it("emits -AppleLanguages but not -AppleLocale when only language is set", () => {
    expect(launchActionToSettings(launch({ language: "ar" })).args).toEqual(["-AppleLanguages", "(ar)"]);
  });

  it("emits no locale flags when only region is set (bare region is not a valid locale id)", () => {
    expect(launchActionToSettings(launch({ region: "JP" })).args).toEqual([]);
  });

  it("tokenizes command-line argument rows on whitespace", () => {
    expect(
      launchActionToSettings(launch({ args: [{ argument: "-AppleLanguages (he)" }, { argument: "--flag" }] })).args,
    ).toEqual(["-AppleLanguages", "(he)", "--flag"]);
  });

  it("skips disabled command-line arguments", () => {
    expect(
      launchActionToSettings(launch({ args: [{ argument: "--keep" }, { argument: "--skip", isEnabled: false }] })).args,
    ).toEqual(["--keep"]);
  });

  it("collects enabled environment variables and drops disabled ones", () => {
    expect(
      launchActionToSettings(
        launch({
          env: [
            { key: "KEEP", value: "1" },
            { key: "SKIP", value: "x", isEnabled: false },
          ],
        }),
      ).env,
    ).toEqual({ KEEP: "1" });
  });

  it("drops environment variables with no value (distinct from empty string)", () => {
    expect(launchActionToSettings(launch({ env: [{ key: "NOVALUE" }, { key: "EMPTY", value: "" }] })).env).toEqual({
      EMPTY: "",
    });
  });

  it("keeps both explicit locale args and language/region attrs (discussion #197)", () => {
    const { args } = launchActionToSettings(
      launch({
        args: [
          { argument: "-AppleLanguages (he)" },
          { argument: "-AppleLocale he_IL" },
          { argument: "-WMFVisualTestBatchRecordMode" },
        ],
        language: "he",
        region: "IL",
      }),
    );
    // The explicit CLI args and the language/region attrs both flow through;
    // Foundation reads the first match at launch.
    expect(args).toContain("-WMFVisualTestBatchRecordMode");
    expect(args.filter((a) => a === "-AppleLanguages")).toHaveLength(2);
    expect(args.filter((a) => a === "-AppleLocale")).toHaveLength(2);
    expect(args).toContain("(he)");
    expect(args).toContain("he_IL");
  });
});
