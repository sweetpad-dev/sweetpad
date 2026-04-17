import {
  buildAppImageFilter,
  createLineBuffer,
  createSyslogLineProcessor,
  parseSyslogLine,
} from "./syslog-parser";

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
    const line =
      "2026-04-16 12:52:32.707333 Laboratory{Laboratory+0x1a2b3c}[67135] <NOTICE>: tick [sub][cat]";
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
    const line =
      "\x1b[32m2026-04-16 12:52:32.707333\x1b[0m Laboratory{Laboratory}[1] <\x1b[33mNOTICE\x1b[0m>: colored";
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

describe("buildAppImageFilter", () => {
  const keep = buildAppImageFilter("Laboratory");
  const make = (imageName: string) => ({
    timestamp: "t",
    processName: "Laboratory",
    imageName,
    pid: 1,
    level: "NOTICE",
    message: "m",
  });

  it("keeps the app's main image", () => {
    expect(keep(make("Laboratory"))).toBe(true);
  });

  it("keeps the ENABLE_DEBUG_DYLIB variant", () => {
    expect(keep(make("Laboratory.debug.dylib"))).toBe(true);
  });

  it("drops framework-emitted noise (CoreFoundation, Foundation, etc.)", () => {
    expect(keep(make("CoreFoundation"))).toBe(false);
    expect(keep(make("Foundation"))).toBe(false);
    expect(keep(make("libdispatch.dylib"))).toBe(false);
  });

  it("does not match by prefix — 'LaboratoryExt' is a different image", () => {
    expect(keep(make("LaboratoryExt"))).toBe(false);
  });
});

describe("createSyslogLineProcessor", () => {
  it("solves PR #231 case 1: drops framework noise with matching process name", () => {
    const process = createSyslogLineProcessor({ executableName: "Laboratory" });
    const noise =
      "2026-04-16 12:52:32.707333 Laboratory{CoreFoundation}[67116] <NOTICE>: looked up value for key X [com.apple.defaults][User Defaults]";
    expect(process(noise)).toBeNull();
  });

  it("solves PR #231 case 2: keeps app logs with a custom Logger subsystem", () => {
    const process = createSyslogLineProcessor({ executableName: "Laboratory" });
    const appLog =
      "2026-04-16 12:53:59.832625 Laboratory{Laboratory.debug.dylib}[67135] <NOTICE>: ContentView initialized [Laboratory Test][App]";
    expect(process(appLog)).toBe(appLog);
  });

  it("keeps the app's main image when not using ENABLE_DEBUG_DYLIB", () => {
    const process = createSyslogLineProcessor({ executableName: "Laboratory" });
    const line = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[67135] <NOTICE>: hi [sub][cat]";
    expect(process(line)).toBe(line);
  });

  it("passes through lines that don't match the format (tunnel notices, banners)", () => {
    const process = createSyslogLineProcessor({ executableName: "Laboratory" });
    expect(process("Connected to tunnel")).toBe("Connected to tunnel");
  });

  it("drops empty lines", () => {
    const process = createSyslogLineProcessor({ executableName: "Laboratory" });
    expect(process("")).toBeNull();
  });

  it("honors an optional subsystem allow-list", () => {
    const process = createSyslogLineProcessor({
      executableName: "Laboratory",
      subsystems: ["Laboratory Test"],
    });
    const match =
      "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: a [Laboratory Test][App]";
    const other =
      "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: b [Other Sub][App]";
    expect(process(match)).toBe(match);
    expect(process(other)).toBeNull();
  });

  it("honors an optional minimum level", () => {
    const process = createSyslogLineProcessor({
      executableName: "Laboratory",
      minLevel: "ERROR",
    });
    const info = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <INFO>: quiet [s][c]";
    const error = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <ERROR>: loud [s][c]";
    expect(process(info)).toBeNull();
    expect(process(error)).toBe(error);
  });
});

describe("createLineBuffer", () => {
  it("emits one event per newline-terminated line", () => {
    const lines: string[] = [];
    const buf = createLineBuffer((l) => lines.push(l));
    buf.push("one\ntwo\n");
    expect(lines).toEqual(["one", "two"]);
  });

  it("buffers partial fragments across chunks", () => {
    const lines: string[] = [];
    const buf = createLineBuffer((l) => lines.push(l));
    buf.push("hel");
    buf.push("lo\nworl");
    buf.push("d\n");
    expect(lines).toEqual(["hello", "world"]);
  });

  it("flush() emits a trailing fragment without newline", () => {
    const lines: string[] = [];
    const buf = createLineBuffer((l) => lines.push(l));
    buf.push("no-newline");
    expect(lines).toEqual([]);
    buf.flush();
    expect(lines).toEqual(["no-newline"]);
  });
});
