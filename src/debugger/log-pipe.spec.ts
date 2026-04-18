import {
  PassthroughLogPipe,
  Pymobiledevice3JsonLogPipe,
  Pymobiledevice3LogPipe,
  buildAppImageFilter,
  parseSyslogLine,
} from "./log-pipe";

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

describe("buildAppImageFilter", () => {
  const make = (imageName: string) => ({
    timestamp: "t",
    processName: "Laboratory",
    imageName,
    pid: 1,
    level: "NOTICE",
    message: "m",
  });

  it("keeps both main image and debug dylib by default", () => {
    const keep = buildAppImageFilter("Laboratory");
    expect(keep(make("Laboratory"))).toBe(true);
    expect(keep(make("Laboratory.debug.dylib"))).toBe(true);
  });

  it("debugDylibOnly=true keeps only the debug dylib", () => {
    const keep = buildAppImageFilter("Laboratory", true);
    expect(keep(make("Laboratory.debug.dylib"))).toBe(true);
    expect(keep(make("Laboratory"))).toBe(false);
  });

  it("drops framework-emitted noise (CoreFoundation, Foundation, etc.)", () => {
    const keep = buildAppImageFilter("Laboratory");
    expect(keep(make("CoreFoundation"))).toBe(false);
    expect(keep(make("Foundation"))).toBe(false);
    expect(keep(make("libdispatch.dylib"))).toBe(false);
  });

  it("does not match by prefix — 'LaboratoryExt' is a different image", () => {
    const keep = buildAppImageFilter("Laboratory");
    expect(keep(make("LaboratoryExt"))).toBe(false);
  });
});

describe("Pymobiledevice3LogPipe", () => {
  function createPipe(
    options: {
      executableName: string;
      debugDylibOnly?: boolean;
      subsystemDenyList?: string[];
      subsystemAllowList?: string[];
      minLevel?: string;
    } = { executableName: "Laboratory" },
  ) {
    const lines: string[] = [];
    const pipe = new Pymobiledevice3LogPipe((line) => lines.push(line), options);
    return { pipe, lines };
  }

  it("drops framework noise", () => {
    const { pipe, lines } = createPipe();
    const noise =
      "2026-04-16 12:52:32.707333 Laboratory{CoreFoundation}[67116] <NOTICE>: noise [com.apple.defaults][User Defaults]";
    pipe.push(`${noise}\n`);
    expect(lines).toEqual([]);
  });

  it("keeps app logs from debug dylib", () => {
    const { pipe, lines } = createPipe();
    const appLog =
      "2026-04-16 12:53:59.832625 Laboratory{Laboratory.debug.dylib}[67135] <NOTICE>: hello [Lab Test][App]";
    pipe.push(`${appLog}\n`);
    expect(lines).toEqual([appLog]);
  });

  it("keeps the main image by default (debugDylibOnly=false)", () => {
    const { pipe, lines } = createPipe();
    const line = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: hi [sub][cat]";
    pipe.push(`${line}\n`);
    expect(lines).toEqual([line]);
  });

  it("debugDylibOnly=true drops the main image", () => {
    const { pipe, lines } = createPipe({ executableName: "Laboratory", debugDylibOnly: true });
    const mainImage = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <DEBUG>: noise [com.apple.Previews][X]";
    const debugDylib = "2026-04-16 12:53:59.832625 Laboratory{Laboratory.debug.dylib}[1] <NOTICE>: app log [sub][cat]";
    pipe.push(`${mainImage}\n`);
    pipe.push(`${debugDylib}\n`);
    expect(lines).toEqual([debugDylib]);
  });

  it("subsystem deny-list drops matching subsystems", () => {
    const { pipe, lines } = createPipe({
      executableName: "Laboratory",
      subsystemDenyList: ["com.apple.*"],
    });
    const apple = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <DEBUG>: noise [com.apple.CFBundle][resources]";
    const app = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: hi [com.example.app][ui]";
    pipe.push(`${apple}\n`);
    pipe.push(`${app}\n`);
    expect(lines).toEqual([app]);
  });

  it("subsystem deny-list does not affect entries without a label", () => {
    const { pipe, lines } = createPipe({
      executableName: "Laboratory",
      subsystemDenyList: ["com.apple.*"],
    });
    const noLabel = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: no label here";
    pipe.push(`${noLabel}\n`);
    expect(lines).toEqual([noLabel]);
  });

  it("subsystem allow-list keeps only matching subsystems", () => {
    const { pipe, lines } = createPipe({
      executableName: "Laboratory",
      subsystemAllowList: ["com.example.*"],
    });
    const match = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: a [com.example.app][ui]";
    const other = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: b [com.apple.CFBundle][resources]";
    pipe.push(`${match}\n`);
    pipe.push(`${other}\n`);
    expect(lines).toEqual([match]);
  });

  it("subsystem allow-list drops entries without a label", () => {
    const { pipe, lines } = createPipe({
      executableName: "Laboratory",
      subsystemAllowList: ["com.example.*"],
    });
    const noLabel = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: no label";
    pipe.push(`${noLabel}\n`);
    expect(lines).toEqual([]);
  });

  it("deny-list and allow-list work together", () => {
    const { pipe, lines } = createPipe({
      executableName: "Laboratory",
      subsystemDenyList: ["com.apple.*"],
      subsystemAllowList: ["com.example.myapp"],
    });
    const apple = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: a [com.apple.X][Y]";
    const myApp = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: b [com.example.myapp][ui]";
    const otherApp = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: c [com.example.other][ui]";
    pipe.push(`${apple}\n`);
    pipe.push(`${myApp}\n`);
    pipe.push(`${otherApp}\n`);
    expect(lines).toEqual([myApp]);
  });

  it("honors an optional minimum level", () => {
    const { pipe, lines } = createPipe({ executableName: "Laboratory", minLevel: "ERROR" });
    const info = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <INFO>: quiet [s][c]";
    const error = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <ERROR>: loud [s][c]";
    pipe.push(`${info}\n`);
    pipe.push(`${error}\n`);
    expect(lines).toEqual([error]);
  });

  it("passes through tunnel notices before the first parsed entry", () => {
    const { pipe, lines } = createPipe();
    pipe.push("Connected to tunnel\n");
    expect(lines).toEqual(["Connected to tunnel"]);
  });

  it("drops empty lines", () => {
    const { pipe, lines } = createPipe();
    pipe.push("\n");
    expect(lines).toEqual([]);
  });

  it("drops continuation lines after a dropped entry", () => {
    const { pipe, lines } = createPipe();
    const dropped =
      "2026-04-16 12:53:59.832625 Laboratory{CoreFoundation}[1] <DEBUG>: first [com.apple.CFBundle][resources]";
    pipe.push(`${dropped}\n`);
    pipe.push("    Localizations : [en]\n");
    pipe.push("    Dev language  : en\n");
    expect(lines).toEqual([]);
  });

  it("keeps continuation lines after a kept entry", () => {
    const { pipe, lines } = createPipe();
    const kept = "2026-04-16 12:53:59.832625 Laboratory{Laboratory.debug.dylib}[1] <NOTICE>: multi-line [sub][cat]";
    pipe.push(`${kept}\n`);
    pipe.push("    second line\n");
    pipe.push("    third line\n");
    expect(lines).toEqual([kept, "    second line", "    third line"]);
  });

  it("passes through unparseable lines before the first parsed entry (tunnel notices)", () => {
    const { pipe, lines } = createPipe();
    pipe.push("Connected to tunnel\n");
    pipe.push("Enabling developer disk image\n");
    const dropped = "2026-04-16 12:53:59.832625 Laboratory{CoreFoundation}[1] <DEBUG>: noise [com.apple.X][Y]";
    pipe.push(`${dropped}\n`);
    pipe.push("    continuation of noise\n");
    expect(lines).toEqual(["Connected to tunnel", "Enabling developer disk image"]);
  });

  it("buffers partial fragments across chunks", () => {
    const { pipe, lines } = createPipe();
    const line = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: hi [s][c]";
    pipe.push(line.slice(0, 10));
    expect(lines).toEqual([]);
    pipe.push(`${line.slice(10)}\n`);
    expect(lines).toEqual([line]);
  });

  it("flush() emits a trailing fragment without newline", () => {
    const { pipe, lines } = createPipe();
    const line = "2026-04-16 12:53:59.832625 Laboratory{Laboratory}[1] <NOTICE>: hi [s][c]";
    pipe.push(line);
    expect(lines).toEqual([]);
    pipe.flush();
    expect(lines).toEqual([line]);
  });
});

describe("PassthroughLogPipe", () => {
  it("forwards chunks as-is", () => {
    const lines: string[] = [];
    const processor = new PassthroughLogPipe((text) => lines.push(text));
    processor.push("hello");
    processor.push("world\nfoo");
    expect(lines).toEqual(["hello", "world\nfoo"]);
  });

  it("flush() does nothing", () => {
    const lines: string[] = [];
    const processor = new PassthroughLogPipe((text) => lines.push(text));
    processor.flush();
    expect(lines).toEqual([]);
  });
});

describe("Pymobiledevice3JsonLogPipe", () => {
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

  function createPipe(
    options: {
      executableName: string;
      debugDylibOnly?: boolean;
      subsystemDenyList?: string[];
      subsystemAllowList?: string[];
      minLevel?: string;
    } = { executableName: "Laboratory" },
  ) {
    const lines: string[] = [];
    const pipe = new Pymobiledevice3JsonLogPipe((line) => lines.push(line), options);
    return { pipe, lines };
  }

  it("keeps app logs and formats them for display", () => {
    const { pipe, lines } = createPipe();
    pipe.push(`${jsonLine()}\n`);
    expect(lines).toEqual([
      "2026-04-16T12:53:59.832625 Laboratory{Laboratory.debug.dylib}[67135] <NOTICE>: ContentView initialized [Laboratory Test][App]",
    ]);
  });

  it("drops framework noise by image_name basename", () => {
    const { pipe, lines } = createPipe();
    pipe.push(`${jsonLine({ image_name: "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation" })}\n`);
    expect(lines).toEqual([]);
  });

  it("passes through non-JSON lines (tunnel notices)", () => {
    const { pipe, lines } = createPipe();
    pipe.push("Connected to remote tunnel\n");
    expect(lines).toEqual(["Connected to remote tunnel"]);
  });

  it("formats entries without a label", () => {
    const { pipe, lines } = createPipe();
    pipe.push(`${jsonLine({ label: null })}\n`);
    expect(lines).toEqual([
      "2026-04-16T12:53:59.832625 Laboratory{Laboratory.debug.dylib}[67135] <NOTICE>: ContentView initialized",
    ]);
  });

  it("subsystem deny-list drops matching subsystems", () => {
    const { pipe, lines } = createPipe({
      executableName: "Laboratory",
      subsystemDenyList: ["com.apple.*"],
    });
    pipe.push(`${jsonLine({ label: { subsystem: "com.apple.CFBundle", category: "resources" } })}\n`);
    pipe.push(`${jsonLine()}\n`);
    expect(lines.length).toBe(1);
    expect(lines[0]).toContain("ContentView initialized");
  });

  it("subsystem allow-list keeps only matching subsystems", () => {
    const { pipe, lines } = createPipe({
      executableName: "Laboratory",
      subsystemAllowList: ["Laboratory Test"],
    });
    pipe.push(`${jsonLine({ label: { subsystem: "Other", category: "Cat" } })}\n`);
    pipe.push(`${jsonLine()}\n`);
    expect(lines.length).toBe(1);
    expect(lines[0]).toContain("ContentView initialized");
  });

  it("honors minimum level", () => {
    const { pipe, lines } = createPipe({ executableName: "Laboratory", minLevel: "ERROR" });
    pipe.push(`${jsonLine({ level: "INFO" })}\n`);
    pipe.push(`${jsonLine({ level: "ERROR" })}\n`);
    expect(lines.length).toBe(1);
    expect(lines[0]).toContain("<ERROR>");
  });

  it("buffers partial chunks", () => {
    const { pipe, lines } = createPipe();
    const full = `${jsonLine()}\n`;
    pipe.push(full.slice(0, 20));
    expect(lines).toEqual([]);
    pipe.push(full.slice(20));
    expect(lines.length).toBe(1);
  });

  it("flush() emits trailing fragment", () => {
    const { pipe, lines } = createPipe();
    pipe.push(jsonLine());
    expect(lines).toEqual([]);
    pipe.flush();
    expect(lines.length).toBe(1);
  });
});
