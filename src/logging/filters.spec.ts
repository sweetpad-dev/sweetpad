import {
  PassthroughLogFilter,
  Pymobiledevice3JsonLogFilter,
  Pymobiledevice3LogFilter,
  parseSyslogLine,
} from "./filters";

describe("parseSyslogLine", () => {
  it("parses a line with subsystem and category (the --label suffix)", () => {
    const line =
      "2026-04-16 12:52:32.707333 Laboratory{Laboratory.debug.dylib}[67135] <NOTICE>: ContentView initialized [Laboratory Test][App]";
    expect(parseSyslogLine(line)).toEqual({
      timestamp: "2026-04-16 12:52:32.707333",
      processName: "Laboratory",
      imageName: "Laboratory.debug.dylib",
      pid: 67135,
      level: "NOTICE",
      message: "ContentView initialized",
      label: { subsystem: "Laboratory Test", category: "App" },
    });
  });

  it("parses a framework-emitted line (image_name != process_name)", () => {
    const line =
      "2026-04-16 12:52:32.707333 Laboratory{CoreFoundation}[67116] <NOTICE>: looked up value for key AppleLanguages [com.apple.defaults][User Defaults]";
    expect(parseSyslogLine(line)).toMatchObject({
      processName: "Laboratory",
      imageName: "CoreFoundation",
      pid: 67116,
      level: "NOTICE",
      label: { subsystem: "com.apple.defaults", category: "User Defaults" },
    });
  });

  it("parses a line without a trailing label (no --label passed)", () => {
    const line = "2026-04-16 12:52:32.707333 Laboratory{Laboratory}[67135] <DEBUG>: hello world";
    const entry = parseSyslogLine(line);
    expect(entry).toMatchObject({
      imageName: "Laboratory",
      level: "DEBUG",
      message: "hello world",
    });
    expect(entry?.label).toBeUndefined();
  });

  it("extracts image_offset when --image-offset is enabled", () => {
    const line = "2026-04-16 12:52:32.707333 Laboratory{Laboratory+0x1a2b3c}[67135] <NOTICE>: tick [sub][cat]";
    expect(parseSyslogLine(line)).toMatchObject({
      imageName: "Laboratory",
      imageOffset: 0x1a2b3c,
    });
  });

  it("preserves brackets inside the message body and only strips the trailing label", () => {
    const line =
      "2026-04-16 12:52:32.707333 Laboratory{Laboratory.debug.dylib}[1] <NOTICE>: got [inner][pair] and [another] [Laboratory Test][App]";
    expect(parseSyslogLine(line)).toMatchObject({
      message: "got [inner][pair] and [another]",
      label: { subsystem: "Laboratory Test", category: "App" },
    });
  });

  it("strips ANSI color escape sequences before parsing", () => {
    const line = "\x1b[32m2026-04-16 12:52:32.707333\x1b[0m Laboratory{Laboratory}[1] <\x1b[33mNOTICE\x1b[0m>: colored";
    expect(parseSyslogLine(line)).toMatchObject({
      processName: "Laboratory",
      level: "NOTICE",
      message: "colored",
    });
  });

  it("returns null for non-log lines (banners, blank, tunnel notices)", () => {
    expect(parseSyslogLine("")).toBeNull();
    expect(parseSyslogLine("[SweetPad] banner")).toBeNull();
    expect(parseSyslogLine("Connected to remote tunnel")).toBeNull();
  });
});

describe("PassthroughLogFilter", () => {
  it("passes non-empty lines through", () => {
    const filter = new PassthroughLogFilter();
    expect(filter.processLine("hello world")).toBe("hello world");
  });

  it("drops empty lines", () => {
    const filter = new PassthroughLogFilter();
    expect(filter.processLine("")).toBeNull();
  });
});

describe("Pymobiledevice3LogFilter", () => {
  function createFilter(
    options: {
      executableName: string;
      debugDylibOnly?: boolean;
      subsystemDenyList?: string[];
      subsystemAllowList?: string[];
      minLevel?: string;
    } = { executableName: "Laboratory" },
  ) {
    return new Pymobiledevice3LogFilter(options);
  }

  it("drops framework noise", () => {
    const filter = createFilter();
    const noise =
      "2026-04-16 12:52:32.707333 Laboratory{CoreFoundation}[67116] <NOTICE>: noise [com.apple.defaults][User Defaults]";
    expect(filter.processLine(noise)).toBeNull();
  });

  it("keeps app logs from debug dylib", () => {
    const filter = createFilter();
    const appLog =
      "2026-04-16 12:53:59.832625 Laboratory{Laboratory.debug.dylib}[67135] <NOTICE>: hello [Lab Test][App]";
    expect(filter.processLine(appLog)).toBe(appLog);
  });

  it("keeps the main image by default (debugDylibOnly=false)", () => {
    const filter = createFilter();
    const line = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: hi [sub][cat]";
    expect(filter.processLine(line)).toBe(line);
  });

  it("debugDylibOnly=true drops the main image", () => {
    const filter = createFilter({ executableName: "Laboratory", debugDylibOnly: true });
    const mainImage = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <DEBUG>: noise [com.apple.Previews][X]";
    const debugDylib = "2026-04-16 12:53:59.832625 Laboratory{Laboratory.debug.dylib}[1] <NOTICE>: app log [sub][cat]";
    expect(filter.processLine(mainImage)).toBeNull();
    expect(filter.processLine(debugDylib)).toBe(debugDylib);
  });

  it("subsystem deny-list drops matching subsystems", () => {
    const filter = createFilter({
      executableName: "Laboratory",
      subsystemDenyList: ["com.apple.*"],
    });
    const apple = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <DEBUG>: noise [com.apple.CFBundle][resources]";
    const app = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: hi [com.example.app][ui]";
    expect(filter.processLine(apple)).toBeNull();
    expect(filter.processLine(app)).toBe(app);
  });

  it("subsystem deny-list does not affect entries without a label", () => {
    const filter = createFilter({
      executableName: "Laboratory",
      subsystemDenyList: ["com.apple.*"],
    });
    const noLabel = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: no label here";
    expect(filter.processLine(noLabel)).toBe(noLabel);
  });

  it("subsystem allow-list keeps only matching subsystems", () => {
    const filter = createFilter({
      executableName: "Laboratory",
      subsystemAllowList: ["com.example.*"],
    });
    const match = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: a [com.example.app][ui]";
    const other = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: b [com.apple.CFBundle][resources]";
    expect(filter.processLine(match)).toBe(match);
    expect(filter.processLine(other)).toBeNull();
  });

  it("subsystem allow-list drops entries without a label", () => {
    const filter = createFilter({
      executableName: "Laboratory",
      subsystemAllowList: ["com.example.*"],
    });
    const noLabel = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: no label";
    expect(filter.processLine(noLabel)).toBeNull();
  });

  it("deny-list and allow-list work together", () => {
    const filter = createFilter({
      executableName: "Laboratory",
      subsystemDenyList: ["com.apple.*"],
      subsystemAllowList: ["com.example.myapp"],
    });
    const apple = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: a [com.apple.X][Y]";
    const myApp = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: b [com.example.myapp][ui]";
    const otherApp = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: c [com.example.other][ui]";
    expect(filter.processLine(apple)).toBeNull();
    expect(filter.processLine(myApp)).toBe(myApp);
    expect(filter.processLine(otherApp)).toBeNull();
  });

  it("honors an optional minimum level", () => {
    const filter = createFilter({ executableName: "Laboratory", minLevel: "ERROR" });
    const info = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <INFO>: quiet [s][c]";
    const error = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <ERROR>: loud [s][c]";
    expect(filter.processLine(info)).toBeNull();
    expect(filter.processLine(error)).toBe(error);
  });

  it("passes through tunnel notices before the first parsed entry", () => {
    const filter = createFilter();
    expect(filter.processLine("Connected to tunnel")).toBe("Connected to tunnel");
  });

  it("drops empty lines", () => {
    const filter = createFilter();
    expect(filter.processLine("")).toBeNull();
  });

  it("drops continuation lines after a dropped entry", () => {
    const filter = createFilter();
    const dropped =
      "2026-04-16 12:53:59.832625 Laboratory{CoreFoundation}[1] <DEBUG>: first [com.apple.CFBundle][resources]";
    expect(filter.processLine(dropped)).toBeNull();
    expect(filter.processLine("    Localizations : [en]")).toBeNull();
    expect(filter.processLine("    Dev language  : en")).toBeNull();
  });

  it("keeps continuation lines after a kept entry", () => {
    const filter = createFilter();
    const kept = "2026-04-16 12:53:59.832625 Laboratory{Laboratory.debug.dylib}[1] <NOTICE>: multi-line [sub][cat]";
    expect(filter.processLine(kept)).toBe(kept);
    expect(filter.processLine("    second line")).toBe("    second line");
    expect(filter.processLine("    third line")).toBe("    third line");
  });

  it("passes through unparseable lines before the first parsed entry (tunnel notices)", () => {
    const filter = createFilter();
    expect(filter.processLine("Connected to tunnel")).toBe("Connected to tunnel");
    expect(filter.processLine("Enabling developer disk image")).toBe("Enabling developer disk image");
    const dropped = "2026-04-16 12:53:59.832625 Laboratory{CoreFoundation}[1] <DEBUG>: noise [com.apple.X][Y]";
    expect(filter.processLine(dropped)).toBeNull();
    expect(filter.processLine("    continuation of noise")).toBeNull();
  });
});

describe("Pymobiledevice3JsonLogFilter", () => {
  function jsonLine(overrides: Record<string, unknown> = {}): string {
    return JSON.stringify({
      pid: 67135,
      timestamp: "2026-04-16T12:53:59.832625",
      level: "NOTICE",
      image_name: "/usr/lib/Laboratory.debug.dylib",
      image_offset: 0,
      filename: "/private/var/containers/Bundle/Application/ABC/Laboratory.app/Laboratory",
      message: "ContentView initialized",
      label: { subsystem: "Laboratory Test", category: "App" },
      ...overrides,
    });
  }

  function createFilter(
    options: {
      executableName: string;
      debugDylibOnly?: boolean;
      subsystemDenyList?: string[];
      subsystemAllowList?: string[];
      minLevel?: string;
    } = { executableName: "Laboratory" },
  ) {
    return new Pymobiledevice3JsonLogFilter(options);
  }

  it("keeps app logs and formats them for display", () => {
    const filter = createFilter();
    expect(filter.processLine(jsonLine())).toBe(
      "2026-04-16T12:53:59.832625 Laboratory{Laboratory.debug.dylib}[67135] <NOTICE>: ContentView initialized [Laboratory Test][App]",
    );
  });

  it("drops framework noise by image_name basename", () => {
    const filter = createFilter();
    expect(
      filter.processLine(
        jsonLine({ image_name: "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation" }),
      ),
    ).toBeNull();
  });

  it("passes through non-JSON lines (tunnel notices)", () => {
    const filter = createFilter();
    expect(filter.processLine("Connected to remote tunnel")).toBe("Connected to remote tunnel");
  });

  it("formats entries without a label", () => {
    const filter = createFilter();
    expect(filter.processLine(jsonLine({ label: null }))).toBe(
      "2026-04-16T12:53:59.832625 Laboratory{Laboratory.debug.dylib}[67135] <NOTICE>: ContentView initialized",
    );
  });

  it("subsystem deny-list drops matching subsystems", () => {
    const filter = createFilter({
      executableName: "Laboratory",
      subsystemDenyList: ["com.apple.*"],
    });
    expect(
      filter.processLine(jsonLine({ label: { subsystem: "com.apple.CFBundle", category: "resources" } })),
    ).toBeNull();
    expect(filter.processLine(jsonLine())).toContain("ContentView initialized");
  });

  it("subsystem allow-list keeps only matching subsystems", () => {
    const filter = createFilter({
      executableName: "Laboratory",
      subsystemAllowList: ["Laboratory Test"],
    });
    expect(filter.processLine(jsonLine({ label: { subsystem: "Other", category: "Cat" } }))).toBeNull();
    expect(filter.processLine(jsonLine())).toContain("ContentView initialized");
  });

  it("honors minimum level", () => {
    const filter = createFilter({ executableName: "Laboratory", minLevel: "ERROR" });
    expect(filter.processLine(jsonLine({ level: "INFO" }))).toBeNull();
    expect(filter.processLine(jsonLine({ level: "ERROR" }))).toContain("<ERROR>");
  });
});
